#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out="$root/target/draw-vfx/mission"
mkdir -p "$out"
bin="$root/target/release/gearbox"
action=${1:-start}
case "$action" in start|stop) ;; *) echo 'Use start or stop' >&2; exit 2 ;; esac
if [[ -f "$out/controller.pid" ]]; then
    pid=$(cat "$out/controller.pid")
    if [[ $pid =~ ^[1-9][0-9]*$ && -r /proc/$pid/cmdline ]] && \
       tr '\0' ' ' < "/proc/$pid/cmdline" | grep -q 'scripts/control/draw_mission.py draw-mission-'; then
        kill -TERM "$pid"
        for _ in {1..50}; do if ! kill -0 "$pid" 2>/dev/null; then break; fi; sleep .1; done
        if [[ -r /proc/$pid/cmdline ]] && \
           tr '\0' ' ' < "/proc/$pid/cmdline" | grep -q 'scripts/control/draw_mission.py draw-mission-'; then
            kill -KILL "$pid" 2>/dev/null || true
        fi
    fi
fi
if [[ -f "$out/instance.txt" ]]; then
    old=$(cat "$out/instance.txt")
    if [[ $old == draw-mission-* ]]; then "$bin" --instance "$old" instance stop "$old" || true; fi
fi
if [[ $action == stop ]]; then exit 0; fi
instance="draw-mission-$$"
run="$out/$instance"
mkdir -p "$run"
export XDG_CONFIG_HOME="$run/config" XDG_STATE_HOME="$run/state" GEARBOX_LOG_FILE="$run/viewer.log"
export GEARBOX_WINDOW=1600x1000 GEARBOX_DRAW_OPACITY=1
if [[ -d "$root/../../.python-packages" ]]; then export PYTHONPATH="$root/../../.python-packages${PYTHONPATH:+:$PYTHONPATH}"; fi
controller=""
cleanup() {
    result=$?
    if (( result )); then
        if [[ -n $controller ]]; then kill -TERM "$controller" 2>/dev/null || true; fi
        "$bin" --instance "$instance" instance stop "$instance" || true
        printf 'Mission startup failed; inspect controller.log and viewer.log.\n' > "$run/NOT_ACCEPTED.txt"
    fi
}
trap cleanup EXIT
"$bin" run --name "$instance" --detach > "$run/launch.json"
printf '%s\n' "$instance" > "$out/instance.txt"
printf '%s\n' "$run" > "$out/latest-run.txt"
gb=("$bin" --instance "$instance" --timeout 30)
"${gb[@]}" instance wait "$instance" --timeout 90
"${gb[@]}" spawn machine "$root/assets/machines/usd/clearpath_husky.usdz" --machine mission_husky --at 0 0 0 --wait-timeout 90
sleep 2
"${gb[@]}" instance camera look --lat 52.3702564 --lon 4.895168 --bearing 180 --pitch 32 --distance 18 --above .5
nohup setsid python3 -u "$root/scripts/control/draw_mission.py" "$instance" --output "$run" > "$run/controller.log" 2>&1 < /dev/null &
controller=$!
printf '%s\n' "$controller" > "$out/controller.pid"
for _ in {1..90}; do
    if [[ -f "$run/ready.json" ]]; then break; fi
    if ! kill -0 "$controller" 2>/dev/null; then cat "$run/controller.log"; exit 1; fi
    sleep 1
done
[[ -f "$run/ready.json" ]]
sleep 3
if grep -E 'OutOfMemory|Out of Memory|Validation Error|panicked at|missing requested attribute|Error matching shader' "$run/viewer.log"; then exit 1; fi
"${gb[@]}" --json instance info "$instance" > "$run/instance.json"
pid=$(jq -r '.pid' "$run/instance.json")
sha256sum "$bin" "/proc/$pid/exe" > "$run/binary.sha256"
[[ $(sha256sum "$bin" | cut -d' ' -f1) == $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) ]]
"${gb[@]}" instance screenshot "$run/mission.window.png" --timeout 30
printf 'Live mission: %s\nController PID: %s\nEvidence: %s\nStop with: oslo make preview-draw-mission --args=stop\n' "$instance" "$controller" "$run"
