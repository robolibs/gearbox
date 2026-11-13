#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/stroke-projection"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin=${GEARBOX_CAPTURE_BIN:-"$root/target/release/gearbox"}
gallery="$root/target/release/examples/draw_gallery"
instance="draw-vfx-strokes-$$"
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
        echo 'Renderer error; stroke capture is invalid' >&2
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
"${gb[@]}" scene pause
for view in near far; do
    distance=10 pitch=20
    if [[ $view == far ]]; then distance=24 pitch=45; fi
    "${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 180 --pitch "$pitch" --distance "$distance" --above 1
    sleep 3
    for reverse in false true; do
        for segments in 1 256; do
            "$gallery" "$instance" stroke-probe "$segments" "$reverse"
            sleep 3
            renderer_ok
            "${gb[@]}" instance screenshot "$out/$view-$reverse-$segments.png" --viewport --timeout 30
            renderer_ok
        done
    done
done
python3 "$root/tests/draw_stroke_images.py" "$out"
