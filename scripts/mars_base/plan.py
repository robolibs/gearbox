"""The farm's work plan: for every robot that tends plants, where each of its
joints must stand to bring its head over each plant place it reaches, and
the order its joints move in. A joint's target is a position 0..1, which
its `builtin:joint_position` controller drives to position × the moved
body's `gearbox:value:range` (negative for a joint that works toward its
lower limit) from the joint's zero. The build
writes the plan next to the fleet; `examples/python/mars_farm.py` works the
farm by it.

Kinematics of each kind of robot, given its sizes and where the fleet
places it, against each place in the base frame (X north, Y west, Z up);
a harvester's arm is solved with `arms.solve`, inside Blender.
"""
import json
import math
from dataclasses import dataclass, field

# The head stands this far over a tray's surface to sow or harvest: within
# 0.3 m of the middle of every crop from a seed to a ripe potato.
OVER_TRAY = 0.15


@dataclass
class Joint:
    name: str
    # The body it moves: a joint is set through its link's `position` value.
    link: str
    # The moved body's `gearbox:value:range`: the joint's value at position 1.
    range: float
    # Fastest motion, metres or radians a second.
    speed: float

    def at(self, value, slack=1e-6):
        """The position 0..1 of `value`, or None outside the range."""
        f = value / self.range
        if not -slack <= f <= 1.0 + slack:
            return None
        return min(max(f, 0.0), 1.0)


@dataclass
class Robot:
    machine: str
    kind: str
    room: str
    joints: list
    # Joint groups moved one after the other to go from place to place: the
    # first leaves the last place, the second crosses, the third reaches in.
    leave: list
    cross: list
    reach: list
    park: dict
    places: dict = field(default_factory=dict)
    # The crop of each tray it reaches.
    trays: dict = field(default_factory=dict)
    # Anything else its kind needs, like a harvester's lane heading.
    extra: dict = field(default_factory=dict)

    def entry(self):
        return {
            **self.extra,
            "kind": self.kind,
            "room": self.room,
            "joints": {j.name: {"link": j.link, "range": j.range, "speed": j.speed} for j in self.joints},
            "moves": {"leave": self.leave, "cross": self.cross, "reach": self.reach},
            "park": self.park,
            "places": self.places,
            "trays": self.trays,
        }


def into(at, yaw, p):
    """A base-frame point in the frame of something standing at `at` turned `yaw`."""
    dx, dy = p[0] - at[0], p[1] - at[1]
    c, s = math.cos(yaw), math.sin(yaw)
    return (c * dx + s * dy, -s * dx + c * dy, p[2] - at[2])


def out_of(at, yaw, p):
    """A point in the frame of something standing at `at` turned `yaw`, in the base frame."""
    c, s = math.cos(yaw), math.sin(yaw)
    return (at[0] + c * p[0] - s * p[1], at[1] + s * p[0] + c * p[1], at[2] + p[2])


def slots(tray):
    """Each place of a tray, `(name, base-frame point OVER_TRAY above its middle)`.
    A tray is a dict: name, at, yaw (in its building), size, slots (cols, rows),
    and its building's `origin` and `turn` in the base."""
    cols, rows = tray["slots"]
    sx, sy = tray["size"]
    c, s = math.cos(tray["yaw"]), math.sin(tray["yaw"])
    for j in range(rows):
        for i in range(cols):
            x = ((i + 0.5) / cols - 0.5) * sx
            y = ((j + 0.5) / rows - 0.5) * sy
            local = (tray["at"][0] + c * x - s * y, tray["at"][1] + s * x + c * y, tray["at"][2] + OVER_TRAY)
            yield f"{tray['name']}/{j * cols + i}", out_of(tray["origin"], tray["turn"], local)


def rail_gantry(r, machine, room, at, yaw):
    """A rail gantry (`robots.Rail`) standing at `at` turned `yaw`; its head
    is the arm's head block. The arm turns to the side a place is on and
    telescopes out to it."""
    travel = Joint("travel", "trolley", r.rail_len - 0.4, 0.3 * r.scale)
    lift = Joint("lift", "mast", -r.lift, 0.25 * r.scale)
    turn = Joint("turn", "turret", math.pi, 1.0)
    reach = Joint("reach", "arm", -r.reach, 0.2 * r.scale)
    robot = Robot(machine, "rail_gantry", room, [travel, lift, turn, reach], ["reach", "lift"], ["travel", "turn"], ["lift", "reach"],
                  {"travel": 0.0, "lift": 0.0, "turn": 0.0, "reach": 0.0})
    near = -(r.arm[1] + r.head)

    def solve(p):
        x, y, z = into(at, yaw, p)
        side = 0.0 if y <= 0 else math.pi
        targets = {
            "travel": travel.at(x - 0.2),
            "turn": turn.at(side),
            "reach": reach.at(near - abs(y)),
            "lift": lift.at(z - r.arm[2] + 0.01),
        }
        return None if None in targets.values() else targets
    return robot, solve


