#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
preview="$root/target/draw-vfx/live-preview"
out="$root/target/draw-vfx/cloud-scale/native"
bin="$root/target/release/gearbox"
mkdir -p "$out"
if [[ -e "$out/report.json" ]]; then
    archive="$out/previous-$(date +%s)"
    mkdir -p "$archive"
    find "$out" -mindepth 1 -maxdepth 1 ! -name 'previous-*' -exec mv -t "$archive" -- {} +
fi
export XDG_CONFIG_HOME="$preview/config" XDG_STATE_HOME="$preview/state"
export GEARBOX_DRAW_RESOURCE_PROBE="$out" GEARBOX_CLOUD_SCALE_PROBE="$out"
if [[ -f "$preview/instance.txt" ]]; then
    instance=$(cat "$preview/instance.txt")
    case "$instance" in draw-world-preview-*) ;; *) echo 'Refusing to replace an unrelated instance' >&2; exit 2 ;; esac
    if "$bin" --instance "$instance" instance info "$instance" >/dev/null 2>&1; then
        "$bin" --instance "$instance" instance stop "$instance"
    fi
fi
trap 'cp "$preview/viewer.log" "$out/viewer.log" 2>/dev/null || true' EXIT
bash "$root/scripts/preview-draw.sh" cloud-scale overview
instance=$(cat "$preview/instance.txt")
"$bin" --instance "$instance" --json instance info "$instance" > "$out/instance.json"
pid=$(jq -r .pid "$out/instance.json")
sha256sum "$bin" "/proc/$pid/exe" > "$out/binary.sha256"
[[ $(sha256sum "$bin" | cut -d' ' -f1) == $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]
bash "$root/scripts/capture-draw-data-views.sh" "$out/views" cloud-scale
jq -e '.result == "pass" and .allocations.positions == 16000000 and .allocations.attributes == 32000000 and .allocations.points == 1000000' "$out/reload.json" >/dev/null
jq -e '.result == "pass" and (.deltas | length) == 2 and .deltas[0].page.items == 65536 and .deltas[1].page.items == 16960' "$out/page-delta.json" >/dev/null
kill -0 "$pid"
printf 'Integrated million-point gate passed; owned survey preview remains open: %s\n' "$instance"
