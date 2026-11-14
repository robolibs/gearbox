#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
instance=${1:?instance required}
manifest=${2:?manifest required}
target=${3:-overview}
size=${GEARBOX_WINDOW:-1600x1000}
width=${size%x*}
height=${size#*x}
geometry="${XDG_CONFIG_HOME:-$HOME/.config}/gearbox/window.txt"
if [[ -f $geometry ]]; then
    read -r width height < <(awk -F= '$1 == "width" {w=$2} $1 == "height" {h=$2} END {print w,h}' "$geometry")
fi
camera=$("${GEARBOX_CAPTURE_GALLERY:-$root/target/release/examples/draw_gallery}" focus-camera "$manifest" "$target" "$width" "$height")
read -r east north above bearing pitch distance < <(jq -r '[.focus[], .bearing, .pitch, .distance] | @tsv' <<< "$camera")
read -r lat lon < <(awk -v east="$east" -v north="$north" 'BEGIN {
    pi = atan2(0, -1); lat = 52.370216 * pi / 180;
    a = 6378137; e2 = 0.00669437999014; d = 1 - e2 * sin(lat)^2;
    m = a * (1 - e2) / d^1.5; n = a / sqrt(d);
    printf "%.10f %.10f\n", 52.370216 + north / m * 180 / pi, 4.895168 + east / (n * cos(lat)) * 180 / pi;
}')
"${GEARBOX_CAPTURE_BIN:-$root/target/release/gearbox}" --instance "$instance" instance camera look \
    --lat "$lat" --lon "$lon" --bearing "$bearing" --pitch "$pitch" --distance "$distance" --above "$above"
