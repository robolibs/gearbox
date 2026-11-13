#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-"$root/target/draw-vfx/$(date +%Y%m%d-%H%M%S)"}
mode=${2:-window}
scenes=${3:-halo,hero,geometry,sensors,uncertainty,fields,measurements,panels,maps,frames,plain,robot,pick-point,pick-pose,controls,dense,survey,spatial,world-controls,world-dense}
case "$mode" in window|headless) ;; *) echo 'mode must be window or headless' >&2; exit 2 ;; esac
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin=${GEARBOX_CAPTURE_BIN:-"$root/target/release/gearbox"}
gallery=${GEARBOX_CAPTURE_GALLERY:-"$root/target/release/examples/draw_gallery"}
bin=$(readlink -f -- "$bin")
gallery=$(readlink -f -- "$gallery")
if [[ ! -x $bin || ! -x $gallery ]]; then echo 'Viewer and gallery binaries must be executable' >&2; exit 2; fi
instance="draw-vfx-$$"
export XDG_CONFIG_HOME="$out/config" XDG_STATE_HOME="$out/state"
export GEARBOX_LOG_FILE="$out/viewer.log" GEARBOX_WINDOW=1600x1000
export GEARBOX_CAMERA_DISTANCE=26 GEARBOX_CAMERA_ELEVATION=42 GEARBOX_DRAW_OPACITY=1
gb=("$bin" --instance "$instance" --timeout 30)
renderer_ok() {
    if [[ -f "$out/viewer.log" ]] && grep -E 'Out of Memory|OutOfMemory|Validation Error|Shader.*[Ee]rror|panicked at|error:.*wgsl|missing requested attribute|Error matching shader' "$out/viewer.log"; then
        echo 'Renderer error in viewer.log; capture is invalid' >&2
        return 1
    fi
}
shot() {
    renderer_ok
    "${gb[@]}" instance screenshot "$@"
    renderer_ok
}
started=0
cleanup() {
    result=$?
    if ! renderer_ok; then result=1; fi
    if (( started )); then "${gb[@]}" instance stop "$instance" || true; fi
    if [[ -f "$out/viewer.log" ]]; then
        grep -E '(^|[[:space:]])ERROR([[:space:]]|:)' "$out/viewer.log" > "$out/error-lines.log" || true
    fi
    if (( result )); then printf 'Not accepted: capture failed; inspect viewer.log and command output.\n' > "$out/NOT_ACCEPTED.txt"; fi
    exit "$result"
}
trap cleanup EXIT
launch=("$bin" run --name "$instance" --detach)
if [[ $mode == headless ]]; then launch+=(--headless 1600x1000); fi
"${launch[@]}" > "$out/launch.json"
started=1
"${gb[@]}" instance wait "$instance" --timeout 60
sha256sum "$bin" > "$out/binary.sha256"
sha256sum "$gallery" > "$out/gallery.sha256"
"${gb[@]}" --json instance info "$instance" > "$out/instance.start.json"
pid=$(sed -nE 's/^[[:space:]]*"pid":[[:space:]]*([0-9]+),?$/\1/p' "$out/instance.start.json")
if [[ ! $pid =~ ^[1-9][0-9]*$ ]]; then echo 'Viewer did not report a live PID' >&2; exit 1; fi
printf '%s\n' "$pid" > "$out/live.pid"
readlink -- "/proc/$pid/exe" > "$out/live-executable.txt"
sha256sum "/proc/$pid/exe" > "$out/live-executable.sha256"
if [[ $(cut -d' ' -f1 "$out/binary.sha256") != $(cut -d' ' -f1 "$out/live-executable.sha256") ]]; then
    echo 'Live viewer image differs from the requested capture binary' >&2
    exit 1
