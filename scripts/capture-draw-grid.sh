#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/grid-transitions"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin=${GEARBOX_CAPTURE_BIN:-"$root/target/release/gearbox"}
gallery="$root/target/release/examples/draw_gallery"
instance="draw-vfx-grid-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log"
gb=("$bin" --instance "$instance" --timeout 30)
"$bin" run --name "$instance" --headless 1200x800 --detach > "$out/launch.json"
trap '"${gb[@]}" instance stop "$instance" || true' EXIT
"${gb[@]}" instance wait "$instance" --timeout 90
"${gb[@]}" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r '.pid' "$out/instance.json")
sha256sum "$bin" "$gallery" "/proc/$pid/exe" > "$out/binary.sha256"
if [[ $(sha256sum "$bin" | cut -d' ' -f1) != $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]; then exit 1; fi
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader|InvalidViewQuery' "$out/viewer.log"; then
        echo 'Renderer error; grid capture is invalid' >&2
        return 1
    fi
}
cleanup() {
    result=$?
    if ! renderer_ok; then result=1; fi
    "${gb[@]}" instance stop "$instance" || true
    if (( result )); then printf 'Not accepted: native probe failed; inspect logs and image-metrics.json.\n' > "$out/NOT_ACCEPTED.txt"; fi
    exit "$result"
}
trap cleanup EXIT
shot() {
    sleep 2
    renderer_ok
    "${gb[@]}" instance screenshot "$out/$1.png" --viewport --timeout 30
    renderer_ok
}
"${gb[@]}" spawn machine "$root/tests/fixtures/draw_vfx/frame_anchor.usda" --machine grid_anchor --at 6 0 8 --wait-timeout 90
sleep 3
"${gb[@]}" scene pause
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 32 --distance 30 --above 2
ns=vfx-gallery/grid-cache
grid() {
    "$gallery" "$instance" grid-probe "$1" "${2:-false}" "${3:-false}"
}
fresh() {
    "${gb[@]}" draw delete "$ns" surface
    sleep 1
    grid grid_anchor/odom "$1" "$2"
}
grid world
shot 01-world
"${gb[@]}" draw update "$ns" surface --prop frame=grid_anchor/odom
shot 02-frame-updated
fresh false false
shot 03-frame-fresh
"${gb[@]}" draw update "$ns" surface --prop drape_anchor=true
shot 04-anchor-updated
fresh true false
shot 05-anchor-fresh
"${gb[@]}" draw update "$ns" surface --prop drape_pose_height=true
shot 06-height-updated
fresh true true
shot 07-height-fresh
"${gb[@]}" --json draw list "$ns" > "$out/entities.json"
python3 "$root/tests/draw_grid_images.py" "$out"
