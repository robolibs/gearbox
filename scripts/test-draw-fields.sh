#!/usr/bin/env bash
# Native distance-field lab on a flat-terrain headless instance: field probes
# through the host, every dataset visible, a CLI field query and layer switch,
# and a screenshot holding the two-tone unknown checker.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/3d-data/fields-$(date -u +%Y%m%dT%H%M%SZ)-$$"}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin="$root/target/release/gearbox"
lab="$root/target/release/examples/draw_distance_field_lab"
ns=vfx-gallery/plan/fields
instance="draw-vfx-fields-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state" GEARBOX_LOG_FILE="$out/viewer.log" GEARBOX_TERRAIN=flat
gb=("$bin" --instance "$instance" --timeout 30)
trap '"${gb[@]}" instance stop "$instance" > /dev/null 2>&1 || true' EXIT
"$bin" run --name "$instance" --headless 1280x800 --detach > "$out/launch.json"
"${gb[@]}" instance wait "$instance" --timeout 90 > /dev/null
"$lab" "$instance" > "$out/lab.json"
read -r lat lon < <(awk 'BEGIN {
    pi = atan2(0, -1); lat = 52.370216 * pi / 180; a = 6378137; e2 = 0.00669437999014;
    d = 1 - e2 * sin(lat)^2; m = a * (1 - e2) / d^1.5; n = a / sqrt(d);
    printf "%.10f %.10f\n", 52.370216 + (-22.0) / m * 180 / pi, 4.895168 + 142.0 / (n * cos(lat)) * 180 / pi }')
"${gb[@]}" instance camera look --lat "$lat" --lon "$lon" --bearing 0 --pitch 58 --distance 26 --above 1 > /dev/null
sleep 3
printf '{' > "$out/visible.json"
first=1
for id in tsdf-sign tsdf-weight tsdf-mesh esdf-level esdf-oblique; do
    state=unknown
    for _ in $(seq 1 60); do
        state=$("${gb[@]}" --json draw data status "$ns" "$id" | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["visibility"]["state"])')
        [[ $state == visible ]] && break
        sleep 0.5
    done
    [[ $first == 1 ]] || printf ',' >> "$out/visible.json"
    printf '"%s":"%s"' "$id" "$state" >> "$out/visible.json"
    first=0
done
printf '}\n' >> "$out/visible.json"
"${gb[@]}" --json draw data field "$ns" esdf-level --at -0.5 -1.6 0.5 --gradient > "$out/field_cli.json"
sleep 2
"${gb[@]}" instance screenshot "$out/fields.png" --viewport --timeout 30 > /dev/null
terrain_ns=vfx-gallery/plan/terrain
"$root/target/release/examples/draw_terrain_layers" "$instance" 6 > "$out/terrain.json"
visible() {
    local ns=$1 id=$2 state=pending
    for _ in $(seq 1 60); do
        state=$("${gb[@]}" --json draw data status "$ns" "$id" | python3 -I -c 'import json,sys; v=json.load(sys.stdin)["visibility"]; print(v["state"], v.get("source_samples", 0), v.get("target", {}).get("dataset", {}).get("generation", 0))')
        [[ $state == visible* ]] && break
        sleep 0.5
    done
    printf '"%s":"%s"' "$id" "$state"
}
{
    printf '{'
    first=1
    for id in elevation slope traversability uncertainty rolling; do
        [[ $first == 1 ]] || printf ','
        visible "$terrain_ns" "$id"
        first=0
    done
    printf '}\n'
} > "$out/terrain_visible.json"
"${gb[@]}" --json draw data layer "$terrain_ns" elevation slope --colormap viridis --range 0 35 > "$out/layer_reply.json"
printf '{%s}\n' "$(visible "$terrain_ns" elevation)" > "$out/terrain_after.json"
"${gb[@]}" instance stop "$instance" > /dev/null 2>&1 || true
trap - EXIT
grep -E 'Validation Error|panicked at|failed to process shader' "$out/viewer.log" > "$out/renderer_errors.txt" || true
python3 -I "$root/tests/draw_fields_native.py" check "$out"
