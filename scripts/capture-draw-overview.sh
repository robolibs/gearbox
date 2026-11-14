#!/usr/bin/env bash
# Full-window and viewport captures of every overview bay in the owned preview, by day
# and by night, with the binary and log checks that make them evidence.
#   capture-draw-overview.sh [OUT]   (after `oslo make preview-draw-overview`)
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
out=${1:-target/draw-vfx/3d-data/overview}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
preview=target/draw-vfx/live-preview
instance=$(cat "$preview/instance.txt")
bin=target/release/gearbox
gb=("$bin" --instance "$instance" --timeout 30)
pid=$("${gb[@]}" --json instance info "$instance" | jq -r '.pid')
if [[ $(sha256sum "$bin" | cut -d' ' -f1) != $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]; then
    echo 'The preview runs another binary; reopen preview-draw-overview' >&2
    exit 1
fi
look() {
    local name=$1 lat lon
    read -r lat lon < <(awk -v east="$2" -v north="$3" 'BEGIN {
        pi = atan2(0, -1); lat = 52.370216 * pi / 180;
        a = 6378137; e2 = 0.00669437999014; d = 1 - e2 * sin(lat)^2;
        m = a * (1 - e2) / d^1.5; n = a / sqrt(d);
        printf "%.10f %.10f\n", 52.370216 + north / m * 180 / pi, 4.895168 + east / (n * cos(lat)) * 180 / pi;
    }')
    "${gb[@]}" instance camera look --lat "$lat" --lon "$lon" --bearing "$6" --pitch "$5" --distance "$4" --above 1 > /dev/null
    sleep 3
    "${gb[@]}" instance screenshot "$out/$name.png" > /dev/null
    "${gb[@]}" instance screenshot --viewport "$out/$name-viewport.png" > /dev/null
    printf '%s\n' "$name"
}
day() {
    look "$1-labs" 150 -12 62 52 200
    look "$1-survey" 0 40 120 45 200
    look "$1-survey-close" 4 -34.2 8 28 150
    look "$1-cloud-basics" -30 0 24 40 165
    look "$1-cloud-palettes" 30 0 24 40 165
    look "$1-cloud-clip" 0 30 24 40 165
    look "$1-octree" 62 -2 26 42 200
    look "$1-octree-close" 56 -8 14 38 200
    look "$1-surfaces" 115 -18 30 40 200
    look "$1-surfaces-grazing" 112 -27 10 12 250
    look "$1-fields" 141 -25 24 38 200
    look "$1-fields-close" 137 -27 12 30 200
    look "$1-terrain" 179 -15 32 42 200
}
day day
"${gb[@]}" scene skip 12h > /dev/null
sleep 2
look night-labs 150 -12 62 52 200
look night-survey 0 40 120 45 200
look night-octree 62 -2 26 42 200
look night-surfaces 115 -18 30 40 200
look night-fields 141 -25 24 38 200
look night-terrain 179 -15 32 42 200
"${gb[@]}" scene skip 12h > /dev/null
errors=$(grep -cE 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|failed to process shader|panicked at|error:.*wgsl|GpuAllocation' "$preview/viewer.log" || true)
jq -n --arg instance "$instance" --arg pid "$pid" --arg binary "$(sha256sum "$bin" | cut -d' ' -f1)" \
    --arg time "$(date -u +%FT%TZ)" --argjson errors "$errors" \
    '{instance: $instance, pid: ($pid | tonumber), binary_sha256: $binary, captured: $time, log_errors: $errors}' > "$out/manifest.json"
if (( errors )); then
    echo "renderer errors in $preview/viewer.log; captures are not evidence" >&2
    exit 1
fi
printf 'overview captured in %s\n' "$out"
