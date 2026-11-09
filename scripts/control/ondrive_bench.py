#!/usr/bin/env python3
"""Offline bench for ondrive controllers: the lab's machines, scenarios and
controller set-up driven on a kinematic model of each machine instead of
the simulator, for quick iteration between lab runs.

Ackermann machines steer as gearbox measured them: the steering angle ramps
at `steer_rate` towards the command after `delay` seconds and stops at
`max_steer`. Differential machines follow (v, w) after the same delay.

    python scripts/control/ondrive_bench.py kubota mpc,ilqr s_curve
    python scripts/control/ondrive_bench.py kubota all s_curve -v
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / ".python-packages"))

import ondrive  # noqa: E402
from ondrive_lab import (  # noqa: E402
    CONTROLLERS, MACHINES, PRED_DT, TICK, Reference, box_clearance, densify, predicted, scenario, tracker_for, wrap,
)

DELAY = 0.2
YAW_GAIN = 1.0


class Plant:
    """Kinematic machine with the lab's measured steering and acceleration."""

    def __init__(self, m, x=0.0, y=0.0, yaw=0.0):
        self.m = m
        self.x, self.y, self.yaw, self.v, self.w, self.steer = x, y, yaw, 0.0, 0.0, 0.0
        self.queue = [(0.0, 0.0)] * max(1, round(DELAY / TICK))

    def step(self, v_cmd, w_cmd, dt):
        self.queue.append((v_cmd, w_cmd))
        v_cmd, w_cmd = self.queue.pop(0)
        dv = max(-self.m.accel * dt, min(self.m.accel * dt, v_cmd - self.v))
        self.v += dv
        if self.m.steering == "ackermann":
            ref = max(abs(self.v), 0.05)
            want = math.atan(w_cmd * self.m.wheelbase / ref) if abs(w_cmd) > 1e-3 else 0.0
            want = max(-self.m.max_steer, min(self.m.max_steer, want))
            rate = (self.m.steer_rate or 10.0) * dt
            self.steer += max(-rate, min(rate, want - self.steer))
            self.w = self.v * math.tan(self.steer) / self.m.wheelbase
        else:
            self.w = YAW_GAIN * max(-self.m.wmax, min(self.m.wmax, w_cmd))
        self.yaw = wrap(self.yaw + self.w * dt)
        a = self.m.rear_offset if self.m.steering == "ackermann" else 0.0
        self.x += (self.v * math.cos(self.yaw) - a * self.w * math.sin(self.yaw)) * dt
        self.y += (self.v * math.sin(self.yaw) + a * self.w * math.cos(self.yaw)) * dt


