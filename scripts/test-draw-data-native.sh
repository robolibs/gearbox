#!/usr/bin/env bash
# Native data picking cases on flat-terrain headless instances at two sizes.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/data-native"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin="$root/target/release/gearbox"
probe="$root/target/release/examples/draw_pick_probe"
status=0
for size in 1200x800 900x700; do
    dir="$out/$size"
    rm -rf "$dir"
    mkdir -p "$dir"
    instance="draw-vfx-pick-$$-$size"
    export XDG_CONFIG_HOME="$dir/config" XDG_STATE_HOME="$dir/state" GEARBOX_LOG_FILE="$dir/viewer.log" GEARBOX_TERRAIN=flat
    gb=("$bin" --instance "$instance" --timeout 30)
    "$bin" run --name "$instance" --headless "$size" --detach > "$dir/launch.json"
    if "${gb[@]}" instance wait "$instance" --timeout 90 \
        && "$probe" "$instance" "$bin" "${size%x*}" "${size#*x}" | tee "$dir/result.jsonl"; then
        :
    else
        status=1
    fi
    if grep -E 'Validation Error|panicked at|failed to process shader' "$dir/viewer.log"; then
        echo "renderer error in $size" >&2
        status=1
    fi
    "${gb[@]}" instance stop "$instance" || true
done
passes=$(cat "$out"/*/result.jsonl | grep -c '"PASS"' || true)
fails=$(cat "$out"/*/result.jsonl | grep -c '"FAIL"' || true)
printf 'data picking: %s PASS, %s FAIL (%s)\n' "$passes" "$fails" "$out"
exit "$status"
