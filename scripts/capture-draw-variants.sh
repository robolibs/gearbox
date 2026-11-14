#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/variants"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin=${GEARBOX_CAPTURE_BIN:-"$root/target/release/gearbox"}
gallery="$root/target/release/examples/draw_gallery"
bin=$(readlink -f -- "$bin")
if [[ ! -x $bin || ! -x $gallery ]]; then echo 'Viewer and gallery binaries must be executable' >&2; exit 2; fi
instance="draw-variants-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state"
export GEARBOX_LOG_FILE="$out/viewer.log" GEARBOX_DRAW_OPACITY=1
export GEARBOX_RENDER_DIAGNOSTICS=1 MARA_GPU_TIMESTAMPS=1
gb=("$bin" --instance "$instance" --timeout 30)
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader' "$out/viewer.log"; then
        echo 'Renderer error in viewer.log; capture is invalid' >&2
        return 1
    fi
}
started=0
cleanup() { if (( started )); then "${gb[@]}" instance stop "$instance" || true; fi; }
trap cleanup EXIT
"$bin" run --name "$instance" --detach --headless 1600x1000 > "$out/launch.json"
started=1
"${gb[@]}" instance wait "$instance" --timeout 60
"${gb[@]}" --json instance info "$instance" > "$out/instance.start.json"
pid=$(sed -nE 's/^[[:space:]]*"pid":[[:space:]]*([0-9]+),?$/\1/p' "$out/instance.start.json")
if [[ ! $pid =~ ^[1-9][0-9]*$ ]]; then echo 'Viewer did not report a live PID' >&2; exit 1; fi
readlink -f "/proc/$pid/exe" > "$out/live-executable.txt"
sha256sum "/proc/$pid/exe" > "$out/live-binary.sha256"
sha256sum "$bin" > "$out/binary.sha256"
sha256sum "$gallery" > "$out/gallery.sha256"
if [[ $(cut -d' ' -f1 "$out/binary.sha256") != $(cut -d' ' -f1 "$out/live-binary.sha256") ]]; then
    echo 'Live viewer image differs from the requested capture binary' >&2
    exit 1
fi
git -C "$root" diff --ignore-submodules=all > "$out/source.patch"
look() { "${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing "$1" --pitch "$2" --distance "$3" --above 1; sleep 3; }
shot() {
    renderer_ok
    "${gb[@]}" instance screenshot "$out/$1.png" --viewport --timeout 30
    renderer_ok
}
measure() {
    printf '%s start %s\n' "$1" "$(date -u +%FT%TZ)" >> "$out/measurement-windows.txt"
    sleep 35
    printf '%s end %s\n' "$1" "$(date -u +%FT%TZ)" >> "$out/measurement-windows.txt"
    grep -E 'VmRSS|VmHWM|VmSize' "/proc/$pid/status" > "$out/$1.memory.txt"
    shot "$1"
}
look 160 42 26
measure empty
"$gallery" "$instance" hero
measure hero
look 160 42 18
shot hero-near
look 160 42 60
shot hero-far
look 180 12 22
shot hero-grazing
look 160 42 26
"$gallery" "$instance" controls
sleep 3
shot controls
sleep 2
shot controls-later
"$gallery" "$instance" dense
measure dense
"$gallery" "$instance" fields
measure fields
"$gallery" "$instance" world-dense
measure world-dense
"$gallery" "$instance" hero
"${gb[@]}" scene skip 12h
sleep 4
"${gb[@]}" --json scene status > "$out/night-clock.json"
shot hero-night
"${gb[@]}" --json instance info "$instance" > "$out/instance.json"
grep ' ERROR ' "$out/viewer.log" > "$out/errors.txt" || true
renderer_ok
printf 'Drawing variant captures: %s\n' "$out"
