#!/usr/bin/env bash
set -euo pipefail
out=${1:?provide the catalogue capture directory}
mode=${2:-window}
case "$mode" in window|viewport) ;; *) echo 'mode must be window or viewport' >&2; exit 2 ;; esac
out=$(cd -- "$out" && pwd)
shopt -s nullglob
files=("$out"/exhibition/B*.$mode.png)
expected=$(jq '.slots | length' "$out/exhibition.manifest.json")
if (( ${#files[@]} != expected )); then echo "Expected $expected bays, found ${#files[@]}" >&2; exit 1; fi
mkdir -p "$out/contact-sheets"
ffmpeg -hide_banner -loglevel error -y -pattern_type glob -i "$out/exhibition/B*.$mode.png" \
    -vf 'scale=460:-1,tile=4x4:padding=4:margin=4' -fps_mode vfr "$out/contact-sheets/sheet-%02d.png"
printf 'Contact sheets: %s/contact-sheets\n' "$out"
