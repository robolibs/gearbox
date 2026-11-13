#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/cloud-legacy-integration/lifecycle"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
if [[ -f "$out/launch.json" ]]; then
    archive="$out/previous-$(date +%s)"
    mkdir -p "$archive"
    find "$out" -mindepth 1 -maxdepth 1 ! -name 'previous-*' -exec mv -t "$archive" -- {} +
fi
bin="$root/target/release/gearbox"
probe="$root/target/release/examples/draw_legacy_lifecycle"
instance="draw-vfx-legacy-life-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log" GEARBOX_DRAW_RESOURCE_PROBE="$out"
gb=("$bin" --instance "$instance" --timeout 30)
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader|InvalidViewQuery|compact legacy .*fallback:|compact legacy point admission:' "$out/viewer.log"; then return 1; fi
}
cleanup() {
    result=$?
    if ! renderer_ok; then result=1; fi
    "${gb[@]}" instance stop "$instance" || true
    if (( result )); then printf 'Not accepted: inspect lifecycle log, metrics and captures.\n' > "$out/NOT_ACCEPTED.txt"; fi
    exit "$result"
}
"$bin" run --name "$instance" --headless 1200x800 --detach > "$out/launch.json"
trap cleanup EXIT
"${gb[@]}" instance wait "$instance" --timeout 90
"${gb[@]}" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r .pid "$out/instance.json")
sha256sum "$bin" "$probe" "/proc/$pid/exe" > "$out/binary.sha256"
[[ $(sha256sum "$bin" | cut -d' ' -f1) == $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]
"${gb[@]}" scene pause
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 180 --pitch 55 --distance 14 --above 2
sleep 2
"$probe" "$instance" "$out" "$bin" | tee "$out/result.jsonl"
renderer_ok
if [[ -d "$root/../../.python-packages" ]]; then export PYTHONPATH="$root/../../.python-packages${PYTHONPATH:+:$PYTHONPATH}"; fi
python3 "$root/tests/draw_legacy_lifecycle_images.py" "$out"
