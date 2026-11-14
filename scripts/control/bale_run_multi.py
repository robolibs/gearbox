#!/usr/bin/env python3
"""Spawn several USD tractors, scatter USD bales, and collect them greedily.

Usage:
    python scripts/bale_run_multi.py [n_tractors] [n_bales] [field_size] [seed]

Defaults: n_tractors=3, n_bales=50, field_size=300, seed=42.

Each tractor is the same USD asset spawned under its own namespace, so each
gets its own machine agent and its own controller topics. Bale positions are
not guessed: Gearbox drops each bale onto the terrain and reports the settled
pose as a `pose` scene event, and the script drives off those. A contact
between a tractor and a bale arrives as a `harvested` event.
"""

from __future__ import annotations

import math
import random
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from gearbox_client import Gearbox, Machine  # noqa: E402

try:
    import ondrive  # type: ignore
except ModuleNotFoundError:
    ondrive = None  # type: ignore[assignment]


TRACTOR_USD_PATH = "bin/gearbox/assets/tractor.usd"
# The Poly Haven bale in metres; bale.usdz alone is drawn 100x too big.
BALE_USD_PATH = "markers/hay_bale.usda"
RING_RADIUS = 15.0
# Field speeds. Physics runs in real time now and the tyres deliver what is
# commanded; faster than this the tractor overshoots a bale between ticks.
MAX_SPEED_MPS = 2.0
# A bale counts as picked up this close to the tractor's origin (rear axle):
# the nose is ~2.5 m ahead and the bale ~0.7 m in radius. Waiting for a
# physical contact meant ramming a fixed bale and stopping dead.
REACH_M = 4.0
MAX_YAW_RPS = 1.2
TICK_DT = 0.10
MARKER_GAP_M = 0.6


def _planar(gx: float, gz: float) -> tuple[float, float]:
    """Gearbox (X, Z) → ondrive planar (x, y), heading sign preserved."""
    return gz, gx


def _build_tracker():
    tracker = ondrive.Tracker("pure_pursuit")
    cfg = ondrive.ControllerConfig.default_()
    cfg.goal_tolerance = 2.0
    cfg.angular_tolerance = math.pi
    cfg.lookahead_distance = 5.0
    cfg.output_units = "physical"
    tracker.set_config(cfg)
    cons = ondrive.RobotConstraints.default_()
    cons.steering_type = "ackermann"
    cons.wheelbase = WHEELBASE_M
    cons.max_linear_velocity = MAX_SPEED_MPS
    cons.min_linear_velocity = 0.0
    cons.max_angular_velocity = MAX_YAW_RPS
    cons.max_steering_angle = math.radians(40.0)
    tracker.init(cons)
    return tracker


# A target more than this far off the nose is reached by a forward turn at
# full lock first. The steering servo straightens at about 30°/s and the
# tractor keeps rotating meanwhile, so the turn lets go of the wheel once the
# heading error is down to that remaining swing; the tracker takes over when
# the wheels are straight. The tracker alone swings well past any corner
# sharper than this at field speed.
TURN_ENTER_RAD = math.radians(25.0)
TURN_SETTLED_RPS = 0.08
TURN_SPEED_MPS = 1.0
TURN_RADIUS_M = 3.4
WHEELBASE_M = 2.37
STEER_RATE_RPS = math.radians(30.0)
CMD_LATENCY_S = 0.2


def swing_left(speed: float, yaw_rate: float) -> float:
    """Heading the tractor still turns if its steering starts straightening now."""
    v = max(abs(speed), 0.3)
    steer = math.atan(WHEELBASE_M * abs(yaw_rate) / v)
    unwind = v * -math.log(math.cos(steer)) / (WHEELBASE_M * STEER_RATE_RPS)
    return unwind + abs(yaw_rate) * CMD_LATENCY_S


def turn_side(cx: float, cz: float, heading: float, tx: float, tz: float, err: float) -> float:
    """Turn toward the target, unless it sits inside that side's turning circle."""
    sign = 1.0 if err >= 0.0 else -1.0
    ox, oz = cx + sign * TURN_RADIUS_M * math.cos(heading), cz - sign * TURN_RADIUS_M * math.sin(heading)
    return -sign if math.hypot(tx - ox, tz - oz) < 1.1 * TURN_RADIUS_M else sign


def wrap_pi(angle: float) -> float:
    return (angle + math.pi) % (2.0 * math.pi) - math.pi


def bale_id_from(text: object) -> int | None:
    text = str(text or "")
    if not text.startswith("bale_"):
        return None
    digits = []
    for ch in text.removeprefix("bale_"):
        if not ch.isdigit():
            break
        digits.append(ch)
    return int("".join(digits)) if digits else None


