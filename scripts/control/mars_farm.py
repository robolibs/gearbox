"""Works the Mars base's farm with its robots, by the work plan its build
writes (`bin/gearbox/assets/world/mars_base_farm.json`):

- the gantries, the vertical farm's crane and the arch harvesters take in
  what is ripe within their reach and sow the places left empty, one place
  after another, and park when there is nothing to do; a harvester drives
  its lane to the table of each place before its arm reaches in;
- the excavators leave their docks along the lane past the chargers, dig
  regolith on their patch west of the soil plant and tip it into the
  plant's bins, round after round, one of them in the yard at a time;
- the washers brush the dust off the arches' and domes' glazing and off
  the solar rows, then brush the room (`clean_glazing`, by the washer) or
  the field (`clean_panels`) clean in the sim, and rest while dust settles.

    gearbox run --planet mars --world bin/gearbox/assets/world/mars_base.usda
    gearbox spawn machine bin/gearbox/assets/world/mars_base_fleet.usda --machine base
    python scripts/control/mars_farm.py [--robots base_fleet_eve,base_fleet_archapicker1]

Every robot runs as its own thread with its own client, so a robot that
waits on the sim never holds up another. A task says how long, in the
sim's seconds, it waits before its next step. A plant action names the
robot (`by`), and the sim refuses it until a part of that robot is within
reach, so a robot's work is real only once its head is there.
"""

from __future__ import annotations

import argparse
import json
import math
import re
import threading
import time
from pathlib import Path

from gearbox_client import Gearbox

PLAN = Path(__file__).resolve().parents[2] / "bin" / "gearbox" / "assets" / "world" / "mars_base_farm.json"
# Seconds a joint takes to settle once it should have arrived, and how long
# a robot may stop closing in on a place before it is given up.
SETTLE_S = 1.5
PATIENCE_S = 4.5
# Joints run below their top speed under load: short moves are given this
# many times their time at it.
SLOW = 2.0
# How far, metres or radians, a crossing joint may move with the head kept
# down at its work.
BESIDE = 0.5
IDLE_S = 30.0
# How old a read of the plants may be before a robot reads them again, s.
FRESH_S = 20.0
HOLD_MS = 120_000
# How often a driving robot is steered, how long it may drive without
# getting closer and how long it may turn on the spot, sim seconds.
STEER_S = 0.1
STALL_S = 10.0
TURN_S = 40.0
FAR = re.compile(r"is ([0-9.]+) m from")
# Robots of one room keep this far apart, metres between the places they
# work: a ridge robot's arm and a harvester's would meet over one table.
APART = 2.5
# Where each robot works now: machine id -> (room, place east and north).
WORKING: dict[str, tuple[str, list[float]]] = {}
WORKING_LOCK = threading.Lock()
# The soil plant's yard (the lane past the docks, the apron and the bins)
# holds one excavator at a time.
YARD = threading.Lock()


def patient(call, *args, tries: int = 6, **kwargs):
    """Calls the sim again when a busy host lets a request time out."""
    for attempt in range(tries):
        try:
            return call(*args, **kwargs)
        except RuntimeError as err:
            if "timed out" not in str(err) or attempt == tries - 1:
                raise
            time.sleep(1.0)


class Clock:
    """The sim's clock, read by one thread for every robot."""

    def __init__(self, instance: str | None):
        self.gb = Gearbox(instance)
        self.world_s = patient(self.gb.clock).world_s
        threading.Thread(target=self._follow, daemon=True).start()

    def _follow(self):
        while True:
            try:
                self.world_s = self.gb.clock().world_s
            except RuntimeError:
                pass
            time.sleep(0.05)

    def wait(self, seconds: float):
        """Waits `seconds` of the sim's time, however fast or slow it runs."""
        until = self.world_s + seconds
        while self.world_s < until:
            time.sleep(0.02)


_PLANTS: dict = {"at": -1.0e9, "plants": {}}
_PLANTS_LOCK = threading.Lock()


def plants(gb: Gearbox) -> dict[str, dict]:
    """Every plant by name, as the farm last read it: read again once
    `FRESH_S` of wall time have passed, so robots share one read."""
    with _PLANTS_LOCK:
        if time.time() - _PLANTS["at"] > FRESH_S:
            _PLANTS["plants"] = {p["plant"]: p for p in patient(gb.plants)}
            _PLANTS["at"] = time.time()
        return _PLANTS["plants"]


