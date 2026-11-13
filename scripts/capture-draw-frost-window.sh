#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/frost-window"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin="$root/target/release/gearbox"
instances=$(hyprctl instances -j)
wm=${HYPRLAND_INSTANCE_SIGNATURE:-$(jq -r --arg socket "${WAYLAND_DISPLAY:-}" '
    [.[] | select(.wl_socket == $socket)] as $matches |
    if ($matches | length) == 1 then $matches[0].instance
    elif length == 1 then .[0].instance else empty end' <<< "$instances")}
[[ -n $wm ]] || { echo 'Select a running Hyprland instance' >&2; exit 2; }
hypr=(hyprctl -i "$wm")
lua=$("${hypr[@]}" eval "assert(type(hl.dsp.window.resize) == 'function')" 2>&1 || true)
dispatch() {
    expression=$1; shift
    if [[ $lua == ok ]]; then reply=$("${hypr[@]}" dispatch "$expression")
    else reply=$("${hypr[@]}" dispatch "$@"); fi
    printf '%s\n' "$reply"
    [[ $reply == ok ]]
}
"${hypr[@]}" -j monitors > "$out/monitors.json"
"${hypr[@]}" version > "$out/compositor-version.txt"
instance="draw-vfx-frost-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log"
export GEARBOX_WINDOW=1200x800
gb=("$bin" --instance "$instance" --timeout 30)
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader|InvalidViewQuery' "$out/viewer.log"; then
        echo 'Renderer error; frost window capture is invalid' >&2
        return 1
    fi
}
started=0
cleanup() {
    result=$?
    if ! renderer_ok; then result=1; fi
    if (( started )); then "${gb[@]}" instance stop "$instance" || true; fi
    if (( result )); then printf 'Not accepted: native resize probe failed; inspect logs.\n' > "$out/NOT_ACCEPTED.txt"; fi
    exit "$result"
}
trap cleanup EXIT
"$bin" run --name "$instance" --detach > "$out/launch.json"
started=1
"${gb[@]}" instance wait "$instance" --timeout 90
"${gb[@]}" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r '.pid' "$out/instance.json")
sha256sum "$bin" "/proc/$pid/exe" > "$out/binary.sha256"
[[ $(sha256sum "$bin" | cut -d' ' -f1) == $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]
client() { "${hypr[@]}" -j clients | jq --argjson pid "$pid" --arg name "$instance" '.[] | select(.pid == $pid and (.title | contains($name)))'; }
for ((i=0; i<30; i++)); do
    address=$(client | jq -r '.address')
    [[ $address =~ ^0x[0-9a-f]+$ ]] && break
    sleep 1
done
[[ $address =~ ^0x[0-9a-f]+$ ]]
client > "$out/window-before.json"
dispatch "hl.dsp.window.float({window='address:$address', action='set'})" setfloating "address:$address"
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 180 --pitch 70 --distance 14 --above 1
"${gb[@]}" scene pause
for spec in 'color -3' 'white 0' 'black 3'; do
    read -r finish y <<< "$spec"
    "${gb[@]}" draw panel vfx-gallery/frost-window "$finish" --kind gauge --values 0.65 --min 0 --max 1 \
        --title "${finish^^} + TINT" --unit m --panel-finish "$finish" --color '#cf72a8' --pose 0 "$y" 0.8
done
"${gb[@]}" --json draw list vfx-gallery/frost-window > "$out/draw-before.json"
for size in 1200x800 800x1000 1600x900 1200x800; do
    index=$((${index:-0} + 1))
    width=${size%x*}; height=${size#*x}
    [[ $(client | jq -r '.address') == "$address" ]]
    dispatch "hl.dsp.window.resize({window='address:$address', x=$width, y=$height, relative=false})" \
        resizewindowpixel "exact $width $height,address:$address"
    resized() { jq -e --argjson w "$width" --argjson h "$height" \
        '.size[0] >= ($w-1) and .size[0] <= ($w+1) and .size[1] >= ($h-1) and .size[1] <= ($h+1)' "$out/window-$index.json" >/dev/null; }
    for ((i=0; i<30; i++)); do
        client > "$out/window-$index.json"
        actual=$(jq -r '.size | map(tostring) | join("x")' "$out/window-$index.json")
        resized && break
        sleep 1
    done
    resized || { echo "Requested $size, compositor reported $actual" >&2; exit 1; }
    sleep 3
    client > "$out/window-$index.json"
    resized
    actual=$(jq -r '.size | map(tostring) | join("x")' "$out/window-$index.json")
    renderer_ok
    "${gb[@]}" instance screenshot "$out/$index-$actual.png" --timeout 30
    renderer_ok
    ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of json "$out/$index-$actual.png" > "$out/image-$index.json"
    [[ $(jq -r '.streams[0] | [.width,.height] | map(tostring) | join("x")' "$out/image-$index.json") == "$actual" ]]
done
"${gb[@]}" --json draw list vfx-gallery/frost-window > "$out/draw-after.json"
cmp "$out/draw-before.json" "$out/draw-after.json"
printf 'Native resize captures complete; visual inspection required.\n'
