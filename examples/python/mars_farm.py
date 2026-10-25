"""Works the Mars base's farm with its robots, by the work plan its build
writes (`bin/gearbox/assets/world/mars_base_farm.json`):

- the gantries, the vertical farm's crane and the arch harvesters take in
  what is ripe within their reach and sow the places left empty, one place
  after another, and park when there is nothing to do; a harvester drives
  its lane to the table of each place before its arm reaches in;
- the excavators dig regolith west of the soil plant and tip it into the
  plant's bins, round after round;
- the washers brush the dust off the arches' and domes' glazing and off
  the solar rows, then brush the room (`clean_glazing`, by the washer) or
  the field (`clean_panels`) clean in the sim, and rest while dust settles.

    gearbox run --planet mars --world bin/gearbox/assets/world/mars_base.usda
    gearbox spawn machine bin/gearbox/assets/world/mars_base_fleet.usda --machine base
    python examples/python/mars_farm.py [--robots base_fleet_eve,base_fleet_archapicker1]

Every robot runs as its own task on one loop: a task says how long it
waits before its next step, so many robots work at once over one client.
A plant action names the robot (`by`), and the sim refuses it until a part
of that robot is within reach, so a robot's work is real only once its head
is there.
"""

from __future__ import annotations

import argparse
import heapq
import json
import math
import re
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
# getting closer and how long it may turn on the spot, seconds.
STEER_S = 0.1
STALL_S = 10.0
TURN_S = 40.0
FAR = re.compile(r"is ([0-9.]+) m from")
# Robots of one room keep this far apart, metres between the places they
# work: a ridge robot's arm and a harvester's would meet over one table.
APART = 2.5
# Where each robot works now: machine id -> (room, place east and north).
WORKING: dict[str, tuple[str, list[float]]] = {}


def patient(call, *args, tries: int = 6, **kwargs):
    """Calls the sim again when a busy host lets a request time out."""
    for attempt in range(tries):
        try:
            return call(*args, **kwargs)
        except RuntimeError as err:
            if "timed out" not in str(err) or attempt == tries - 1:
                raise
            time.sleep(1.0)


_PLANTS: dict = {"at": -1.0e9, "plants": {}}


def plants(gb: Gearbox) -> dict[str, dict]:
    """Every plant by name, as the farm last read it: read again once
    `FRESH_S` of wall time have passed, so robots share one read."""
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

    def __init__(self, gb: Gearbox, machine: str, entry: dict, log):
        self.gb = gb
        self.machine = machine
        self.entry = entry
        self.log = log
        self.robot = patient(gb.machine, machine)
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
        drives up to it. Gives up once it stops getting closer."""
        best, stalled = float("inf"), 0.0
        while True:
            s = self.robot.state(wait=0.5)
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
                if gap < within:
                    break
                v = clamp(0.8 * along, speed)
                w = clamp(1.5 * wrap(heading - s.heading_rad) + 1.2 * across * math.copysign(1.0, v), 0.4)
            if gap < best - 0.01:
                best, stalled = gap, 0.0
            elif stalled > STALL_S:
                break
            # Turning on the spot gets no closer and is no stall.
            stalled += STEER_S if v != 0.0 else 0.0
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


class Tender(Robot):
    """A robot that tends plants by the plan's joint targets."""

    def __init__(self, gb, machine, entry, log):
        super().__init__(gb, machine, entry, log)
        self.at = dict(entry["park"])
        self.stop = None
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
        return any(m != self.machine and r == room and math.dist(at, there) < APART for m, (r, there) in WORKING.items())

    def beside(self, targets: dict) -> bool:
        """Whether the head can go straight to `targets` from where it works
        now: the robot stands where it is and its crossing joints move less
        than a place or two."""
        moves = self.entry["moves"]
        working = any(self.at[k] != self.entry["park"][k] for k in moves["leave"])
        if "drive" in targets:
            return working and targets["drive"] == self.stop
        return working and all(abs((targets[k] - self.at[k]) * self.entry["joints"][k]["range"]) < BESIDE for k in moves["cross"])

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
                WORKING.pop(self.machine, None)
                yield SLOW * self.move(moves["leave"], park)
                joints = [k for k in moves["cross"] if k != "drive"]
                if joints:
                    yield self.move(joints, park)
                yield IDLE_S
                continue
            for op, place, crop in jobs:
                targets = self.entry["places"][place]
                if self.crowded(targets["at"]):
                    continue
                WORKING[self.machine] = (self.entry["room"], targets["at"])
                if self.beside(targets):
                    yield SLOW * self.move(moves["cross"] + moves["reach"], targets)
                else:
                    yield SLOW * self.move(moves["leave"], park)
                    yield from self.cross(targets)
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

    def run(self):
        bays = self.entry["bays"]
        loads = 0
        while True:
            self.arms("carry")
            yield 4.0
            yield from self.drive(self.entry["dig"])
            # Drums down and turning, creeping forward across the patch.
            self.arms("dig", self.entry["drum"]["dig"])
            yield 4.0
            for _ in range(int(8.0 / STEER_S)):
                self.twist(0.12, 0.0)
                yield STEER_S
            self.twist(0.0, 0.0)
            self.arms("carry")
            yield 4.0
            bay = bays[loads % len(bays)]
            yield from self.drive(bay["approach"])
            east, north = bay["stop"][0] - bay["approach"][0], bay["stop"][1] - bay["approach"][1]
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
    start = time.time()

    def log(text):
        print(f"[{time.time() - start:7.1f}s] {text}", flush=True)

    # Tasks wait in the sim's own seconds, however fast or slow it runs.
    def now() -> float:
        clock = patient(gb.clock)
        return clock.world_s / (clock.rate or 1.0)

    tasks = []
    for machine in wanted:
        entry = plan[machine]
        robot = KINDS[entry["kind"]](gb, machine, entry, log)
        log(f"{machine}: {entry['kind']}" + (f", {len(entry['places'])} places in {entry['room']}" if "places" in entry else ""))
        tasks.append((now(), machine, robot.run()))
    heapq.heapify(tasks)
    while tasks:
        wake, machine, task = heapq.heappop(tasks)
        while now() < wake:
            time.sleep(0.02)
        heapq.heappush(tasks, (now() + next(task), machine, task))


if __name__ == "__main__":
    main()
