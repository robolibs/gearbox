#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/ghost-lifecycle"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin=${GEARBOX_CAPTURE_BIN:-"$root/target/release/gearbox"}
instance="draw-vfx-ghosts-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log"
gb=("$bin" --instance "$instance" --timeout 30)
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader|InvalidViewQuery' "$out/viewer.log"; then
        echo 'Renderer error; ghost capture is invalid' >&2
        return 1
    fi
}
started=0
cleanup() {
    result=$?
    if ! renderer_ok; then result=1; fi
    if (( started )); then "${gb[@]}" instance stop "$instance" || true; fi
    if (( result )); then printf 'Not accepted: native ghost probe failed; inspect logs and image-metrics.json.\n' > "$out/NOT_ACCEPTED.txt"; fi
    exit "$result"
}
trap cleanup EXIT
"$bin" run --name "$instance" --headless 1200x800 --detach > "$out/launch.json"
started=1
"${gb[@]}" instance wait "$instance" --timeout 90
"${gb[@]}" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r '.pid' "$out/instance.json")
sha256sum "$bin" "/proc/$pid/exe" > "$out/binary.sha256"
if [[ $(sha256sum "$bin" | cut -d' ' -f1) != $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]; then exit 1; fi
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 180 --pitch 35 --distance 16 --above 1
"${gb[@]}" scene pause
"${gb[@]}" draw ghost vfx-gallery/ghost-lifecycle robot --machine ghost_probe --color '#ff00ff' --glow 0.2 --pulse -1 --flow -1 --fade 1 '3,0,0.7' '5,0,0.7'
"${gb[@]}" --json draw list vfx-gallery/ghost-lifecycle > "$out/draw-before.json"
shot() {
    sleep 3
    renderer_ok
    "${gb[@]}" instance screenshot "$out/$1.png" --viewport --timeout 30
    renderer_ok
}
spawn() {
    "${gb[@]}" spawn machine "$root/assets/machines/usd/clearpath_husky.usdz" --machine ghost_probe --at 0 0 0 --wait-timeout 90
    sleep 3
    "${gb[@]}" scene pause
}
shot 01-unloaded
spawn
shot 02-loaded
"${gb[@]}" clear usd ghost_probe
for ((i=0; i<30; i++)); do
    "${gb[@]}" --json instance info "$instance" > "$out/removed-instance.json"
    if [[ $(jq '.machine_count' "$out/removed-instance.json") == 0 ]]; then break; fi
    sleep 1
done
[[ $(jq '.machine_count' "$out/removed-instance.json") == 0 ]]
shot 03-removed
spawn
shot 04-reloaded
"${gb[@]}" --json draw list vfx-gallery/ghost-lifecycle > "$out/draw-after.json"
cmp "$out/draw-before.json" "$out/draw-after.json"
python3 "$root/tests/draw_ghost_images.py" "$out"
