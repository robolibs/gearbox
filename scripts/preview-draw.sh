#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out="$root/target/draw-vfx/live-preview"
scene=${1:-hero}
view=${2:-keep}
record_seconds=${3:-0}
if [[ ! $record_seconds =~ ^[0-9]+$ ]] || (( record_seconds > 30 )); then echo 'record seconds must be 0..30' >&2; exit 2; fi
if [[ $scene != exhibition ]]; then
    case "$view" in
        keep|near|far|grazing|overview) ;;
        cloud|octree) [[ $scene == data-lab ]] || { echo 'cloud/octree view needs data-lab' >&2; exit 2; } ;;
        ghost) [[ $scene == energy-review || $scene == energy-colors ]] || { echo 'ghost view needs an energy scene' >&2; exit 2; } ;;
        *) echo 'view must be keep, near, far, grazing, overview or energy-scene ghost' >&2; exit 2 ;;
    esac
fi
mkdir -p "$out"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state"
export GEARBOX_LOG_FILE="$out/viewer.log" GEARBOX_WINDOW=1600x1000
export GEARBOX_CAMERA_DISTANCE=26 GEARBOX_CAMERA_ELEVATION=42 GEARBOX_DRAW_OPACITY=1
bin="$root/target/release/gearbox"
gallery="$root/target/release/examples/draw_gallery"
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader|Use-after-free' "$out/viewer.log"; then
        echo 'Renderer error in viewer.log; preview capture is invalid' >&2
        return 1
    fi
}
instance=""
started_here=0
if [[ -f "$out/instance.txt" ]]; then instance=$(cat "$out/instance.txt"); fi
if [[ -n $instance ]] && "$bin" --instance "$instance" --json instance info "$instance" > "$out/current.json" 2>/dev/null; then
    pid=$(sed -nE 's/^[[:space:]]*"pid":[[:space:]]*([0-9]+),?$/\1/p' "$out/current.json")
    if [[ $pid =~ ^[1-9][0-9]*$ && -e /proc/$pid/exe ]] && \
       [[ $(sha256sum "$bin" | cut -d' ' -f1) != $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]; then
        "$bin" --instance "$instance" instance stop "$instance"
        instance=""
    fi
fi
if [[ -z $instance ]] || ! "$bin" --instance "$instance" instance info "$instance" >/dev/null 2>&1; then
    instance="draw-world-preview-$$"
    "$bin" run --name "$instance" --detach > "$out/launch.json"
    started_here=1
    printf '%s\n' "$instance" > "$out/instance.txt"
    "$bin" --instance "$instance" instance wait "$instance" --timeout 60
    "$bin" --instance "$instance" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 42 --distance 26 --above 1
fi
if [[ $scene == halo || $scene == robot || $scene == styles || $scene == exhibition || $scene == energy-review || $scene == energy-colors || $scene == review-system || $scene == blur-review ]]; then
    if (( started_here )) || ! "$bin" --instance "$instance" --timeout 3 machine info vfx_rover >/dev/null 2>&1; then
        "$bin" --instance "$instance" spawn machine "$root/assets/machines/usd/clearpath_husky.usdz" --machine vfx_rover --at 0 0 0 --wait-timeout 90
        sleep 3
    fi
fi
if [[ $scene == cloud-basics || $scene == cloud-scale ]]; then
    "$root/target/release/examples/draw_${scene//-/_}" "$instance" | tee "$out/$scene.typed.json"
elif [[ $scene == survey-glow ]]; then
    "$gallery" "$instance" survey vfx_rover
    for item in 'sample-a marks' 'sample-b marks' 'sample-c marks' 'sample-d marks' \
                'route route' 'route stations' 'dock rings' 'dock marker' 'dock leader' 'bounds brackets'; do
        read -r ns id <<< "$item"
        "$bin" --instance "$instance" draw update "vfx-gallery/survey/$ns" "$id" --glow 0.45
    done
else
    "$gallery" "$instance" "$scene" vfx_rover
fi
if [[ $scene == energy-review || $scene == energy-colors || $scene == review-system || $scene == structure-language || $scene == region-language || $scene == blur-review || $scene == data-lab || $scene == cloud-basics || $scene == cloud-scale ]]; then "$bin" --instance "$instance" scene play; fi
if [[ $view != keep ]]; then
  if [[ $scene == exhibition ]]; then
    "$gallery" exhibition-manifest vfx_rover > "$out/exhibition.manifest.json"
    bash "$root/scripts/focus-draw.sh" "$instance" "$out/exhibition.manifest.json" "$view"
  elif [[ $scene == cloud-basics || $scene == cloud-scale ]]; then
    "$root/target/release/examples/draw_${scene//-/_}" manifest > "$out/$scene.manifest.json"
    bash "$root/scripts/focus-draw.sh" "$instance" "$out/$scene.manifest.json" "$view"
  elif [[ $scene == data-lab ]]; then
    "$gallery" data-lab-manifest > "$out/data-lab.manifest.json"
    target=overview
    if [[ $view == cloud || $view == near ]]; then target=cloud; elif [[ $view == octree ]]; then target=octree; fi
    bash "$root/scripts/focus-draw.sh" "$instance" "$out/data-lab.manifest.json" "$target"
  elif [[ $scene == review || $scene == energy-review || $scene == review-colors || $scene == review-colors-light || $scene == review-finishes || $scene == review-system || $scene == structure-language || $scene == region-language || $scene == energy-colors || $scene == blur-review ]]; then
    "$gallery" review-manifest > "$out/review.manifest.json"
    jq --arg scene "$scene" --arg view "$view" '{overview: (.scenes[] | select(.id == $scene))}
        | if $view == "ghost" then .overview += {focus:[10.5,-4.5,0.4],distance:7,pitch:30,bearing:180} | del(.overview.bounds)
          elif $scene == "structure-language" and $view == "near" then
            .overview += {focus:[-12,7,1.6],distance:7,pitch:30,bearing:150} | del(.overview.bounds)
          elif $scene == "structure-language" and $view == "grazing" then
            .overview += {focus:[-12,7,1.6],distance:9,pitch:12,bearing:315} | del(.overview.bounds)
          elif $scene == "blur-review" and $view == "near" then
            .overview += {focus:[0,3.7,1],distance:14,pitch:35,bearing:180} | del(.overview.bounds)
          else . end' \
        "$out/review.manifest.json" > "$out/review.focus.json"
    bash "$root/scripts/focus-draw.sh" "$instance" "$out/review.focus.json" overview
  else
    case "$view" in
        near) bearing=160; pitch=32; distance=12 ;;
        far) bearing=160; pitch=42; distance=60 ;;
        grazing) bearing=180; pitch=12; distance=22 ;;
        overview) bearing=160; pitch=42; distance=26 ;;
    esac
    if [[ $scene == measurements && $view == overview ]]; then bearing=180; pitch=58; distance=21; fi
    if [[ ( $scene == corner-review || $scene == corner-helpers ) && $view == overview ]]; then bearing=160; pitch=48; distance=17; fi
    "$bin" --instance "$instance" instance camera look --lat 52.370216 --lon 4.895168 --bearing "$bearing" --pitch "$pitch" --distance "$distance" --above 1
  fi