fi
git -C "$root" rev-parse HEAD > "$out/base-commit.txt"
git -C "$root" diff --ignore-submodules=all > "$out/source.patch"
{
    git -C "$root" diff --name-only --ignore-submodules=all
    git -C "$root" ls-files --others --exclude-standard -- \
        crates/gearbox-api/examples/draw_gallery.rs crates/gearbox-api/examples/draw_gallery \
        crates/gearbox-api/src/wire/draw_widgets/world.rs \
        bin/gearbox/src/host/draw_labels.rs bin/gearbox/src/host/draw_layout.rs bin/gearbox/src/host/draw_annotations.rs bin/gearbox/src/host/draw_frost.rs bin/gearbox/src/host/draw_frost.wgsl \
        bin/gearbox/src/draw/visual.rs bin/gearbox/src/draw/structure.rs bin/gearbox/src/draw/ghost_edges.rs bin/gearbox/src/draw/stroke_terrain.rs bin/gearbox/src/draw/lifecycle_tests.rs bin/gearbox/assets/fonts \
        bin/gearbox/src/draw/frost.rs bin/gearbox/src/draw/draw_frost.wgsl docs/DRAW_BLUR_CONTRACT.md \
        docs/DRAW_HALO_DESIGN_SYSTEM.md docs/DRAW_HALO_IMPLEMENTATION.md docs/DRAW_COLOR_CONTRACT.md \
        docs/DRAW_VFX_VERIFICATION.md docs/DRAW_WORLD_LANGUAGE.md \
        scripts/capture-draw.sh scripts/capture-draw-variants.sh scripts/preview-draw.sh scripts/focus-draw.sh scripts/draw-contact-sheets.sh \
        scripts/capture-draw-frames.sh scripts/capture-draw-grid.sh scripts/capture-draw-sensors.sh scripts/capture-draw-strokes.sh scripts/capture-draw-ghosts.sh scripts/capture-draw-frost-window.sh scripts/capture-draw-layout.sh scripts/capture-draw-drape.sh scripts/capture-draw-normals.sh scripts/test-draw-live.sh \
        tests/draw_frame_images.py tests/draw_grid_images.py tests/draw_stroke_images.py tests/draw_ghost_images.py tests/draw_drape_images.py tests/test_draw_drape_images.py tests/draw_normals_images.py tests/test_draw_normals_images.py tests/draw_live_updates.py \
        tests/fixtures/draw_vfx tests/test_draw_world.py tests/compare_draw_world.py
} | sort -u > "$out/source-files.txt"
tar -C "$root" -czf "$out/changed-source.tar.gz" -T "$out/source-files.txt"
"${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 42 --distance 26 --above 1
sleep 3
IFS=, read -r -a selected <<< "$scenes"
for scene in "${selected[@]}"; do
    if [[ $scene == robot || $scene == halo || $scene == styles || $scene == exhibition || $scene == energy-review || $scene == energy-colors ]] && ! "${gb[@]}" machine info vfx_rover >/dev/null 2>&1; then
        "${gb[@]}" spawn machine "$root/assets/machines/usd/clearpath_husky.usdz" --machine vfx_rover --at 0 0 0 --wait-timeout 90
        sleep 3
    fi
    "$gallery" "$instance" "$scene" vfx_rover | tee "$out/$scene.entities.txt"
    if [[ $scene == structure-language || $scene == region-language ]]; then "${gb[@]}" scene play; fi
    if [[ $scene == review || $scene == energy-review || $scene == review-colors || $scene == review-colors-light || $scene == review-finishes || $scene == structure-language || $scene == region-language || $scene == energy-colors ]]; then
        "$gallery" review-manifest | jq --arg scene "$scene" '{overview: (.scenes[] | select(.id == $scene))}' > "$out/$scene.manifest.json"
        bash "$root/scripts/focus-draw.sh" "$instance" "$out/$scene.manifest.json" overview
    elif [[ $scene == exhibition ]]; then
        "$gallery" exhibition-manifest vfx_rover > "$out/exhibition.manifest.json"
        bash "$root/scripts/focus-draw.sh" "$instance" "$out/exhibition.manifest.json" overview
    elif [[ $scene == corner-review || $scene == corner-helpers ]]; then
        "${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 48 --distance 17 --above 1
    else
        "${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 42 --distance 26 --above 1
    fi
    sleep 3
    shot "$out/$scene.viewport.png" --viewport --timeout 30
    if [[ $mode == window ]]; then
        shot "$out/$scene.window.png" --timeout 30
    fi
    if [[ $scene == structure-language ]]; then
        for view in 'near 150 30 7' 'grazing 315 12 9'; do
            read -r name bearing pitch distance <<< "$view"
            jq --argjson bearing "$bearing" --argjson pitch "$pitch" --argjson distance "$distance" \
                '.overview += {focus:[-12,7,1.6],bearing:$bearing,pitch:$pitch,distance:$distance} | del(.overview.bounds)' \
                "$out/$scene.manifest.json" > "$out/$scene.$name.manifest.json"
            bash "$root/scripts/focus-draw.sh" "$instance" "$out/$scene.$name.manifest.json" overview
            sleep 2
            shot "$out/$scene.$name.viewport.png" --viewport --timeout 30
            if [[ $mode == window ]]; then shot "$out/$scene.$name.window.png" --timeout 30; fi
        done
    fi
    if [[ $scene == robot ]]; then
        "${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 32 --distance 15 --above 0
        sleep 3
        shot "$out/robot.near.viewport.png" --viewport --timeout 30
        "${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 42 --distance 26 --above 1
        sleep 2
    fi
    if [[ $scene == hero || $scene == spatial || $scene == energy-review || $scene == energy-colors ]]; then
        "${gb[@]}" scene play
        "${gb[@]}" instance record start "$out/$scene.mp4"
        captured=0
        for ((i=0; i<180; i++)); do
            if [[ -f "$out/$scene.camera.csv" ]] && (( $(wc -l < "$out/$scene.camera.csv") >= 310 )); then captured=1; break; fi
            sleep 1
        done
        "${gb[@]}" instance record stop
        sleep 3
        if (( ! captured )); then echo 'Did not capture five seconds of animation' >&2; exit 1; fi
        shot "$out/$scene.later.viewport.png" --viewport --timeout 30
    fi
    if [[ $scene == halo || $scene == survey || $scene == spatial || $scene == world-dense ]]; then
        for view in 'near 160 42 18' 'far 160 42 60' 'grazing 180 12 22'; do
            read -r name bearing pitch distance <<< "$view"
            "${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing "$bearing" --pitch "$pitch" --distance "$distance" --above 1
            sleep 3
            shot "$out/$scene.$name.viewport.png" --viewport --timeout 30
        done
        "${gb[@]}" instance camera look --lat 52.370216 --lon 4.895168 --bearing 160 --pitch 42 --distance 26 --above 1
        "${gb[@]}" scene skip 12h
        sleep 4
        shot "$out/$scene.night.viewport.png" --viewport --timeout 30
        "${gb[@]}" --json scene status > "$out/$scene.night-clock.json"
        "${gb[@]}" scene skip 12h
        sleep 3
    fi
    if [[ $scene == world-controls ]]; then
        sleep 3
        shot "$out/world-controls.later.viewport.png" --viewport --timeout 30
    fi
    if [[ $scene == energy-review || $scene == energy-colors ]]; then
        jq '.overview += {focus:[10.5,-4.5,0.4],distance:7,pitch:30,bearing:150} | del(.overview.bounds)' \
            "$out/$scene.manifest.json" > "$out/$scene.ghost.manifest.json"
        bash "$root/scripts/focus-draw.sh" "$instance" "$out/$scene.ghost.manifest.json" overview
        sleep 2
        shot "$out/$scene.ghost.viewport.png" --viewport --timeout 30
        if [[ $mode == window ]]; then shot "$out/$scene.ghost.window.png" --timeout 30; fi
    fi
    if [[ $scene == exhibition ]]; then
        mkdir -p "$out/exhibition"
        if [[ -n ${GEARBOX_CAPTURE_FOCI:-} ]]; then
            tr ',' '\n' <<< "$GEARBOX_CAPTURE_FOCI" > "$out/exhibition.focus.txt"
        else
            jq -r '.sections[].id, .slots[].id' "$out/exhibition.manifest.json" > "$out/exhibition.focus.txt"
        fi
        while read -r focus; do
            bash "$root/scripts/focus-draw.sh" "$instance" "$out/exhibition.manifest.json" "$focus"
            sleep 0.5
            if [[ $mode == window ]]; then
                shot "$out/exhibition/$focus.window.png" --timeout 30
            else
                shot "$out/exhibition/$focus.viewport.png" --viewport --timeout 30
            fi
        done < "$out/exhibition.focus.txt"
    fi
done
if [[ $mode == window ]]; then
    geometry="$XDG_CONFIG_HOME/gearbox/window.txt"
    if [[ ! -f $geometry ]]; then echo 'Native window geometry unavailable for interaction acceptance' >&2; exit 1; fi
    cp -- "$geometry" "$out/interaction-window.txt"
    "$gallery" "$instance" verify-interactions "$bin" "$geometry" | tee "$out/interactions.jsonl"
else
    printf '%s\n' '{"not_run":"interaction acceptance requires the native window input loop"}' > "$out/interactions.jsonl"
fi
"${gb[@]}" --json instance info "$instance" > "$out/instance.json"
grep -E '(^|[[:space:]])ERROR([[:space:]]|:)' "$out/viewer.log" > "$out/error-lines.log" || true
renderer_ok
sha256sum --check "$out/binary.sha256" "$out/gallery.sha256" "$out/live-executable.sha256"
printf 'Drawing captures: %s\n' "$out"
