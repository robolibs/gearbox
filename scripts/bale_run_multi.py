#!/usr/bin/env python3
"""Like `bale_run.py`, but with N tractors. Each tractor picks the
nearest unclaimed bale in greedy nearest-neighbour fashion, drives to
it, then picks the next one. Pure greedy; no Hungarian, no LLM, no
external coordinator — just multiple of the same loop running in
parallel.

Usage:
    python bale_run_multi.py [n_tractors] [n_bales] [field_size] [seed]

Defaults: n_tractors=3, n_bales=50, field_size=200, seed=42."""

from __future__ import annotations

import math
import random
import sys
import time

import zenoh
import cbor2


PRESET = "tractor"
RING_RADIUS    = 10.0   # m — spawn ring around (0, 0)
GOTO_TOLERANCE = 2.0    # m arrival radius
GOTO_TIMEOUT   = 120.0  # s before we give up on a single goto
TICK_DT        = 0.2    # s per main-loop iteration


# ── Spawn helpers ───────────────────────────────────────────────────
def spawn_n_tractors(session: zenoh.Session, n: int) -> list[str]:
    """Spawn `n` tractors in a ring around (0, 0), facing inward.
    Returns the list of topic prefixes the simulator confirmed."""
    landed: list[dict] = []

    def on_spawned(sample: zenoh.Sample) -> None:
        try:
            landed.append(cbor2.loads(bytes(sample.payload)))
        except Exception:  # noqa: BLE001
            pass

    sub = session.declare_subscriber("gearbox/sim/spawned", on_spawned)
    # Subscriber-advertisement settle — the simulator's reply lands on
    # the very next Update frame after the spawn (~16 ms) and would
    # otherwise race our subscriber registration.
    time.sleep(0.5)

    for i in range(n):
        angle = (2.0 * math.pi * i) / n
        x = RING_RADIUS * math.cos(angle)
        z = RING_RADIUS * math.sin(angle)
        # gearbox forward = -Z; yaw = atan2(-x, -z) points the
        # vehicle's forward axis toward the origin.
        yaw_deg = math.degrees(math.atan2(-x, -z))
        session.put("gearbox/sim/spawn", cbor2.dumps({
            "preset": PRESET,
            "x": float(x), "y": 0.0, "z": float(z),
            "yaw_deg": float(yaw_deg),
            "player": False,
        }))
        time.sleep(0.1)  # let the sim allocate ids monotonically

    t0 = time.time()
    while len(landed) < n and time.time() - t0 < 5.0:
        time.sleep(0.05)
    del sub
    return [f"{ev['name']}_{ev['id']}" for ev in landed]


# ── Per-robot state ─────────────────────────────────────────────────
class RobotProxy:
    """Mirror of one gearbox vehicle on the python side. Holds the
    last odom pose, the goto-active edge tracker, and the bale we
    last asked it to visit (None when free)."""

    def __init__(self, idx: int, prefix: str):
        self.idx = idx
        self.prefix = prefix
        self.pose = (0.0, 0.0)
        self.goto_active = False
        self.goto_was_active = False
        self.target_bale: int | None = None
        self.goto_t0 = 0.0
        self.collected: list[int] = []

    def declare(self, session: zenoh.Session) -> None:
        def on_odom(sample: zenoh.Sample) -> None:
            try:
                d = cbor2.loads(bytes(sample.payload))
            except Exception:  # noqa: BLE001
                return
            p = d.get("position", [0.0, 0.0, 0.0])
            self.pose = (float(p[0]), float(p[2]))

        def on_status(sample: zenoh.Sample) -> None:
            try:
                d = cbor2.loads(bytes(sample.payload))
            except Exception:  # noqa: BLE001
                return
            self.goto_active = bool(d.get("active", False))

        # Bind the subscriber objects to `self` so the GC doesn't drop
        # them; zenoh stops delivering once the handle is collected.
        self._sub_odom = session.declare_subscriber(
            f"{self.prefix}/odom", on_odom
        )
        self._sub_status = session.declare_subscriber(
            f"{self.prefix}/goto_status", on_status
        )

    @property
    def is_idle(self) -> bool:
        return self.target_bale is None and not self.goto_active


# ── Bale helpers ────────────────────────────────────────────────────
def recolor_bale(
    session: zenoh.Session, bale_id: int, x: float, z: float, variant: str
) -> None:
    session.put(
        f"gearbox/markers/bale_{bale_id}",
        cbor2.dumps({
            "x": float(x), "z": float(z),
            "usd_path": "markers/bale.usda",
            "usd_variants": [["/bale", "color", variant]],
            "remove": False,
        }),
    )


def remove_bale(session: zenoh.Session, bale_id: int) -> None:
    session.put(
        f"gearbox/markers/bale_{bale_id}",
        cbor2.dumps({
            "x": 0.0, "z": 0.0, "height": 0.0, "radius": 0.0,
            "kind": "", "color": [0.0, 0.0, 0.0], "remove": True,
        }),
    )