class RobotProxy:
    def __init__(self, idx: int, namespace: str):
        self.idx = idx
        self.namespace = namespace
        self.machine: Machine | None = None
        self.pose = (0.0, 0.0)
        self.last_pose = (0.0, 0.0)
        self.heading_rad: float | None = None
        self.yaw_rate = 0.0
        self.speed = 0.0
        self.seen = False
        self.target_bale: int | None = None
        self.marker_bale: int | None = None
        self.collected: list[int] = []
        self._tracker = None
        self._tracker_target: tuple[float, float] | None = None
        self._tracker_path_yaw: float = 0.0
        self._tracker_last_tick: float | None = None
        self._maneuver: str = "forward"
        self._turn_sign: float = 0.0
        self._maneuver_t0: float = 0.0

    @property
    def is_idle(self) -> bool:
        return self.target_bale is None

    def attach(self, gb: Gearbox, timeout: float) -> bool:
        self.machine = gb.machine(self.namespace, timeout=timeout)
        self.machine.claim(hold_ms=1000, client=f"bale_run_multi/{self.namespace}")
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.refresh():
                return True
            time.sleep(0.05)
        return False

    def refresh(self) -> bool:
        """Pull the latest state sample. Gearbox state is Y-up: plane is X/Z."""
        if self.machine is None:
            return False
        s = self.machine.state()
        if s is None:
            return False
        self.last_pose = self.pose
        # Odom is REP-103 now (Z up); this file's (x, z) math predates that
        # and still expects the old sim-native (X, Z) ground-plane pair, so
        # reconstruct it here rather than touch every formula below: old_x
        # is the new y, old_z is the new x.
        self.pose = (s.y, s.x)
        self.heading_rad = s.heading_rad
        self.yaw_rate = s.yaw_rate
        self.speed = s.linear_speed
        self.seen = True
        return True

    def publish_cmd(self, speed: float, yaw_rate: float) -> None:
        if self.machine is not None:
            self.machine.cmd_vel(speed, yaw_rate)

    def stop(self) -> None:
        self.publish_cmd(0.0, 0.0)

    def heading_or_motion(self, target_heading: float) -> float:
        if self.heading_rad is not None:
            return self.heading_rad
        vx = self.pose[0] - self.last_pose[0]
        vz = self.pose[1] - self.last_pose[1]
        if math.hypot(vx, vz) > 0.05:
            return math.atan2(vx, vz)
        return target_heading


class SceneWatch:
    """Folds the host's scene events into bale poses and harvests."""

    def __init__(self, gb: Gearbox):
        self._events = gb.events()
        self.poses: dict[int, tuple[float, float, float, float]] = {}
        self._harvested: set[int] = set()

    def poll(self) -> None:
        for ev in self._events.poll():
            raw = str(ev.get("bale_id") or "")
            bale_id = int(raw) if raw.isdigit() else bale_id_from(ev.get("id"))
            if bale_id is None:
                continue
            if ev["kind"] == "pose":
                self.poses[bale_id] = (ev["x"], ev["y"], ev["z"], ev["top_y"])
            elif ev["kind"] == "harvested":
                self._harvested.add(bale_id)

    def drain_harvested(self) -> set[int]:
        self.poll()
        out = set(self._harvested)
        self._harvested.clear()
        return out


def spawn_usd_tractors(gb: Gearbox, robots: list[RobotProxy]) -> None:
    n = len(robots)
    for robot in robots:
        angle = (2.0 * math.pi * robot.idx) / max(1, n)
        x = RING_RADIUS * math.cos(angle)
        z = RING_RADIUS * math.sin(angle)
        # Heading convention matches state: 0° drives +Z, 90° drives +X.
        yaw_deg = math.degrees(math.atan2(-x, -z))
        status = gb.load_machine(
            robot.namespace,
            TRACTOR_USD_PATH,
            x=x,
            z=z,
            yaw_deg=yaw_deg,
            label=f"tractor_{robot.idx}.usd",
        )
        if not status.ok:
            raise RuntimeError(f"load {robot.namespace}: {status.message}")
        time.sleep(0.1)

    missing = [r.namespace for r in robots if not r.attach(gb, timeout=20.0)]
    if missing:
        raise RuntimeError("no state for spawned USD tractors: " + ", ".join(missing))
    print("spawned USD tractors: " + ", ".join(f"R{r.idx}={r.namespace}" for r in robots))


