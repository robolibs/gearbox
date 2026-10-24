#!/usr/bin/env bash
# Headless Blender preview of a Mars base module or spot; see preview.py.
#   scripts/mars_base/preview.sh OUT module habitat.hab_tower '{"name": "Hab"}'
#   scripts/mars_base/preview.sh OUT base X Y RADIUS [HEIGHT]
root="$(cd "$(dirname "$0")/../.." && pwd)"
out="$(realpath -m "$1")"
shift
exec flatpak run --filesystem="$root" org.blender.Blender -b --factory-startup \
    --python "$root/scripts/mars_base/preview.py" -- "$out" "$@"