def run(mname: str, kind: str, sname: str, verbose: bool, params: dict | None = None) -> dict:
    m = MACHINES[mname]
    sc = scenario(sname, m.scale)
    pts = list(sc.points)
    path = ondrive.Path(is_closed=sc.closed)
    for i, (x, y) in enumerate(pts):
        nx, ny = pts[min(i + 1, len(pts) - 1)]
        px, py = pts[max(i - 1, 0)]
        yaw = math.atan2(ny - py, nx - px)
        sp = sc.speeds[i] if sc.speeds else None
        if sp is not None and sp < 0:
            yaw = wrap(yaw + math.pi)
        path.add_waypoint_xy(x, y, yaw=yaw, speed=sp)
    path.smoothen(0.2 * max(1.0, m.scale ** 0.5))
    ref = Reference(densify(pts, 0.1), sc.closed)
    t = tracker_for(kind, m, sc, params or {})
    if sc.goal:
        gx, gy, gyaw = sc.goal
        t.set_goal(ondrive.Goal(target_pose=((gx, gy, 0.0), gyaw), tolerance_position=0.25 * m.scale, tolerance_orientation=0.25))
    else:
        t.set_path(path)
    plant = Plant(m)
    limit = sc.time_limit or (ref.lap_len * (sc.laps or 1.0) / (0.5 * 0.8 * m.vmax) + 15.0)
    errs, flips, last_w, status, time_s = [], 0, 0.0, "timeout", 0.0
    dws, prev_w = [], 0.0
    lost = max(3.0, 2.0 * m.length)
    movers = [{"spec": mv, "at": list(mv["start"]), "going": mv.get("meet") is None} for mv in sc.movers]
    clear = float("inf")
    while time_s < limit:
        obs = [(x, y, 0.0, 0.0, r) for x, y, r in sc.static]
        for mv in movers:
            spec, (ix, iy, iyaw) = mv["spec"], mv["at"]
            if not mv["going"]:
                mx, my = spec["meet"]
                v_robot = max(abs(plant.v), 0.3 * 0.8 * m.vmax)
                mv["going"] = math.hypot(mx - ix, my - iy) / spec["speed"] + 1.2 >= math.hypot(mx - plant.x, my - plant.y) / v_robot
            speed = spec["speed"] if mv["going"] else 0.0
            vx, vy = speed * math.cos(iyaw), speed * math.sin(iyaw)
            obs.append((ix, iy, vx, vy, 0.6))
            mv["at"] = [ix + vx * TICK, iy + vy * TICK, iyaw]
        world = ondrive.World()
        for i, o in enumerate(obs):
            world.add_obstacle(i, o[4], [predicted(o)], dt=PRED_DT)
        now_clear = min((box_clearance(plant.x, plant.y, plant.yaw, m.length, m.width, o[0], o[1], o[4], m.body_offset) for o in obs),
                        default=float("inf"))
        clear = min(clear, now_clear)
        state = ondrive.RobotState(pose=((plant.x, plant.y, 0.0), plant.yaw), velocity=(plant.v, plant.w, 0.0),
                                   allow_move=True, allow_reverse=sc.reverse)
        cmd = t.tick(state, TICK, world)
        v, w = (cmd.linear_velocity, cmd.angular_velocity) if cmd.valid else (0.0, 0.0)
        plant.step(v, w, TICK)
        time_s += TICK
        dws.append(w - prev_w)
        prev_w = w
        err = ref.error(plant.x, plant.y)
        errs.append(err)
        if abs(w) > 0.2 * m.wmax and last_w * w < 0:
            flips += 1
        if abs(w) > 0.2 * m.wmax:
            last_w = w
        if verbose and len(errs) % (5 if verbose > 1 else 20) == 0:
            aim = t.current_target() or (float("nan"), float("nan"), 0.0)
            movers_at = " ".join(f"({mv['at'][0]:.1f},{mv['at'][1]:+.1f})" for mv in movers)
            print(f"  t={time_s:5.1f} x={plant.x:6.2f} y={plant.y:+6.2f} yaw={plant.yaw:+.2f} err={err:5.2f} v={v:+.2f} "
                  f"w={w:+.2f} steer={plant.steer:+.2f} c={now_clear:5.2f} aim=({aim[0]:.1f},{aim[1]:+.1f}) {movers_at} "
                  f"{cmd.status_message}")
        if t.is_path_completed() or (sc.goal and t.is_goal_reached()):
            status = "completed"
            break
        if err > lost:
            status = f"lost {err:.1f} m"
            break
    rms = math.sqrt(sum(e * e for e in errs) / max(1, len(errs)))
    return {"kind": kind, "status": status, "time": time_s, "rms": rms, "max": max(errs, default=0.0),
            "flips": flips / max(time_s / 60.0, 1e-9), "clear": clear,
            "jitter": math.sqrt(sum(d * d for d in dws) / max(1, len(dws)))}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("machine")
    ap.add_argument("controller")
    ap.add_argument("scenario")
    ap.add_argument("-v", "--verbose", action="count", default=0)
    ap.add_argument("--params", default="{}", help="controller parameters as JSON")
    ap.add_argument("--yaw-gain", type=float, default=1.0, help="share of the commanded turn a skid-steer delivers")
    a = ap.parse_args()
    global YAW_GAIN
    YAW_GAIN = a.yaw_gain
    kinds = CONTROLLERS if a.controller == "all" else a.controller.split(",")
    for sname in a.scenario.split(","):
        for kind in kinds:
            r = run(a.machine, kind, sname, a.verbose, json.loads(a.params))
            print(f"{a.machine:8} {kind:18} {sname:12} {r['status'][:14]:14} {r['time']:6.1f} {r['rms']:6.2f} "
                  f"{r['max']:6.2f} {r['flips']:5.1f} {r['clear']:6.2f} {r['jitter']:5.3f}", flush=True)


if __name__ == "__main__":
    main()