def wrap(angle: float) -> float:
    return (angle + math.pi) % (2 * math.pi) - math.pi


def clamp(v: float, limit: float) -> float:
    return max(-limit, min(limit, v))


class Robot:
    """A claimed machine of the plan."""

    def __init__(self, gb: Gearbox, machine: str, entry: dict, log, clock: Clock, listing: list[dict[str, str]]):
        self.gb = gb
        self.machine = machine
        self.entry = entry
        self.log = log
        self.clock = clock
        self.robot = patient(gb.machine, machine, listing=listing)
        patient(self.robot.claim, hold_ms=HOLD_MS, take=True, client="mars_farm")

    def set(self, link: str, name: str, value: float):
        status = patient(self.robot.set_value, link, name, value)
        if not status.ok:
            # The claim lapsed while the robot was busy: claim it again.
            patient(self.robot.claim, hold_ms=HOLD_MS, take=True, client="mars_farm")
            status = patient(self.robot.set_value, link, name, value)
        if not status.ok:
            raise RuntimeError(f"{self.machine}: {link}: {status.message}")

    def twist(self, forward: float, turn: float):
        """Drives with a twist, claiming the robot again if the claim lapsed."""
        try:
            status = self.robot.cmd_vel(forward, turn)
        except RuntimeError:
            status = None
        if status is None or not status.ok:
            patient(self.robot.claim, hold_ms=HOLD_MS, take=True, client="mars_farm")
            patient(self.robot.cmd_vel, forward, turn)

    def drive(self, to: list[float], heading: float | None = None, speed: float = 0.35, within: float = 0.03):
        """Drives to `to` (east, north). Along a lane `heading`, forward or
        back without turning off it; otherwise it turns to face the point and
        drives up to it. Gives up once it drives `STALL_S` without getting
        closer, or has turned on the spot for `TURN_S` in all."""
        best, stalled, turning, last = float("inf"), 0.0, 0.0, self.clock.world_s
        while True:
            s = self.robot.state(wait=0.5)
            now = self.clock.world_s
            dt, last = now - last, now
            dx, dy = to[0] - s.x, to[1] - s.y
            if heading is None:
                gap = math.hypot(dx, dy)
                if gap < within:
                    break
                err = wrap(math.atan2(dy, dx) - s.heading_rad)
                v = 0.0 if abs(err) > 0.35 else clamp(0.8 * gap, speed)
                w = clamp(1.8 * err, 0.5)
            else:
                along = dx * math.cos(heading) + dy * math.sin(heading)
                across = -dx * math.sin(heading) + dy * math.cos(heading)
                gap = abs(along)
                off = wrap(heading - s.heading_rad)
                if gap < within and abs(off) < 0.05:
                    break
                if gap < within:
                    # There, but turned off the lane: turns back onto it on the spot.
                    v, w = 0.0, clamp(1.5 * off, 0.4)
                else:
                    v = clamp(0.8 * along, speed)
                    w = clamp(1.5 * off + 1.2 * across * math.copysign(1.0, v), 0.4)
            # Turning on the spot gets no closer and is no stall.
            if gap < best - 0.01:
                best, stalled = gap, 0.0
            elif v != 0.0:
                stalled += dt
            turning += dt if v == 0.0 else 0.0
            if stalled > STALL_S or turning > TURN_S:
                break
            self.twist(v, w)
            yield STEER_S
        self.twist(0.0, 0.0)
        yield SETTLE_S

    def face(self, heading: float):
        for _ in range(int(TURN_S / STEER_S)):
            s = self.robot.state(wait=0.5)
            err = wrap(heading - s.heading_rad)
            if abs(err) < 0.03:
                break
            self.twist(0.0, clamp(1.8 * err, 0.5))
            yield STEER_S
        self.twist(0.0, 0.0)
        yield SETTLE_S

    def run(self):
        raise NotImplementedError

    def work(self):
        """Runs the robot's task in this thread, waiting in the sim's time."""
        for seconds in self.run():
            self.clock.wait(seconds)


