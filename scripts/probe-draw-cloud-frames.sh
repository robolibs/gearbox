#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/cloud-frames/native"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
if [[ -f "$out/launch.json" ]]; then
    archive="$out/previous-$(date +%s)"
    mkdir -p "$archive"
    find "$out" -mindepth 1 -maxdepth 1 ! -name 'previous-*' -exec mv -t "$archive" -- {} +
fi
bin="$root/target/release/gearbox"
probe="$root/target/release/examples/draw_cloud_frames"
instance="draw-vfx-cloud-frames-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log" GEARBOX_DRAW_RESOURCE_PROBE="$out"
gb=("$bin" --instance "$instance" --timeout 30)
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader|InvalidViewQuery' "$out/viewer.log"; then
        echo 'Renderer error invalidates frame verification' >&2
        return 1
    fi
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
sha256sum "$bin" "$probe" "/proc/$pid/exe" > "$out/binary.sha256"
[[ $(sha256sum "$bin" | cut -d' ' -f1) == $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]
"${gb[@]}" spawn machine "$root/assets/machines/usd/clearpath_husky.usdz" --machine frame_robot --at 0 0 0 --wait-timeout 90
sleep 3
"${gb[@]}" scene pause
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 45 --distance 20 --above 2
sleep 2
renderer_ok
"$probe" "$instance" frame_robot "$out" "$bin" | tee "$out/result.jsonl"
renderer_ok
if [[ -d "$root/../../.python-packages" ]]; then export PYTHONPATH="$root/../../.python-packages${PYTHONPATH:+:$PYTHONPATH}"; fi
python3 "$root/tests/draw_cloud_frame_images.py" "$out"
printf 'Native typed frame transitions and point-record reuse passed: %s\n' "$out"