def polar_gantry(p, machine, room, at, yaw):
    """A dome's polar gantry (`robots.Polar`) standing at the dome's centre;
    its head is the mast's head block."""
    slew = Joint("slew", "boom", 2 * math.pi, 0.15)
    travel = Joint("travel", "carriage", p.travel, 0.5)
    lift = Joint("lift", "mast", -p.lift, 0.3)
    robot = Robot(machine, "polar_gantry", room, [slew, travel, lift], ["lift"], ["slew", "travel"], ["lift"],
                  {"slew": 0.0, "travel": 0.0, "lift": 0.0})

    def solve(q):
        x, y, z = into(at, yaw, q)
        angle = math.atan2(y, x) % (2 * math.pi)
        targets = {
            "slew": slew.at(angle),
            "travel": travel.at(math.hypot(x, y) - p.inner),
            "lift": lift.at(z - p.head_z + 0.06),
        }
        return None if None in targets.values() else targets
    return robot, solve


def stacker_crane(c, machine, room, at, yaw, nursery_x, floor):
    """The vertical farm's stacker crane (`crane.Crane`), its machine frame at
    `at` on the hall's floor: the arm turns to a rack and telescopes over a
    tray, its head as high over the tray as the tray's lamp lets it. Trays
    short of `nursery_x` along the hall are the nursery's, under low lamps."""
    travel = Joint("travel", "crane", c.travel, 1.5)
    lift = Joint("lift", "carriage", c.lift, 0.8)
    turn = Joint("turn", "turret", math.pi, 1.0)
    # One controller drives both stages of the telescope, each half the stroke.
    reach = Joint("reach", "stage", c.reach, 0.4)
    robot = Robot(machine, "stacker_crane", room, [travel, lift, turn, reach], ["reach"], ["travel", "lift", "turn"], ["reach"],
                  {"travel": 0.0, "lift": 0.0, "turn": 0.0, "reach": 0.0})

    def solve(p):
        x, y, z = into(at, yaw, p)
        over = 0.05 if x < nursery_x else 0.12
        targets = {
            "travel": travel.at(x),
            "turn": turn.at(0.0 if y > 0 else math.pi),
            "reach": reach.at(abs(y) - 0.44),
            "lift": lift.at(max(z - OVER_TRAY + floor + over - 0.6525, 0.0)),
        }
        return None if None in targets.values() else targets
    return robot, solve


def mobile_harvester(h, machine, room, origin, turn, side, stops, lane=1.35):
    """A harvester (`harvester.Harvester`) working one row of an arch's tables
    from the aisle: it drives along its lane to the stop opposite a place's
    table and its arm brings the tool down over the place, tilted toward the
    aisle as little as it can be. `side` is the row's: +1 the +Y row of the
    arch, -1 the other; `stops` the tables' middles along the arch.
    A place's `drive` is where the robot stands, east and north as its
    odometry reads them."""
    import arms
    import harvester as hv
    from mathutils import Matrix, Vector

    names = [j.name for j in hv.ARM.joints]
    links = [link.name for link in hv.ARM.links]
    spans = [hv.ARM_RANGES[n] for n in names]
    joints = [Joint(n, link, math.radians(hi - lo), math.radians(60.0)) for n, link, (lo, hi) in zip(names, links, spans)]
    stow = {n: (q - lo) / (hi - lo) for n, q, (lo, hi) in zip(names, hv.ARM_REST, spans)}
    robot = Robot(machine, "mobile_harvester", room, joints, names, ["drive"], names, dict(stow))
    local, carriage = hv.mount(h)
    heading = math.radians(90.0 * side)
    # It drives toward its -Y, so its odometry heading is its yaw in the base, plus the arch's.
    robot.extra = {"heading": heading + turn}

    def solve(p):
        x, y, z = into(origin, turn, p)
        if y * side <= 0:
            return None
        stop = min(stops, key=lambda s: abs(s - x))
        place = Matrix.Translation((stop, side * lane, 0.12)) @ Matrix.Rotation(heading, 4, "Z")
        base = place @ Matrix.Translation(carriage) @ local
        aim = Vector((x, y, z))
        for tilt in (0, 10, 20, 30, 40, 50):
            m = Matrix.Rotation(heading - math.pi / 2, 4, "Z") @ Matrix.Rotation(math.radians(tilt), 4, "X") @ Matrix.Rotation(math.pi, 4, "X")
            m.translation = aim
            q, e, ang = arms.solve(hv.ARM, base, m, [0, -40, 60, -110, -90, 0], tip=hv.tcp(h), limits=hv.ARM_RANGES, rest=hv.ARM_REST)
            if e < 0.005 and ang < 0.5:
                break
        else:
            return None
        bx, by, _ = out_of(origin, turn, (stop, side * lane, 0.0))
        targets = {n: (v - lo) / (hi - lo) for n, v, (lo, hi) in zip(names, q, spans)}
        targets["drive"] = [round(-by, 4), round(bx, 4)]
        return targets
    return robot, solve


