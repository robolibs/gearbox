#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
preview="$root/target/draw-vfx/live-preview"
out="$root/target/draw-vfx/cloud-status/probe"
bin="$root/target/release/gearbox"
instance=$(cat "$preview/instance.txt")
export XDG_CONFIG_HOME="$preview/config" XDG_STATE_HOME="$preview/state"
export GEARBOX_PROBE_RENDER_LOG="$preview/viewer.log"
mkdir -p "$out"
"$bin" --instance "$instance" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r .pid "$out/instance.json")
[[ $pid =~ ^[1-9][0-9]*$ && -e /proc/$pid/exe ]]
built=$(sha256sum "$bin" | cut -d' ' -f1)
live=$(sha256sum "/proc/$pid/exe" | cut -d' ' -f1)
if [[ $built != "$live" ]]; then echo 'Preview binary is stale; run make preview-draw-data first' >&2; exit 1; fi
check_renderer() {
    if grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader' "$preview/viewer.log"; then
        echo 'Renderer failure invalidates this probe' >&2
        return 1
    fi
    kill -0 "$pid"
}
check_renderer
"$root/target/release/examples/draw_render_status_probe" "$instance" | tee "$out/status.jsonl"
expected=$(jq -r 'select(.checkpoint == "replacement_resources") | .status | "\(.key.namespace)/\(.key.id) incarnation=\(.retained.incarnation) generation=\(.retained.generation) points=2"' "$out/status.jsonl")
[[ -n $expected ]]
if ! grep -Fq "typed cloud activated $expected" "$preview/viewer.log"; then
    echo "Missing exact native activation: $expected" >&2
    exit 1
fi
check_renderer
cp "$preview/viewer.log" "$out/viewer.log"
sha256sum "$bin" > "$out/binary.sha256"
printf 'Native renderer resource/failure and completed-frame probe passed. Preview left open: %s\n' "$instance"
