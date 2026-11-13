#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
preview="$root/target/draw-vfx/live-preview"
out=${1:-"$root/target/draw-vfx/cloud-runtime/views"}
scene=${2:-cloud-basics}
case "$scene" in cloud-basics|cloud-scale) ;; *) echo 'Expected cloud-basics or cloud-scale' >&2; exit 2 ;; esac
bin="$root/target/release/gearbox"
instance=$(cat "$preview/instance.txt")
export XDG_CONFIG_HOME="$preview/config" XDG_STATE_HOME="$preview/state"
mkdir -p "$out"
"$bin" --instance "$instance" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r .pid "$out/instance.json")
[[ $pid =~ ^[1-9][0-9]*$ && -e /proc/$pid/exe ]]
built=$(sha256sum "$bin" | cut -d' ' -f1)
live=$(sha256sum "/proc/$pid/exe" | cut -d' ' -f1)
if [[ $built != "$live" ]]; then echo 'Preview binary is stale; run make preview-draw-data first' >&2; exit 1; fi
check_renderer() {
    if grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader' "$preview/viewer.log"; then
        echo 'Renderer failure invalidates these captures' >&2
        return 1
    fi
    while IFS= read -r expected; do
        grep -Fq "typed cloud activated $expected" "$preview/viewer.log" || return 1
    done < <(jq -r '.datasets[] | "\(.key.namespace)/\(.key.id) incarnation=\(.incarnation) generation=\(.stored_generation)"' "$preview/$scene.typed.json")
}
for view in near grazing overview; do
    check_renderer
    bash "$root/scripts/focus-draw.sh" "$instance" "$preview/$scene.manifest.json" "$view"
    sleep 3
    "$bin" --instance "$instance" instance screenshot "$out/$view.window.png" --timeout 30
    check_renderer
done
cp "$preview/viewer.log" "$out/viewer.log"
cp "$preview/$scene.typed.json" "$out/typed.json"
sha256sum "$bin" > "$out/binary.sha256"
printf 'Typed near/grazing/overview captures: %s; preview left open: %s\n' "$out" "$instance"
