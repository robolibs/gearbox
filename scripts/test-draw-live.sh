#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
instance=${1:?provide new or a running preview instance name}
bin="$root/target/release/gearbox"
probe_args=()
if [[ $instance == new ]]; then
    probe_args+=(--reset-owned-scene)
    out=${GEARBOX_DRAW_LIVE_OUT:-"$root/target/draw-vfx/live-contract"}
    mkdir -p "$out"
    out=$(cd -- "$out" && pwd)
    instance="draw-vfx-live-$$"
    export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log"
    "$bin" run --name "$instance" --headless 800x600 --detach > "$out/launch.json"
    cleanup() {
        result=$?
        if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader|InvalidViewQuery' "$out/viewer.log"; then result=1; fi
        "$bin" --instance "$instance" instance stop "$instance" || true
        if (( result )); then printf 'Not accepted: native API probe failed; inspect logs.\n' > "$out/NOT_ACCEPTED.txt"; fi
        exit "$result"
    }
    trap cleanup EXIT
    "$bin" --instance "$instance" instance wait "$instance" --timeout 90
fi
if [[ -d "$root/../../.python-packages" ]]; then export PYTHONPATH="$root/../../.python-packages${PYTHONPATH:+:$PYTHONPATH}"; fi
pid=$("$bin" --instance "$instance" --json instance info "$instance" | jq -r '.pid')
if [[ ! $pid =~ ^[1-9][0-9]*$ || ! -e /proc/$pid/exe ]]; then
    echo 'Native verification requires a live viewer PID' >&2
    exit 1
fi
if [[ $(sha256sum "$bin" | cut -d' ' -f1) != $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]; then
    echo 'Reopen preview-draw: the running image differs from the built viewer' >&2
    exit 1
fi
sha256sum "$bin" "$root/target/release/examples/draw_gallery"
if [[ -n ${out:-} ]]; then sha256sum "$bin" "$root/target/release/examples/draw_gallery" "/proc/$pid/exe" > "$out/binary.sha256"; fi
"$root/target/release/examples/draw_data_probe" "$instance" "${probe_args[@]}"
if [[ -n ${out:-} ]]; then
    "$root/target/release/examples/draw_data_capacity" "$instance" 100000 3 > "$out/capacity.json"
    jq -e '.points == 100000 and .rendered == false and (.results | length == 4) and all(.results[]; .pipeline.outstanding == 0 and .pipeline.queued == 0 and .pipeline.running == 0)' "$out/capacity.json" >/dev/null
    "$root/target/release/examples/draw_data_capacity" "$instance" 1000000 1 > "$out/capacity-million.json"
    jq -e '.points == 1000000 and .rendered == false and (.results | length == 4) and all(.results[]; .pipeline.outstanding == 0 and .pipeline.queued == 0 and .pipeline.running == 0)' "$out/capacity-million.json" >/dev/null
    printf 'Native retained-upload capacity measured; no renderer/scale acceptance claim\n'
fi
capabilities=$("$bin" --instance "$instance" --json api call /gearbox/draw/data/capabilities '{}')
printf '%s\n' "$capabilities" | jq -e '.code == 0 and .stage == 0 and .host_epoch != 0 and ((.metadata_json | fromjson) | .revision == 1 and .retained == ["cloud", "octree", "surfels", "mesh_blocks", "volume", "elevation"] and .rendered == .retained and .features.atomic_generations and .features.delta_updates and .features.unified_listing)' >/dev/null
printf 'Native raw CLI companion capability negotiation passed\n'
if [[ -n ${out:-} ]]; then printf '%s\n' "$capabilities" > "$out/cli-capabilities.json"; fi
"$root/target/release/examples/draw_gallery" "$instance" verify-lifecycle "$bin"
python3 "$root/tests/draw_live_updates.py" "$instance"