def set_target_marker(gb: Gearbox, robot_idx: int, pose: tuple[float, float, float, float] | None) -> None:
    mark_id = f"target_marker_{robot_idx}"
    if pose is None:
        gb.marker_delete(mark_id)
        return
    bx, _by, bz, top_y = pose
    gb.marker_set(mark_id, bx, top_y + MARKER_GAP_M, bz)


def pick_nearest_bale(robot: RobotProxy, bale_pos, visited: set[int], claimed: set[int]) -> int | None:
    cx, cz = robot.pose
    best_idx: int | None = None
    best_d = math.inf
    for bid, (bx, _by, bz, _top) in bale_pos.items():
        if bid in visited or bid in claimed:
            continue
        d = math.hypot(bx - cx, bz - cz)
        if d < best_d:
            best_idx = bid
            best_d = d
    return best_idx


def drive_toward(robot: RobotProxy, target: tuple[float, float]) -> float:
    if ondrive is None:
        raise RuntimeError("ondrive Python bindings not found; build them into .python-packages")

    tx, tz = target
    cx, cz = robot.pose
    d_now = math.hypot(tx - cx, tz - cz)

    if robot._tracker is None or robot._tracker_target != target:
        robot._tracker = _build_tracker()
        start_px, start_py = _planar(cx, cz)
        goal_px, goal_py = _planar(tx, tz)
        path_yaw = math.atan2(goal_py - start_py, goal_px - start_px)
        path = ondrive.Path()
        path.add_waypoint_xy(start_px, start_py, yaw=path_yaw)
        path.add_waypoint_xy(goal_px, goal_py, yaw=path_yaw)
        path.smoothen(2.0)
        robot._tracker.set_path(path)
        robot._tracker.set_goal(
            ondrive.Goal(
                target_pose=((goal_px, goal_py, 0.0), path_yaw),
                tolerance_position=2.0,
                tolerance_orientation=math.pi,
            )
        )
        robot._tracker_target = target
        robot._tracker_path_yaw = path_yaw
        robot._tracker_last_tick = None
        robot._maneuver = "forward"
        robot._turn_sign = 0.0

    heading = robot.heading_or_motion(robot._tracker_path_yaw)
    heading_err = wrap_pi(math.atan2(tx - cx, tz - cz) - heading)

    now = time.time()
    if robot._maneuver == "forward" and abs(heading_err) > TURN_ENTER_RAD and d_now > 2.0:
        robot._maneuver = "turn"
        robot._turn_sign = turn_side(cx, cz, heading, tx, tz, heading_err)

    if (
        robot._maneuver == "turn"
        and heading_err * robot._turn_sign > 0.0
        and abs(heading_err) <= swing_left(robot.speed, robot.yaw_rate)
    ):
        robot._maneuver = "straighten"
    if robot._maneuver == "straighten" and abs(robot.yaw_rate) < TURN_SETTLED_RPS:
        # Nose on the bale, wheels straight: replan from here and hand back.
        robot._maneuver = "forward"
        robot._tracker_target = None
        return d_now
    if robot._maneuver in ("turn", "straighten"):
        yaw = robot._turn_sign * MAX_YAW_RPS if robot._maneuver == "turn" else 0.0
        robot.publish_cmd(TURN_SPEED_MPS, yaw)
        return d_now

    px, py = _planar(cx, cz)
    state = ondrive.RobotState(pose=((px, py, 0.0), heading), allow_move=True, allow_reverse=False)
    dt = max(1e-3, now - robot._tracker_last_tick) if robot._tracker_last_tick else TICK_DT
    robot._tracker_last_tick = now
    cmd = robot._tracker.tick(state, dt)
    if not cmd.valid:
        robot.publish_cmd(0.0, 0.0)
        return d_now
    robot.publish_cmd(float(cmd.linear_velocity), float(cmd.angular_velocity))
    return d_now


