#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/sensor-isolation"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin="$root/target/release/gearbox"
gallery="$root/target/release/examples/draw_gallery"
cloud="$root/target/release/examples/draw_cloud_sensors"
instance="draw-vfx-sensors-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log"
gb=("$bin" --instance "$instance" --timeout 30)
"$bin" run --name "$instance" --headless 1200x800 --detach > "$out/launch.json"
trap '"${gb[@]}" instance stop "$instance" || true' EXIT
"${gb[@]}" instance wait "$instance" --timeout 90
"${gb[@]}" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r '.pid' "$out/instance.json")
sha256sum "$bin" "$gallery" "$cloud" "/proc/$pid/exe" > "$out/binary.sha256"
if [[ $(sha256sum "$bin" | cut -d' ' -f1) != $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]; then exit 1; fi
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader|InvalidViewQuery|compact legacy .*fallback:|compact legacy point admission:' "$out/viewer.log"; then
        echo 'Renderer error; sensor capture is invalid' >&2
        return 1
    fi
}
cleanup() {
    result=$?
    if ! renderer_ok; then result=1; fi
    "${gb[@]}" instance stop "$instance" || true
    if (( result )); then printf 'Not accepted: native probe failed; inspect viewer.log and the command output.\n' > "$out/NOT_ACCEPTED.txt"; fi
    exit "$result"
}
trap cleanup EXIT
"${gb[@]}" spawn machine "$root/tests/fixtures/draw_vfx/sensor_isolation.usda" --machine sensor_probe --at 0 0 0 --wait-timeout 90
sleep 3
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 30 --distance 18 --above 2
sleep 2
renderer_ok
"${gb[@]}" instance screenshot "$out/viewer-before.png" --viewport --timeout 30
for link in camera_link full_camera_link; do
    mode=optimized
    if [[ $link == full_camera_link ]]; then mode=full; fi
    mkdir -p "$out/$mode"
    "$gallery" "$instance" verify-sensor sensor_probe "$out/$mode" "$link" | tee "$out/$mode/sensor-result.jsonl"
    renderer_ok
    "${gb[@]}" instance screenshot "$out/$mode/viewer-with-draws.png" --viewport --timeout 30
    renderer_ok
    for stage in before with-draws; do
        ffmpeg -v error -y -f rawvideo -pixel_format rgba -video_size 160x120 -i "$out/$mode/sensor-$stage.rgba" -frames:v 1 "$out/$mode/sensor-$stage.png"
    done
done
"${gb[@]}" --json draw list vfx-gallery/sensor-isolation > "$out/entities.json"
if [[ -d "$root/../../.python-packages" ]]; then export PYTHONPATH="$root/../../.python-packages${PYTHONPATH:+:$PYTHONPATH}"; fi
for link in camera_link full_camera_link; do
    mode=optimized
    if [[ $link == full_camera_link ]]; then mode=full; fi
    typed="$out/$mode/typed-cloud"
    mkdir -p "$typed"
    "$cloud" "$instance" sensor_probe "$link" "$typed" "$bin" | tee "$typed/sensor-result.jsonl"
    renderer_ok
    for stage in before opaque translucent octree surfels mesh volume elevation clear; do
        ffmpeg -v error -y -f rawvideo -pixel_format rgba -video_size 160x120 -i "$typed/sensor-$stage.rgba" -frames:v 1 "$typed/sensor-$stage.png"
    done
    python3 "$root/tests/draw_cloud_sensor_images.py" "$typed"
    legacy="$out/$mode/legacy-compact"
    mkdir -p "$legacy"
    "$cloud" "$instance" sensor_probe "$link" "$legacy" "$bin" legacy | tee "$legacy/sensor-result.jsonl"
    renderer_ok
    for stage in before opaque translucent clear; do
        ffmpeg -v error -y -f rawvideo -pixel_format rgba -video_size 160x120 -i "$legacy/sensor-$stage.rgba" -frames:v 1 "$legacy/sensor-$stage.png"
    done
    python3 "$root/tests/draw_cloud_sensor_images.py" "$legacy" legacy-compact
done
[[ $(grep -c 'compact legacy ready .*points=1617 positions=25872 attributes=25872 uniform=128' "$out/viewer.log") -ge 4 ]]
printf 'Sensor isolation data and viewer positive control saved in %s\n' "$out"