class Tender(Robot):
    """A robot that tends plants by the plan's joint targets."""

    def __init__(self, *args):
        super().__init__(*args)
        self.at = dict(self.entry["park"])
        self.stop = None
        # The poses the head came down through to where it works now.
        self.via: list[dict] = []
        self.done = 0
        # Places the sim refused, left alone for a few rounds.
        self.refused: dict[str, int] = {}
        self.round = 0

    def move(self, joints: list[str], targets: dict) -> float:
        """Sends `joints` to their targets; how long they take to get there."""
        longest = 0.0
        for name in joints:
            if name not in targets or name == "drive":
                continue
            joint = self.entry["joints"][name]
            span = abs((targets[name] - self.at.get(name, targets[name])) * joint["range"])
            self.set(joint["link"], "position", targets[name])
            self.at[name] = targets[name]
            longest = max(longest, span / joint["speed"])
        return longest + SETTLE_S

    def crowded(self, at: list[float]) -> bool:
        """Whether another robot of the room works within APART of a place."""
        room = self.entry["room"]
        with WORKING_LOCK:
            return any(m != self.machine and r == room and math.dist(at, there) < APART for m, (r, there) in WORKING.items())

    def working(self, at: list[float] | None):
        with WORKING_LOCK:
            if at is None:
                WORKING.pop(self.machine, None)
            else:
                WORKING[self.machine] = (self.entry["room"], at)

    def beside(self, targets: dict) -> bool:
        """Whether the head can go straight to `targets` from where it works
        now: the robot stands where it is and its crossing joints move less
        than a place or two."""
        moves = self.entry["moves"]
        working = any(self.at[k] != self.entry["park"][k] for k in moves["leave"])
        if "drive" in targets:
            return working and targets["drive"] == self.stop
        return working and all(abs((targets[k] - self.at[k]) * self.entry["joints"][k]["range"]) < BESIDE for k in moves["cross"])

    def parked(self) -> bool:
        return all(self.at[k] == self.entry["park"][k] for k in self.entry["moves"]["leave"])

    def lift(self):
        """Takes the head back up the way it came down."""
        for via in reversed(self.via):
            yield SLOW * self.move(self.entry["moves"]["reach"], via)
        self.via = []

    def lower(self, targets: dict):
        """Brings the head down through a place's `via` poses, out of its
        park by way of the ready pose."""
        if "ready" in self.entry and self.parked():
            yield SLOW * self.move(self.entry["moves"]["reach"], self.entry["ready"])
        for via in targets.get("via", []):
            yield SLOW * self.move(self.entry["moves"]["reach"], via)
        self.via = targets.get("via", [])

    def park(self):
        """Back up the way the head came down, by way of the ready pose, to its park."""
        yield from self.lift()
        if "ready" in self.entry and not self.parked():
            yield SLOW * self.move(self.entry["moves"]["reach"], self.entry["ready"])
        yield SLOW * self.move(self.entry["moves"]["leave"], self.entry["park"])

    def cross(self, targets: dict):
        if "drive" in targets and targets["drive"] != self.stop:
            yield from self.drive(targets["drive"], self.entry["heading"])
            self.stop = targets["drive"]
        joints = [k for k in self.entry["moves"]["cross"] if k != "drive"]
        if joints:
            yield self.move(joints, targets)

    def jobs(self) -> list[tuple[str, str, str]]:
        """What to do now, in the plan's order: (op, place, crop)."""
        places = self.entry["places"]
        standing = {name: p for name, p in plants(self.gb).items() if name in places}
        jobs = []
        self.round += 1
        for place in places:
            if self.round - self.refused.get(place, -10) < 4:
                continue
            plant = standing.get(place)
            if plant is not None and plant["ripe"]:
                jobs.append(("harvest", place, plant["species"]))
            elif plant is not None and not plant["alive"]:
                jobs.append(("clear", place, plant["species"]))
            elif plant is None:
                jobs.append(("sow", place, self.entry["trays"][place.split("/")[0]]))
        return jobs

    def act(self, op: str, place: str, crop: str):
        tray, slot = place.split("/")
        keys = {"plant": place} if op != "sow" else {"tray": tray, "slot": slot, "species": crop}
        return patient(self.gb.greenhouse, op, by=self.machine, **keys)

    def close_in(self, op: str, place: str, crop: str):
        """Asks the sim until the robot is there: a refusal says how far it
        still is, and the asking stops once the robot stops closing in."""
        best, stalled = float("inf"), 0.0
        while True:
            status = self.act(op, place, crop)
            away = FAR.search(status.message)
            if status.ok or away is None:
                return status
            if float(away.group(1)) < best - 0.02:
                best, stalled = float(away.group(1)), 0.0
            elif stalled >= PATIENCE_S:
                return status
            yield SETTLE_S
            stalled += SETTLE_S

    def run(self):
        """The task: a generator yielding seconds to wait."""
        moves = self.entry["moves"]
        park = self.entry["park"]
        while True:
            jobs = self.jobs()
            if not jobs:
                self.working(None)
                yield from self.park()
                joints = [k for k in moves["cross"] if k != "drive"]
                if joints:
                    yield self.move(joints, park)
                yield IDLE_S
                continue
            for op, place, crop in jobs:
                targets = self.entry["places"][place]
                if self.crowded(targets["at"]):
                    continue
                self.working(targets["at"])
                if self.beside(targets):
                    yield from self.lift()
                    yield from self.lower(targets)
                    yield SLOW * self.move(moves["cross"] + moves["reach"], targets)
                else:
                    yield from self.park()
                    yield from self.cross(targets)
                    yield from self.lower(targets)
                status = yield from self.close_in(op, place, crop)
                if not status.ok and FAR.search(status.message):
                    yield SLOW * self.move(moves["reach"], targets)
                    status = yield from self.close_in(op, place, crop)
                done = ", ".join(f"{k} {v}" for k, v in status.props.items() if k != "message")
                self.log(f"{self.machine}: {op} {place} ({crop}): {'done ' + done if status.ok else status.message}")
                self.done += status.ok
                if not status.ok:
                    self.refused[place] = self.round
            # A round whose places were all taken by neighbours waits a little.
            yield SETTLE_S