def run(robots: list[RobotProxy], n_bales: int, field: float, seed: int, instance: str | None = None) -> None:
    rng = random.Random(seed)
    half = field / 2.0
    scatter = [(rng.uniform(-half, half), rng.uniform(-half, half)) for _ in range(n_bales)]

    run_id = int(time.time())
    run_nonce = f"bale_run_{run_id}"
    bale_runtime_ids = {i: f"bale_{i}_{run_nonce}" for i in range(n_bales)}

    gb = Gearbox(instance)
    gb.wait_ready()
    watch: SceneWatch | None = None
    try:
        # The sim's own ground (meadow, wheat) is the field: loading a USD
        # terrain would retire it for the rest of the session.
        print("clearing simulator")
        gb.clear()
        time.sleep(0.3)

        print(f"spawning {len(robots)} USD tractors")
        spawn_usd_tractors(gb, robots)

        print(f"scattering {n_bales} bales across {field:.0f} × {field:.0f} m field")
        watch = SceneWatch(gb)
        for i in range(32):
            gb.marker_delete(f"target_marker_{i}")
        for i, (bx, bz) in enumerate(scatter):
            gb.load(bale_runtime_ids[i], BALE_USD_PATH, x=bx, z=bz, nonce=run_nonce)

        print("waiting for Gearbox to report settled bale poses ...")
        t0 = time.time()
        while len(watch.poses) < n_bales and time.time() - t0 < 20.0:
            watch.poll()
            time.sleep(0.1)
        bale_pos = dict(watch.poses)
        if len(bale_pos) < n_bales:
            # Pose events we missed: the scene list has the current pose of
            # every prop, which for a bale on the ground is its rest pose.
            by_id = {o.get("id"): o for o in gb.list("prop")}
            for i in range(n_bales):
                o = by_id.get(bale_runtime_ids[i])
                if i not in bale_pos and o is not None:
                    bale_pos[i] = (o["x"], o["y"], o["z"], o["y"] + MARKER_GAP_M)
        n_targets = len(bale_pos)
        if n_targets < n_bales:
            print(f"  only {n_targets}/{n_bales} bale poses arrived; running with those")
        else:
            print(f"  got all {n_targets} bale poses")

        visited: set[int] = set()
        print(f"\n── DRIVING  R={len(robots)}  B={n_targets}  field={field:.0f} m ──\n")

        def mark_harvested(bale_id: int) -> None:
            if bale_id not in bale_pos or bale_id in visited:
                return
            visited.add(bale_id)
            gb.delete(bale_runtime_ids[bale_id])
            for robot in robots:
                if robot.target_bale == bale_id:
                    # Keep rolling: the next target is picked this tick. A
                    # zero command between bales braked the tractor to a
                    # dead stop at every bale.
                    robot.collected.append(bale_id)
                    robot.target_bale = None
                    print(
                        f"  R{robot.idx} touched bale_{bale_id}"
                        f"  collected={len(robot.collected)}"
                        f"  total={len(visited)}/{n_targets}"
                    )
                    return
            print(f"  contact harvested bale_{bale_id}  total={len(visited)}/{n_targets}")

        while len(visited) < n_targets:
            for bid in watch.drain_harvested():
                mark_harvested(bid)
            for robot in robots:
                robot.refresh()
            claimed = {r.target_bale for r in robots if r.target_bale is not None}

            for robot in robots:
                if robot.target_bale in visited:
                    robot.target_bale = None

                if robot.target_bale is not None:
                    bx, _by, bz, _top = bale_pos[robot.target_bale]
                    if drive_toward(robot, (bx, bz)) < REACH_M:
                        mark_harvested(robot.target_bale)

                if robot.is_idle:
                    pick = pick_nearest_bale(robot, bale_pos, visited, claimed)
                    if pick is None:
                        continue
                    bx, _by, bz, _top = bale_pos[pick]
                    robot.target_bale = pick
                    claimed.add(pick)
                    cx, cz = robot.pose
                    d = math.hypot(bx - cx, bz - cz)
                    print(f"  R{robot.idx} → bale_{pick}  target=({bx:+7.2f},{bz:+7.2f})  d={d:6.2f} m")

            for robot in robots:
                if robot.marker_bale != robot.target_bale:
                    target = None if robot.target_bale is None else bale_pos[robot.target_bale]
                    set_target_marker(gb, robot.idx, target)
                    robot.marker_bale = robot.target_bale

            time.sleep(TICK_DT)

    except KeyboardInterrupt:
        print("\ninterrupted — stopping all tractors")
    finally:
        for robot in robots:
            robot.stop()
            set_target_marker(gb, robot.idx, None)
            if robot.machine is not None:
                robot.machine.release()
        print(f"\nfinal: collected {len({b for r in robots for b in r.collected})}/{n_bales} bales")
        for robot in robots:
            print(f"  R{robot.idx} ({robot.namespace}): {len(robot.collected)} bales")


def main() -> None:
    n_tractors = int(sys.argv[1]) if len(sys.argv) > 1 else 3
    n_bales = int(sys.argv[2]) if len(sys.argv) > 2 else 50
    field = float(sys.argv[3]) if len(sys.argv) > 3 else 300.0
    seed = int(sys.argv[4]) if len(sys.argv) > 4 else 42
    run_id = int(time.time())
    robots = [RobotProxy(i, f"robot_{run_id}_{i}") for i in range(n_tractors)]
    run(robots, n_bales, field, seed)


if __name__ == "__main__":
    main()
