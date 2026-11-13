#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
preview="$root/target/draw-vfx/live-preview"
out="$root/target/draw-vfx/cloud-multiview/native"
bin="$root/target/release/gearbox"
mkdir -p "$out"
if [[ -e "$out/control.json" || -e "$out/report.json" ]]; then
    archive="$out/previous-$(date +%s)"
    mkdir -p "$archive"
    find "$out" -maxdepth 1 -type f -exec mv -t "$archive" -- {} +
fi
export GEARBOX_DRAW_MULTIVIEW_PROBE="$out"
export XDG_CONFIG_HOME="$preview/config" XDG_STATE_HOME="$preview/state"
if [[ -f "$preview/instance.txt" ]]; then
    instance=$(cat "$preview/instance.txt")
    case "$instance" in draw-world-preview-*) ;; *) echo 'Refusing to replace an unrelated instance' >&2; exit 2 ;; esac
    if "$bin" --instance "$instance" instance info "$instance" >/dev/null 2>&1; then
        "$bin" --instance "$instance" instance stop "$instance"
    fi
fi
bash "$root/scripts/preview-draw.sh" cloud-basics overview
instance=$(cat "$preview/instance.txt")
"$bin" --instance "$instance" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r .pid "$out/instance.json")
sha256sum "$bin" "/proc/$pid/exe" > "$out/binary.sha256"
[[ $(sha256sum "$bin" | cut -d' ' -f1) == $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]
restore() {
    if [[ -f "$out/control.json" ]]; then
        jq '{sequence: (.sequence + 1), action: "remove", active: true, max_generation: 0}' "$out/control.json" > "$out/control.tmp"
        mv "$out/control.tmp" "$out/control.json"
    fi
    bash "$root/scripts/focus-draw.sh" "$instance" "$preview/cloud-basics.manifest.json" overview || true
    cp "$preview/viewer.log" "$out/viewer.log"
}
trap restore EXIT
"$bin" --instance "$instance" instance camera look --lat 52.3704406 --lon 4.895168 --bearing 180 --pitch 25 --distance 10 --above 2
sleep 2
"$root/target/release/examples/draw_multiview_probe" "$instance" "$out" | tee "$out/probe.jsonl"
python3 "$root/tests/draw_multiview_images.py" "$out"
if grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader' "$preview/viewer.log"; then
    echo 'Native renderer failure invalidates the multiview proof' >&2
    exit 1
fi
kill -0 "$pid"
printf 'Native multiview probe passed; owned preview remains open: %s\n' "$instance"