class Digger(Robot):
    """An excavator digging regolith and tipping it into the soil plant."""

    def arms(self, pose: str, drum: float = 0.0):
        arm = self.entry["arm"][pose]
        links = self.entry["links"]
        self.set(links["arm_front"], "position", arm)
        self.set(links["arm_rear"], "position", arm)
        self.set(links["drum_front"], "velocity", drum)
        self.set(links["drum_rear"], "velocity", -drum)

    def go(self, *points, within: float = 0.15):
        for point in points:
            yield from self.drive(point, within=within)

    def in_field(self) -> bool:
        """Whether the robot stands past the lane's far end, on the dig ground."""
        s = self.robot.state(wait=0.5)
        (ax, ay), (bx, by) = self.entry["lane"]
        return ((s.x - ax) * (bx - ax) + (s.y - ay) * (by - ay)) / ((bx - ax) ** 2 + (by - ay) ** 2) > 1.0

    def run(self):
        bays = self.entry["bays"]
        apron, lane = self.entry["lane"]
        self.arms("carry")
        yield 4.0
        if not self.in_field():
            with YARD:
                s = self.robot.state(wait=0.5)
                docked = math.dist((s.x, s.y), self.entry["dock"]) < 1.0
                self.log(f"{self.machine}: {'off the dock' if docked else 'out of the yard'} and along the lane")
                yield from self.go(*([self.entry["leave"]] if docked else []), apron, lane)
        loads = 0
        while True:
            yield from self.go(self.entry["dig"], within=0.3)
            # Drums down and turning, creeping forward across the patch.
            self.arms("dig", self.entry["drum"]["dig"])
            yield 4.0
            for _ in range(int(8.0 / STEER_S)):
                self.twist(0.12, 0.0)
                yield STEER_S
            self.twist(0.0, 0.0)
            self.arms("carry")
            yield 4.0
            self.log(f"{self.machine}: dug load {loads + 1}, waiting for the yard")
            bay = bays[loads % len(bays)]
            east, north = bay["stop"][0] - bay["approach"][0], bay["stop"][1] - bay["approach"][1]
            with YARD:
                yield from self.go(lane, apron, bay["approach"])
                yield from self.face(math.atan2(north, east))
                yield from self.drive(bay["stop"], math.atan2(north, east), speed=0.15, within=0.05)
                self.arms("dump")
                self.set(self.entry["links"]["drum_front"], "velocity", self.entry["drum"]["dump"])
                yield 8.0
                loads += 1
                self.log(f"{self.machine}: tipped load {loads} into bay {loads % len(bays) or len(bays)}")
                self.arms("carry")
                yield 3.0
                yield from self.drive(bay["approach"], math.atan2(north, east), speed=0.2, within=0.1)
                yield from self.go(apron, lane)


