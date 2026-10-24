#!/usr/bin/env bash
# Headless Blender check of where the Mars base's buildings meet; see clash.py.
#   scripts/mars_base/clash.sh OUT.json
root="$(cd "$(dirname "$0")/../.." && pwd)"
out="$(realpath -m "$1")"
mkdir -p "$(dirname "$out")"
exec flatpak run --filesystem="$root" org.blender.Blender -b --factory-startup \
    --python "$root/scripts/mars_base/clash.py" -- "$out"
