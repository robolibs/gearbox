#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
instance=${1:?provide new or the dedicated drawing preview instance}
case "$instance" in new|draw-world-preview-*|draw-vfx-*) ;; *) echo 'Use new or a dedicated drawing preview' >&2; exit 2 ;; esac
out=${2:-"$root/target/draw-vfx/frame-contract"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin="$root/target/release/gearbox"
started=0
capture=()
if [[ $instance == new ]]; then
    instance="draw-vfx-frames-$$"
    export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log"
    "$bin" run --name "$instance" --headless 1200x800 --detach > "$out/launch.json"
    started=1
    capture=(--viewport)
    trap '"$bin" --instance "$instance" instance stop "$instance" || true' EXIT
    "$bin" --instance "$instance" instance wait "$instance" --timeout 90
fi
gb=("$bin" --instance "$instance" --timeout 30)
"${gb[@]}" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r '.pid' "$out/instance.json")
if [[ ! $pid =~ ^[1-9][0-9]*$ || ! -e /proc/$pid/exe ]] || \
   [[ $(sha256sum "$bin" | cut -d' ' -f1) != $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]; then
    echo 'Reopen preview-draw with the built image before checking frames' >&2
    exit 1
fi
sha256sum "$bin" "/proc/$pid/exe" > "$out/binary.sha256"
log=$(tr '\0' '\n' < "/proc/$pid/environ" | sed -n 's/^GEARBOX_LOG_FILE=//p')
renderer_ok() {
    if [[ -n $log && -f $log ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader' "$log"; then
        echo 'Renderer error; frame capture is invalid' >&2
        return 1
    fi
}
restore() {
    if (( started )); then
        "${gb[@]}" instance stop "$instance" || true
    elif [[ $(jq -r '.paused' "$out/instance.json") == true ]]; then "${gb[@]}" scene pause; else "${gb[@]}" scene play; fi
    if [[ -n $log && -f $log && $log != "$out/viewer.log" ]]; then cp -- "$log" "$out/viewer.log"; fi
}
trap restore EXIT
shot() {
    sleep 2
    renderer_ok
    "${gb[@]}" instance screenshot "$out/$1.png" "${capture[@]}" --timeout 30
    renderer_ok
    "${gb[@]}" --json scene pose "$machine" > "$out/$1.machine.json"
}
machine="draw_frame_probe_$$"
ns=vfx-gallery/frame-contract
"${gb[@]}" scene pause
"${gb[@]}" draw clear vfx-gallery
"${gb[@]}" spawn machine "$root/assets/machines/usd/clearpath_husky.usdz" --machine "$machine" --at 0 0 0 --wait-timeout 90
sleep 3
"${gb[@]}" scene pause
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 45 --distance 19 --above 1
shape() {
    local id=$1 frame=$2 tint=$3 x=$4 y=$5 z=$6
    shift 6
    "${gb[@]}" draw shapes "$ns" "$id" 0,0,0 --frame "$frame" --pose "$x" "$y" "$z" \
        --shape box --size 0.65,0.65,0.65 --style holo --color "$tint" --beam -1 --glow 0.35 --pulse 0.7 \
        --no-drape --label "$id" --label-category FRAME "$@"
}
shape snapshot "$machine" '#ed82c6' -2 1 2 --snapshot
shape follow "$machine" '#73d78a' 2 1 2
shape world world '#f0c060' -3 -2 1.5
shape odom "$machine/odom" '#78b9e9' 3 -2 1.5
shot 01-initial
"${gb[@]}" scene play
timeout 90 "${gb[@]}" machine move "$machine" --forward 1 --distance 3
"${gb[@]}" scene pause
shot 02-frame-moved
"${gb[@]}" draw update "$ns" snapshot --glow 0.2 --prop label_detail=glow-only-update
shot 03-snapshot-restyled
"${gb[@]}" draw update "$ns" snapshot --pose -1 1 2
shot 04-snapshot-local-pose
shape snapshot "$machine" '#ed82c6' -2 1 2 --snapshot
shot 05-snapshot-replaced
"${gb[@]}" --json draw list "$ns" > "$out/entities.json"
printf 'Captured frame transitions in %s; inspect fixed snapshot, live follow, local pose and replacement.\n' "$out"
if (( started )); then python3 "$root/tests/draw_frame_images.py" "$out"; fi