# ── Greedy assignment ───────────────────────────────────────────────
def pick_nearest_bale(
    robot: RobotProxy,
    bales: list[tuple[float, float]],
    visited: set[int],
    claimed: set[int],
) -> int | None:
    """Closest bale to `robot` that no other robot owns and that
    hasn't already been collected. Returns the bale id, or None if
    nothing's available."""
    cx, cz = robot.pose
    best_idx, best_d = -1, math.inf
    for i, (bx, bz) in enumerate(bales):
        if i in visited or i in claimed:
            continue
        d = math.hypot(bx - cx, bz - cz)
        if d < best_d:
            best_d = d
            best_idx = i
    return best_idx if best_idx >= 0 else None


def send_goto(
    session: zenoh.Session, prefix: str, x: float, z: float
) -> None:
    session.put(
        f"{prefix}/goto",
        cbor2.dumps({
            "x": float(x), "z": float(z),
            "yaw_deg": 0.0,
            "tolerance": GOTO_TOLERANCE,
            "yaw_tolerance_deg": 0.0,
            "max_speed": 0.0,
            "cancel": False,
        }),
    )


def cancel_goto(session: zenoh.Session, prefix: str) -> None:
    session.put(
        f"{prefix}/goto",
        cbor2.dumps({
            "x": 0.0, "z": 0.0, "yaw_deg": 0.0,
            "tolerance": 0.0, "yaw_tolerance_deg": 0.0,
            "max_speed": 0.0, "cancel": True,
        }),
    )


# ── Main ────────────────────────────────────────────────────────────
def main() -> None:
    n_tractors = int(sys.argv[1])   if len(sys.argv) > 1 else 3
    n_bales    = int(sys.argv[2])   if len(sys.argv) > 2 else 50
    field      = float(sys.argv[3]) if len(sys.argv) > 3 else 200.0
    seed       = int(sys.argv[4])   if len(sys.argv) > 4 else 42

    rng = random.Random(seed)
    half = field / 2.0
    bales: list[tuple[float, float]] = [
        (rng.uniform(-half, half), rng.uniform(-half, half))
        for _ in range(n_bales)
    ]

    session = zenoh.open(zenoh.Config())

    # Wipe whatever's left from a previous run (vehicles + markers).
    session.put("gearbox/sim/reset", cbor2.dumps({"pause_clock": False}))
    # Unpause so the freshly-spawned tractors can drive.
    session.put("gearbox/sim/clock/command", cbor2.dumps({"SetPaused": False}))
    time.sleep(0.2)

    print(f"spawning {n_tractors} tractors")
    prefixes = spawn_n_tractors(session, n_tractors)
    if len(prefixes) < n_tractors:
        print(f"only got {len(prefixes)}/{n_tractors} confirmations — "
              f"is the simulator running?")
        session.close()
        sys.exit(1)

    robots = [RobotProxy(i, p) for i, p in enumerate(prefixes)]
    for r in robots:
        r.declare(session)
    time.sleep(0.3)  # let per-vehicle subscribers wire up

    print(f"scattering {n_bales} bales across {field:.0f} × {field:.0f} m field")
    for i, (bx, bz) in enumerate(bales):
        recolor_bale(session, i, bx, bz, "default")
    time.sleep(0.5)

    visited: set[int] = set()
    print(f"\n── DRIVING  R={n_tractors}  B={n_bales}  field={field:.0f} m ──\n")

    try:
        while len(visited) < n_bales:
            now = time.time()
            claimed = {
                r.target_bale for r in robots if r.target_bale is not None
            }

            for r in robots:
                # ── Edge: goto active → inactive  ⇒  arrival ──
                if r.goto_was_active and not r.goto_active:
                    if r.target_bale is not None:
                        bid = r.target_bale
                        if bid not in visited:
                            visited.add(bid)
                            r.collected.append(bid)
                            remove_bale(session, bid)
                            print(f"  R{r.idx} reached bale_{bid}  "
                                  f"  collected={len(r.collected)}  "
                                  f"  total={len(visited)}/{n_bales}")
                        r.target_bale = None
                r.goto_was_active = r.goto_active

                # ── Timeout — drop the goal so this robot can pick again ──
                if (r.target_bale is not None
                        and now - r.goto_t0 > GOTO_TIMEOUT):
                    print(f"  R{r.idx} TIMEOUT on bale_{r.target_bale} — releasing")
                    cancel_goto(session, r.prefix)
                    r.target_bale = None

                # ── If idle, pick the nearest unclaimed bale ──
                if r.is_idle:
                    pick = pick_nearest_bale(r, bales, visited, claimed)
                    if pick is None:
                        continue
                    bx, bz = bales[pick]
                    r.target_bale = pick
                    r.goto_t0 = now
                    claimed.add(pick)
                    cx, cz = r.pose
                    d = math.hypot(bx - cx, bz - cz)
                    print(f"  R{r.idx} → bale_{pick}  "
                          f"target=({bx:+7.2f},{bz:+7.2f})  "
                          f"d={d:6.2f} m")
                    recolor_bale(session, pick, bx, bz, "red")
                    send_goto(session, r.prefix, bx, bz)

            time.sleep(TICK_DT)

    except KeyboardInterrupt:
        print("\ninterrupted — cancelling all gotos")
        for r in robots:
            cancel_goto(session, r.prefix)
    finally:
        print(f"\nfinal: collected {len(visited)}/{n_bales} bales")
        for r in robots:
            print(f"  R{r.idx} ({r.prefix}): {len(r.collected)} bales")
        session.close()


if __name__ == "__main__":
    main()
