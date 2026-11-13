#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/cloud-legacy/native"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
if [[ -f "$out/launch.json" ]]; then
    archive="$out/previous-$(date +%s)"
    mkdir -p "$archive"
    find "$out" -mindepth 1 -maxdepth 1 ! -name 'previous-*' -exec mv -t "$archive" -- {} +
fi
bin="$root/target/release/gearbox"
gallery="$root/target/release/examples/draw_gallery"
instance="draw-vfx-legacy-points-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log"
gb=("$bin" --instance "$instance" --timeout 30)
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader|InvalidViewQuery|compact legacy .*fallback:|compact legacy point admission:' "$out/viewer.log"; then return 1; fi
}
cleanup() {
    result=$?
    if ! renderer_ok; then result=1; fi
    "${gb[@]}" instance stop "$instance" || true
    if (( result )); then printf 'Not accepted: inspect native log and captures.\n' > "$out/NOT_ACCEPTED.txt"; fi
    exit "$result"
}
"$bin" run --name "$instance" --headless 1200x800 --detach > "$out/launch.json"
trap cleanup EXIT
"${gb[@]}" instance wait "$instance" --timeout 90
"${gb[@]}" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r .pid "$out/instance.json")
sha256sum "$bin" "$gallery" "/proc/$pid/exe" > "$out/binary.sha256"
[[ $(sha256sum "$bin" | cut -d' ' -f1) == $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]
"${gb[@]}" scene pause
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 180 --pitch 55 --distance 24 --above 2
sleep 3
for case in $(seq 0 20); do
    for path in compact generic; do
        generic=false
        [[ $path != generic ]] || generic=true
        "$gallery" "$instance" legacy-point-probe "$generic" "$case"
        sleep 1
        renderer_ok
        printf -v filename 'case-%02d-%s.png' "$case" "$path"
        "${gb[@]}" instance screenshot "$out/$filename" --viewport
        renderer_ok
    done
done
"$gallery" "$instance" legacy-point-probe false
sleep 2
"${gb[@]}" instance screenshot "$out/all-compact.png" --viewport
"${gb[@]}" draw clear vfx-gallery/legacy-point-probe
sleep 2
"${gb[@]}" instance screenshot "$out/clear.png" --viewport
renderer_ok
[[ $(grep -c 'compact legacy ready .*points=16 positions=256 attributes=256 uniform=128' "$out/viewer.log") -ge 40 ]]
[[ $(grep -c 'compact legacy ready .*points=2 positions=32 attributes=32 uniform=128' "$out/viewer.log") -ge 2 ]]
if [[ -d "$root/../../.python-packages" ]]; then export PYTHONPATH="$root/../../.python-packages${PYTHONPATH:+:$PYTHONPATH}"; fi
python3 "$root/tests/draw_legacy_point_images.py" "$out"