def enu(p):
    """A base-frame point as odometry reads it: east and north."""
    return [round(-p[1], 3), round(p[0], 3)]


def excavator(machine, dock, dig, bays, approach=3.0):
    """A regolith excavator's round: from its dock out to its patch of the dig
    ground, back to a tipping bay of the soil plant, driven into it from
    `approach` metres out along the bay's -Y, and home. Points are base-frame;
    the entry carries them as odometry reads them."""
    return {
        "kind": "excavator",
        "dock": enu(dock),
        "dig": enu(dig),
        "bays": [{"approach": enu((x, y + approach, 0.0)), "stop": enu((x, y, 0.0))} for x, y in bays],
        # Arm positions and drum speeds (rad/s): the front drum digs turning
        # forward and empties turning back over the bay's lip. The drums
        # meet the ground at 0.787; further down they lift the wheels off it.
        "arm": {"carry": 0.40, "dig": 0.78, "dump": 0.33, "stow": 0.0},
        "drum": {"dig": 2.1, "dump": -2.1},
        "links": {"arm_front": "arm_front", "arm_rear": "arm_rear", "drum_front": "drum_front", "drum_rear": "drum_rear"},
    }


def arch_washer(g, press, room):
    """An arch greenhouse's roof washer (`washers.arch`): bay by bay its
    portal stops over the bay's middle and the brush sweeps the vault from
    curb to curb, pressed in as `press(g, angle)` says; from the dock at
    travel 0. Then it brushes the room's glazing clean."""
    sweep = []
    for k in range(9):
        p = k / 8
        sweep.append([round(p, 4), round(press(g, g.hi - (g.hi - g.lo) * p) / g.stroke, 4)])
    return {
        "kind": "arch_washer",
        "room": room,
        "links": {"travel": "portal", "sweep": "carriage", "press": "head", "brush": "brush"},
        "bays": [round((g.span - g.bay * j) / g.span, 4) for j in range(g.bays)],
        "sweep": sweep,
        # Seconds over a whole range, from each joint's top speed.
        "full_s": {"travel": g.span / 0.3, "sweep": (g.hi - g.lo) / 0.15, "press": g.stroke / 0.1},
        "brush": 20.0,
    }


def dome_washer(room, bands=(0.1, 0.3, 0.5, 0.7, 0.9), glazed_from=0.7):
    """A dome's glazing washer: band by band down the dome, its arch slews
    once round with the brush turning; then, with the head low enough to be
    at the room, it brushes the room's glazing clean."""
    return {
        "kind": "dome_washer",
        "room": room,
        "links": {"slew": "arch", "meridian": "carriage", "brush": "brush"},
        "bands": list(bands),
        "glazed_from": glazed_from,
        "full_s": {"slew": 2 * math.pi / 0.05, "meridian": math.radians(72.5) / 0.04},
        "brush": 20.0,
    }


def panel_washer(length):
    """A solar row's washer: out along its row and back with the brush
    turning, then home in its dock; the field is clean once every row is."""
    return {
        "kind": "panel_washer",
        "links": {"travel": "frame", "brush": "brush"},
        "full_s": {"travel": length / 0.4},
        "brush": 15.0,
    }


def plan(robots, trays, others=()):
    """Fills each robot's places with every tray place of its room it reaches;
    `robots` are `(Robot, solve)` pairs."""
    for robot, solve in robots:
        for tray in trays:
            if tray["room"] != robot.room:
                continue
            for name, p in slots(tray):
                targets = solve(p)
                if targets is not None:
                    robot.places[name] = {k: v if isinstance(v, list) else round(v, 4) for k, v in targets.items()}
                    robot.places[name]["at"] = enu(p)
                    robot.trays[tray["name"]] = tray["species"]
    machines = {robot.machine: robot.entry() for robot, _ in robots}
    machines.update(others)
    return {"machines": machines}


def write(path, robots, trays, others=()):
    """Writes the plan, the tending robots' and `others`' entries (machine
    id: entry); returns how many places each tending robot reaches."""
    data = plan(robots, trays, dict(others))
    with open(path, "w") as f:
        json.dump(data, f, indent=1)
        f.write("\n")
    return {name: len(entry.get("places", ())) for name, entry in data["machines"].items()}
