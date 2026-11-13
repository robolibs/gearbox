#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/mesh-normals"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin=${GEARBOX_CAPTURE_BIN:-"$root/target/release/gearbox"}
gallery="$root/target/release/examples/draw_gallery"
instance="draw-vfx-normals-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log"
gb=("$bin" --instance "$instance" --timeout 30)
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader|InvalidViewQuery' "$out/viewer.log"; then
        echo 'Renderer error; normal capture is invalid' >&2
        return 1
    fi
}
started=0
cleanup() {
    result=$?
    if ! renderer_ok; then result=1; fi
    if (( started )); then "${gb[@]}" instance stop "$instance" || true; fi
    if (( result )); then printf 'Not accepted: mesh-normal probe failed; inspect logs and image-metrics.json.\n' > "$out/NOT_ACCEPTED.txt"; fi
    exit "$result"
}
trap cleanup EXIT
"$bin" run --name "$instance" --headless 1200x800 --detach > "$out/launch.json"
started=1
"${gb[@]}" instance wait "$instance" --timeout 90
"${gb[@]}" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r '.pid' "$out/instance.json")
sha256sum "$bin" "$gallery" "/proc/$pid/exe" > "$out/binary.sha256"
[[ $(sha256sum "$bin" | cut -d' ' -f1) == $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]
"${gb[@]}" scene pause
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 140 --pitch 28 --distance 14 --above 2.5
sleep 3
for mode in reference flat authored smooth concave-reference concave concave-authored; do
    "$gallery" "$instance" normals-probe "$mode"
    sleep 3
    renderer_ok
    "${gb[@]}" instance screenshot "$out/$mode.png" --viewport --timeout 30
    renderer_ok
done
sha256sum --check "$out/binary.sha256"
python3 "$root/tests/draw_normals_images.py" "$out"