class Washer(Robot):
    """A cleaning robot sweeping its glazing or its row of panels, then
    resting till the dust has settled again."""

    REST_S = 1800.0

    def joint(self, name: str, position: float, at: dict) -> float:
        """Sends a joint to `position`; how long it takes to get there."""
        self.set(self.entry["links"][name], "position", position)
        took = abs(position - at.get(name, 0.0)) * self.entry["full_s"][name]
        at[name] = position
        return SLOW * took + SETTLE_S

    def brush(self, on: bool):
        self.set(self.entry["links"]["brush"], "velocity", self.entry["brush"] if on else 0.0)

    def clean(self, op: str, **keys):
        status = patient(self.gb.greenhouse, op, **keys)
        self.log(f"{self.machine}: {op} {keys.get('room', '')}: {'done' if status.ok else status.message}")

    def run(self):
        kind = self.entry["kind"]
        at: dict[str, float] = {}
        while True:
            if kind == "arch_washer":
                sweep = self.entry["sweep"]
                for k, bay in enumerate(self.entry["bays"]):
                    yield self.joint("press", 0.0, at)
                    yield self.joint("travel", bay, at)
                    self.brush(True)
                    for s, press in sweep if k % 2 == 0 else sweep[::-1]:
                        yield max(self.joint("sweep", s, at), self.joint("press", press, at))
                    self.brush(False)
                yield self.joint("press", 0.0, at)
                yield self.joint("sweep", 0.5, at)
                self.clean("clean_glazing", room=self.entry["room"], by=self.machine)
                yield self.joint("travel", 0.0, at)
            elif kind == "dome_washer":
                self.brush(True)
                for k, band in enumerate(self.entry["bands"]):
                    yield self.joint("meridian", band, at)
                    yield self.joint("slew", 1.0 if k % 2 == 0 else 0.0, at)
                self.brush(False)
                yield self.joint("meridian", max(self.entry["glazed_from"], at.get("meridian", 0.0)), at)
                self.clean("clean_glazing", room=self.entry["room"], by=self.machine)
                yield self.joint("slew", 0.0, at)
                yield self.joint("meridian", 0.0, at)
            else:
                self.brush(True)
                yield self.joint("travel", 1.0, at)
                yield self.joint("travel", 0.0, at)
                self.brush(False)
                self.clean("clean_panels")
            yield self.REST_S


KINDS = {"rail_gantry": Tender, "polar_gantry": Tender, "stacker_crane": Tender, "mobile_harvester": Tender, "excavator": Digger,
         "arch_washer": Washer, "dome_washer": Washer, "panel_washer": Washer}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--plan", default=str(PLAN))
    ap.add_argument("--robots", help="comma-separated machine ids; every robot in the plan by default")
    ap.add_argument("--instance")
    args = ap.parse_args()

    plan = json.loads(Path(args.plan).read_text())["machines"]
    wanted = args.robots.split(",") if args.robots else list(plan)
    gb = Gearbox(args.instance)
    gb.wait_ready()
    patient(gb.play)
    clock = Clock(args.instance)
    # One read of the machines for every robot's own client.
    listing = patient(gb.machines)
    start = time.time()
    lock = threading.Lock()

    def log(text):
        with lock:
            print(f"[{time.time() - start:7.1f}s] {text}", flush=True)

    def work(machine: str):
        entry = plan[machine]
        try:
            robot = KINDS[entry["kind"]](Gearbox(args.instance), machine, entry, log, clock, listing)
            log(f"{machine}: {entry['kind']}" + (f", {len(entry['places'])} places in {entry['room']}" if "places" in entry else ""))
            robot.work()
        except Exception as err:  # noqa: BLE001 - one robot's failure is logged, the others go on
            log(f"{machine}: stopped: {err}")

    threads = [threading.Thread(target=work, args=(machine,), daemon=True) for machine in wanted]
    for thread in threads:
        thread.start()
    try:
        while any(thread.is_alive() for thread in threads):
            time.sleep(1.0)
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
