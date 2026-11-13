#!/usr/bin/env bash
# The representation overview in the owned live preview: every typed spatial
# representation in its own lab, spaced apart and labelled, then an overview
# camera. Each lab clears and redraws only its own namespace.
#   ROBOT=1     also drive the capture-time LiDAR robot (spawns machines)
#   SECONDS_=N  how long the terrain robot rolls its window (default 20)
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
bin=target/release/gearbox
ex=target/release/examples
GEARBOX_CLOUD_POINTS=5000000 bash scripts/preview-draw.sh cloud-scale keep
instance=$(cat target/draw-vfx/live-preview/instance.txt)
gb=("$bin" --instance "$instance" --timeout 30)
step() { printf '=== %s\n' "$1" >&2; }
cloud() {
    local name=$1 offset=$2; shift 2
    step "typed clouds: $name at $offset"
    env GEARBOX_CLOUD_NS="vfx-gallery/plan/$name" GEARBOX_CLOUD_OFFSET="$offset" "$@" "$ex/draw_cloud_basics" "$instance" > /dev/null
}
cloud basics "-30,0"
cloud palettes "30,0" GEARBOX_CLOUD_PALETTES=1
cloud clip "0,30" GEARBOX_CLOUD_CLIP=1
cloud clocks "0,-30" GEARBOX_CLOUD_CLOCKS=1
cloud precision "-30,-30" GEARBOX_CLOUD_PRECISION=1
step "occupancy octrees"
"$ex/draw_octree_lab" "$instance" > /dev/null
step "surfels and mesh blocks"
"$ex/draw_surface_lab" "$instance" > /dev/null
step "sparse volumes"
"$ex/draw_distance_field_lab" "$instance" > /dev/null
step "layered elevation"
"$ex/draw_terrain_layers" "$instance" "${SECONDS_:-20}" > /dev/null &
terrain=$!
if [[ ${ROBOT:-0} == 1 ]]; then
    step "capture-time LiDAR robot"
    "$ex/draw_robot_scan" "$instance" "$bin" 30 > /dev/null &
fi
"${gb[@]}" scene play > /dev/null
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.8975 --bearing 200 --pitch 48 --distance 150 --above 1 > /dev/null
wait "$terrain"
printf 'overview ready in %s\n' "$instance"