fi
"$bin" --instance "$instance" --json instance info "$instance" > "$out/instance.json"
sha256sum "$bin" > "$out/binary.sha256"
sha256sum "$gallery" > "$out/gallery.sha256"
if [[ $scene == cloud-basics || $scene == cloud-scale ]]; then
    ready=0
    for ((i=0; i<60; i++)); do
        renderer_ok
        ready=1
        while IFS= read -r expected; do
            if ! grep -Fq "typed cloud activated $expected" "$out/viewer.log"; then ready=0; fi
        done < <(jq -r '.datasets[] | "\(.key.namespace)/\(.key.id) incarnation=\(.incarnation) generation=\(.stored_generation)"' "$out/$scene.typed.json")
        if (( ready )); then break; fi
        sleep 1
    done
    if (( ! ready )); then echo 'Typed cloud activation timed out; capture is invalid' >&2; exit 1; fi
fi
if (( record_seconds > 0 )); then
    clip="$out/$scene-$(date +%Y%m%d-%H%M%S)"
    "$bin" --instance "$instance" instance record start "$clip.mp4"
    captured=0
    for ((i=0; i<120; i++)); do
        if [[ -f "$clip.camera.csv" ]] && (( $(wc -l < "$clip.camera.csv") >= record_seconds * 60 + 2 )); then captured=1; break; fi
        sleep 1
    done
    "$bin" --instance "$instance" instance record stop
    if (( ! captured )); then echo 'Animation recording did not reach requested duration' >&2; exit 1; fi
    printf '%s\n' "$clip.mp4" > "$out/latest-recording.txt"
fi
sleep 2
renderer_ok
"$bin" --instance "$instance" instance screenshot "$out/$scene.window.png" --timeout 30
renderer_ok
if [[ $view != keep ]]; then cp -- "$out/$scene.window.png" "$out/$scene.$view.window.png"; fi
printf 'Live preview left open: %s (%s)\n' "$instance" "$scene"
