#!/usr/bin/env bash
# Release A scale bench: one uncapped headless instance per scenario, frozen
# profile size/MSAA/limits, warmup then a measured window, per-frame host log.
# Usage: bench-draw-data.sh reference|low-memory OUTPUT
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
profile=${1:?reference or low-memory}
out=${2:?output directory}
case $profile in
    reference) size=1920x1080 msaa=4 scenarios=${GEARBOX_BENCH_SCENARIOS:-"cloud-1m cloud-5m-paged cloud-100k-live octree-100k octree-500k mixed cycles"} ;;
    low-memory) size=1280x720 msaa=1 scenarios=${GEARBOX_BENCH_SCENARIOS:-"cloud-1m reject cycles"} ;;
    *) echo "unknown profile $profile" >&2; exit 2 ;;
esac
warmup=${GEARBOX_BENCH_WARMUP:-10}
measure=${GEARBOX_BENCH_SECONDS:-60}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
bin="$root/target/release/gearbox"
bench="$root/target/release/examples/draw_bench"
scale="$root/target/release/examples/draw_cloud_scale"
gallery="$root/target/release/examples/draw_gallery"
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
now() { date +%s.%N; }

{
    printf '{"schema":"gearbox.draw.data.bench.v1","run_id":"%s","profile":"%s","size":"%s","msaa":%s,' "$run_id" "$profile" "$size" "$msaa"
    printf '"warmup_s":%s,"measure_s":%s,"uncapped":true,' "$warmup" "$measure"
    printf '"source_head":"%s","source_diff_sha256":"%s",' "$(git -C "$root" rev-parse HEAD)" "$(git -C "$root" diff | sha256sum | cut -d' ' -f1)"
    printf '"binary_sha256":"%s",' "$(sha256sum "$bin" | cut -d' ' -f1)"
    printf '"device":"%s",' "$(nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader 2>/dev/null | head -1)"
    printf '"competing_instances":%s,' "$("$bin" --json instance list 2>/dev/null | jq -c '[.[]?.name]' 2>/dev/null || echo '[]')"
    printf '"gpu_utilization_at_start":"%s",' "$(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null | head -1)"
    printf '"scenarios":"%s","started":"%s"}\n' "$scenarios" "$(date -u +%FT%TZ)"
} > "$out/manifest.json"

launch() {
    local dir=$1
    instance="draw-vfx-bench-$$-$(basename "$dir")"
    gb=("$bin" --instance "$instance" --timeout 30)
    XDG_CONFIG_HOME="$dir/config" XDG_STATE_HOME="$dir/state" GEARBOX_LOG_FILE="$dir/viewer.log" \
        GEARBOX_DRAW_RESOURCE_PROBE="$dir" GEARBOX_DRAW_BENCH=1 GEARBOX_HEADLESS_FPS=0 GEARBOX_MSAA="$msaa" \
        GEARBOX_DRAW_PROFILE="$profile" \
        "$bin" run --name "$instance" --headless "$size" --detach > "$dir/launch.json"
    "${gb[@]}" instance wait "$instance" --timeout 90 > /dev/null
}

look() { "${gb[@]}" instance camera look --lat "$1" --lon "$2" --bearing "$3" --pitch "$4" --distance "$5" --above 1 ${6:+--over "$6"} > /dev/null; }

window() {
    local dir=$1 seconds=${2:-$measure}
    sleep "$warmup"
    local start; start=$(now)
    sleep "$seconds"
    printf '{"start_unix_s":%s,"end_unix_s":%s}\n' "$start" "$(now)" >> "$dir/window.jsonl"
}

finish() {
    local dir=$1
    "${gb[@]}" instance stop "$instance" > /dev/null 2>&1 || true
    if grep -E 'Validation Error|Out of Memory|OutOfMemory|panicked at|failed to process shader' "$dir/viewer.log" > "$dir/renderer_errors.txt"; then
        echo "renderer errors in $(basename "$dir")" >&2
    fi
}

# 52.370216 4.895168 is home; the bench field spans 102 m east-west and north 0-102 m.
field=(52.370676 4.895168)
instance=""
trap '[[ -n $instance ]] && "$bin" --instance "$instance" instance stop "$instance" > /dev/null 2>&1' EXIT
run_scenario() {
    local scenario=$1 dir=$2
    launch "$dir"
    case $scenario in
            cloud-1m)
                look "${field[@]}" 180 45 150
                "$bench" "$instance" cloud 1000000 >> "$dir/events.jsonl"
                window "$dir"
                "$bench" "$instance" sparse 1000000 >> "$dir/events.jsonl"
                sleep 3
                "$bench" "$instance" restyle >> "$dir/events.jsonl"
                sleep 3
                ;;
            cloud-5m-paged)
                GEARBOX_CLOUD_POINTS=5000000 "$scale" "$instance" > "$dir/scale.json"
                look 52.370216 4.895168 165 52 95
                ( sleep "$warmup"; for _ in $(seq 1 $(( measure / 8 ))); do
                    look 52.370216 4.895168 200 25 40 3; sleep 4; look 52.370216 4.895168 120 60 160 3; sleep 4
                done ) &
                mover=$!
                window "$dir"
                wait "$mover" || true
                "$bench" "$instance" status survey vfx-gallery/cloud-scale >> "$dir/events.jsonl"
                ;;
            cloud-100k-live)
                look "${field[@]}" 180 45 60
                "$bench" "$instance" live 100000 10 $(( warmup + measure + 2 )) >> "$dir/events.jsonl" &
                writer=$!
                window "$dir"
                wait "$writer"
                ;;
            octree-100k|octree-500k)
                look 52.371000 4.894724 200 40 60
                leaves=${scenario#octree-}; leaves=${leaves%k}000
                "$bench" "$instance" octree "$leaves" >> "$dir/events.jsonl"
                window "$dir"
                ;;
            mixed)
                "$gallery" "$instance" > "$dir/gallery.json" 2>&1 || true
                "${gb[@]}" spawn machine "$root/assets/machines/usd/clearpath_husky.usdz" --machine bench_robot --at 10 10 --wait-timeout 120 > /dev/null
                look 52.370216 4.895168 165 45 120
                window "$dir" $(( measure / 2 ))
                "$bench" "$instance" cloud 1000000 >> "$dir/events.jsonl"
                "$bench" "$instance" octree 100000 >> "$dir/events.jsonl"
                window "$dir" $(( measure / 2 ))
                ;;
            cycles)
                look "${field[@]}" 180 45 150
                if [[ $profile == reference ]]; then count=1000000; else count=100000; fi
                "$bench" "$instance" cycles 20 "$count" >> "$dir/events.jsonl"
                sleep 2
                ;;
            reject)
                look "${field[@]}" 180 45 150
                "$bench" "$instance" reject 100000 5000000 >> "$dir/events.jsonl"
                ;;
        esac
}
for scenario in $scenarios; do
    dir="$out/$scenario"
    rm -rf "$dir"
    mkdir -p "$dir"
    echo "== $scenario" >&2
    instance="draw-vfx-bench-$$-$scenario"
    gb=("$bin" --instance "$instance" --timeout 30)
    set +e
    ( set -e; run_scenario "$scenario" "$dir" )
    status=$?
    set -e
    if (( status )); then
        printf '{"scenario":"%s","exit":%s}\n' "$scenario" "$status" > "$dir/scenario_failed.json"
    fi
    finish "$dir"
    instance=""
done
printf '{"finished":"%s"}\n' "$(date -u +%FT%TZ)" >> "$out/manifest.json"
python3 "$root/scripts/bench_draw_data.py" "$out" "$profile" "$root/tests/fixtures/draw_data/profiles.json"
