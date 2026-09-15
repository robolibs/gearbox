#!/usr/bin/env python3
"""Plan a GPS pea field with maptrax, draw it in Gearbox and rerun, drive Oxbos.

Run Gearbox first:

    make run

Then run:

    python scripts/oxbo_maptrax_field.py --machines 4

The script:

* takes a field as WGS84 corners (default: a field north of Wageningen, NL),
  converts it to local metres and plans one headland ring per machine, 6 m swaths and one
  forward-only Dubins tour per machine with maptrax,
* writes boundary, headlands and tours as thin ribbons into a USD layer and
  loads it into Gearbox on top of the pea field,
* opens a rerun viewer with a map (lat/lon) view and a local ENU view of the
  same lines, then streams the live machine positions into both,
* spawns one Oxbo per machine under ``oxbo_0``, ``oxbo_1``, ... and drives
  each one first around its headland ring(s), then along its swaths.

Local metres are Gearbox world x/z; maptrax ENU y is Gearbox z.
"""

from __future__ import annotations

import argparse
import math
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import maptrax
from gearbox_client import Gearbox
from gearbox_client import Machine as GearboxMachine


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_TERRAIN_USD = "world/peafield.usd"
DEFAULT_OXBO_USD = "bin/gearbox/assets/oxbo.usd"
DEFAULT_LINES_USD = REPO_ROOT / "bin/gearbox/assets/world/maptrax_field.usd"

# Field corners as (lat, lon), a plot east of Wageningen (from a GeoJSON
# LineString, closing point dropped). The datum is the field centroid: Gearbox
# world (0, 0) and the maptrax ENU origin.
DATUM_LAT_LON_ALT = (51.9897671, 5.6604848, 10.0)
DEFAULT_FIELD_LATLON: list[tuple[float, float]] = [
    (51.9914638, 5.6570745),
    (51.9876551, 5.6601006),
    (51.9881095, 5.6617422),
    (51.9892174, 5.6608965),
    (51.9897229, 5.6626625),
    (51.9924338, 5.6604323),
]

SWATH_WIDTH_M = 6.0
MACHINE_WIDTH_M = 3.0
MACHINE_LENGTH_M = 9.0
MIN_TURNING_RADIUS_M = 8.0
HEADLAND_RINGS = None  # one ring per machine
DUBINS_STEP_M = 0.75

TICK_DT = 0.10
POSE_TIMEOUT_S = 120.0
LOOKAHEAD_M = 6.0
PASS_TOLERANCE_M = 2.5
LINE_WIDTH_M = 0.10
LINE_HEIGHT_M = 0.05
RERUN_LOG_PERIOD_S = 0.25

MACHINE_COLORS = [
    (0.95, 0.35, 0.25),
    (0.25, 0.55, 0.95),
    (0.95, 0.75, 0.20),
    (0.75, 0.35, 0.85),
    (0.20, 0.85, 0.80),
]
BOUNDARY_COLOR = (0.98, 0.98, 0.95)
HEADLAND_COLOR = (0.55, 0.42, 0.80)

EARTH_RADIUS_M = 6_378_137.0


def wrap_pi(angle: float) -> float:
    return (angle + math.pi) % (2.0 * math.pi) - math.pi


def clamp(value: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, value))


def latlon_to_local(points: list[tuple[float, float]], datum: tuple[float, float, float]) -> list[tuple[float, float]]:
    lat0, lon0 = math.radians(datum[0]), math.radians(datum[1])
    out = []
    for lat, lon in points:
        x = (math.radians(lon) - lon0) * math.cos(lat0) * EARTH_RADIUS_M
        y = (math.radians(lat) - lat0) * EARTH_RADIUS_M
        out.append((x, y))
    return out


# ── maptrax planning ─────────────────────────────────────────────────────

@dataclass
class FieldPlan:
    planner: maptrax.Maptrax
    boundary: list[tuple[float, float]]
    headlands: list[list[tuple[float, float]]]
    tours: list[list[tuple[float, float]]]

    def to_latlon(self, points: list[tuple[float, float]]) -> list[list[float]]:
        return [[lat, lon] for lat, lon in self.planner.enu_to_wgs_batch(points)]


def plan_field(polygon: list[tuple[float, float]], machines: int, swath_angle_deg: float, swath_width: float) -> FieldPlan:
    planner = maptrax.Maptrax()
    planner.set_field(polygon, DATUM_LAT_LON_ALT)
    planner.generate_field(swath_width, swath_angle_deg, HEADLAND_RINGS or machines)
    part = planner.get_part(0)
    plan = planner.plan_machines(
        part_index=0,
        machines=machines,
        pattern="block",
        balance="length",
        headland_mode="one_per_machine",
        routing_strategy="turn_radius_aware",
        local_improvement_passes=1,
        turn_model="dubins",
        min_turning_radius=MIN_TURNING_RADIUS_M,
        step_size=DUBINS_STEP_M,
        machine_length=MACHINE_LENGTH_M,
        machine_width=MACHINE_WIDTH_M,
        swath_width=swath_width,
    )
    count = max(1, len(plan["machines"]))
    tours = [
        stitch_tour(machine["tour"], machine["machine_index"] / count)
        for machine in plan["machines"]
    ]
    headlands = [[(float(x), float(y)) for x, y in ring["points"]] for ring in part["headlands"]]
    print(f"maptrax: area {planner.total_area():.0f} m², {len(part['swaths'])} swaths, "
          f"{len(headlands)} headland rings, {len(tours)} machine tours")
    for i, tour in enumerate(tours):
        print(f"  machine {i}: {len(tour)} path points, {polyline_length(tour):.0f} m")
    return FieldPlan(planner=planner, boundary=list(polygon), headlands=headlands, tours=tours)


def segment_points(segment: dict) -> list[tuple[float, float]]:
    return dedupe([(float(x), float(y)) for x, y in segment["points"]])


def rotate_ring(points: list[tuple[float, float]], fraction: float) -> list[tuple[float, float]]:
    """Start a closed ring a given fraction of the way round, so machines on
    neighbouring rings begin at different corners."""
    ring = points[:-1] if len(points) > 1 and points[0] == points[-1] else list(points)
    shift = int(round(fraction * len(ring))) % len(ring)
    ring = ring[shift:] + ring[:shift]
    return ring + ring[:1]


def heading_between(a: tuple[float, float], b: tuple[float, float]) -> float:
    return math.atan2(b[1] - a[1], b[0] - a[0])


def stitch_tour(tour: list[dict], ring_start_fraction: float) -> list[tuple[float, float]]:
    """Join the tour segments (headland rings first, then swaths) with
    forward-only Dubins turns into one polyline."""
    turner = maptrax.Dubins(MIN_TURNING_RADIUS_M)
    ordered = [s for s in tour if s["type"] == "headland"] + [s for s in tour if s["type"] != "headland"]
    path: list[tuple[float, float]] = []
    end_pose: tuple[float, float, float] | None = None
    for segment in ordered:
        points = segment_points(segment)
        if segment["type"] == "headland":
            points = rotate_ring(points, ring_start_fraction)
        if len(points) < 2:
            continue
        start_pose = (*points[0], heading_between(points[0], points[1]))
        if end_pose is not None and math.dist(end_pose[:2], points[0]) > 0.5:
            turn = turner.plan(end_pose, start_pose, DUBINS_STEP_M)
            path += [(float(x), float(y)) for x, y, _yaw in turn.waypoints]
        path += points
        end_pose = (*points[-1], heading_between(points[-2], points[-1]))
    return dedupe(path)


def dedupe(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    out: list[tuple[float, float]] = []
    for p in points:
        if not out or math.dist(out[-1], p) > 1e-6:
            out.append(p)
    return out


def polyline_length(points: list[tuple[float, float]]) -> float:
    return sum(math.dist(a, b) for a, b in zip(points, points[1:]))


# ── USD ribbon layer ─────────────────────────────────────────────────────

def ribbon_mesh(points: list[tuple[float, float]], width: float, height: float) -> tuple[list, list, list]:
    """Flat quads along the polyline; USD is Z-up with y = -world z."""
    half = width * 0.5
    verts: list[tuple[float, float, float]] = []
    counts: list[int] = []
    indices: list[int] = []
    for (ax, az), (bx, bz) in zip(points, points[1:]):
        dx, dz = bx - ax, bz - az
        n = math.hypot(dx, dz)
        if n < 1e-6:
            continue
        nx, nz = -dz / n * half, dx / n * half
        base = len(verts)
        verts += [
            (ax + nx, -(az + nz), height),
            (ax - nx, -(az - nz), height),
            (bx - nx, -(bz - nz), height),
            (bx + nx, -(bz + nz), height),
        ]
        counts.append(4)
        indices += [base, base + 1, base + 2, base + 3]
    return verts, counts, indices


def usd_mesh_prim(name: str, points: list[tuple[float, float]], color: tuple[float, float, float]) -> str:
    verts, counts, indices = ribbon_mesh(points, LINE_WIDTH_M, LINE_HEIGHT_M)
    if not counts:
        return ""
    pts = ", ".join(f"({x:.3f}, {y:.3f}, {z:.3f})" for x, y, z in verts)
    return f"""
    def Mesh "{name}"
    {{
        uniform token subdivisionScheme = "none"
        bool doubleSided = 1
        int[] faceVertexCounts = [{", ".join(str(c) for c in counts)}]
        int[] faceVertexIndices = [{", ".join(str(i) for i in indices)}]
        point3f[] points = [{pts}]
        color3f[] primvars:displayColor = [({color[0]:.3f}, {color[1]:.3f}, {color[2]:.3f})]
    }}
"""


def write_lines_usd(plan: FieldPlan, path: Path) -> None:
    body = usd_mesh_prim("boundary", plan.boundary + plan.boundary[:1], BOUNDARY_COLOR)
    for i, ring in enumerate(plan.headlands):
        body += usd_mesh_prim(f"headland_{i}", ring + ring[:1], HEADLAND_COLOR)
    for i, tour in enumerate(plan.tours):
        body += usd_mesh_prim(f"tour_{i}", tour, MACHINE_COLORS[i % len(MACHINE_COLORS)])
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "#usda 1.0\n(\n    defaultPrim = \"MaptraxField\"\n    upAxis = \"Z\"\n    metersPerUnit = 1\n)\n\n"
        "# Generated by scripts/oxbo_maptrax_field.py. Field boundary, headland\n"
        "# rings and per-machine tours as flat ribbons just above the ground.\n\n"
        "def Xform \"MaptraxField\"\n{\n" + body + "}\n"
    )
    print(f"wrote {path}")


# ── rerun ────────────────────────────────────────────────────────────────

class RerunView:
    """Map (lat/lon) and local ENU views of the plan plus live machines."""

    def __init__(self, plan: FieldPlan, mode: str, out: Path | None):
        self.plan = plan
        self.enabled = mode != "off"
        if not self.enabled:
            return
        import rerun as rr

        self.rr = rr
        rr.init("gearbox_maptrax_field", spawn=(mode == "spawn"))
        if mode == "save" and out is not None:
            rr.save(str(out))
        self.log_plan()

    def rgb(self, color: tuple[float, float, float]) -> list[int]:
        return [int(c * 255) for c in color]

    def log_lines(self, path: str, lines: list[list[tuple[float, float]]], color, width_pt: float) -> None:
        rr = self.rr
        rr.log(f"enu/{path}", rr.LineStrips2D([[list(p) for p in line] for line in lines],
                                              colors=[self.rgb(color)], radii=rr.Radius.ui_points(width_pt)),
               static=True)
        rr.log(f"map/{path}", rr.GeoLineStrings(lat_lon=[self.plan.to_latlon(line) for line in lines],
                                                colors=[self.rgb(color)], radii=rr.Radius.ui_points(width_pt)),
               static=True)

    def log_plan(self) -> None:
        plan = self.plan
        self.log_lines("field/border", [plan.boundary + plan.boundary[:1]], BOUNDARY_COLOR, 2.0)
        self.log_lines("field/headlands", [ring + ring[:1] for ring in plan.headlands], HEADLAND_COLOR, 1.2)
        for i, tour in enumerate(plan.tours):
            self.log_lines(f"machines/m{i}/tour", [tour], MACHINE_COLORS[i % len(MACHINE_COLORS)], 1.5)

    def log_machine(self, idx: int, x: float, z: float, trail: list[tuple[float, float]]) -> None:
        if not self.enabled:
            return
        rr = self.rr
        color = self.rgb(MACHINE_COLORS[idx % len(MACHINE_COLORS)])
        rr.set_time("wall", timestamp=time.time())
        rr.log(f"enu/machines/m{idx}/position", rr.Points2D([[x, z]], colors=[color], radii=rr.Radius.ui_points(6.0)))
        rr.log(f"map/machines/m{idx}/position", rr.GeoPoints(lat_lon=self.plan.to_latlon([(x, z)]),
                                                             colors=[color], radii=rr.Radius.ui_points(6.0)))
        if len(trail) >= 2:
            rr.log(f"enu/machines/m{idx}/trail", rr.LineStrips2D([[list(p) for p in trail]], colors=[color],
                                                                  radii=rr.Radius.ui_points(1.0)))
            rr.log(f"map/machines/m{idx}/trail", rr.GeoLineStrings(lat_lon=[self.plan.to_latlon(trail)],
                                                                    colors=[color], radii=rr.Radius.ui_points(1.0)))


# ── Gearbox side ─────────────────────────────────────────────────────────

@dataclass
class MachinePose:
    x: float = 0.0
    z: float = 0.0
    heading_rad: float = 0.0
    seen: bool = False


@dataclass
class Machine:
    idx: int
    path: list[tuple[float, float]]
    namespace: str
    oxbo_usd: str
    progress: int = 0
    done: bool = False
    trail: list[tuple[float, float]] = field(default_factory=list)
    pose: MachinePose = field(default_factory=MachinePose)
    machine: GearboxMachine | None = None

    def attach(self, gb: Gearbox, timeout_s: float) -> bool:
        try:
            self.machine = gb.machine(self.namespace, timeout=timeout_s)
        except RuntimeError:
            return False
        self.machine.claim(hold_ms=1000, take=True, client="oxbo_maptrax_field.py")
        return self.machine.state(wait=30.0) is not None

    def snapshot(self) -> MachinePose:
        if self.machine is not None:
            s = self.machine.state()
            if s is not None:
                self.pose = MachinePose(s.x, s.z, s.heading_rad, True)
        return MachinePose(self.pose.x, self.pose.z, self.pose.heading_rad, self.pose.seen)

    def load(self, gb: Gearbox) -> None:
        (x0, z0), (x1, z1) = self.path[0], self.path[min(1, len(self.path) - 1)]
        yaw_deg = math.degrees(math.atan2(x1 - x0, z1 - z0))
        gb.load_machine(self.namespace, self.oxbo_usd, x=x0, z=z0, yaw_deg=yaw_deg, label=f"{self.namespace}.usd")

    def publish_cmd(self, speed: float, yaw_rate: float) -> None:
        if self.machine is not None:
            self.machine.cmd_vel(speed, yaw_rate)

    def release(self) -> None:
        if self.machine is not None:
            self.machine.stop()
            self.machine.release()

    def step(self) -> None:
        pose = self.snapshot()
        if not pose.seen or self.done:
            self.publish_cmd(0.0, 0.0)
            return
        here = (pose.x, pose.z)
        if not self.trail or math.dist(self.trail[-1], here) > 0.5:
            self.trail.append(here)
        last = len(self.path) - 1
        while self.progress < last and math.dist(here, self.path[self.progress]) < PASS_TOLERANCE_M:
            self.progress += 1
        if self.progress >= last and math.dist(here, self.path[last]) < PASS_TOLERANCE_M:
            self.done = True
            self.publish_cmd(0.0, 0.0)
            print(f"{self.namespace}: tour finished")
            return
        target_idx = self.progress
        while target_idx < last and math.dist(here, self.path[target_idx]) < LOOKAHEAD_M:
            target_idx += 1
        tx, tz = self.path[target_idx]
        heading_err = wrap_pi(math.atan2(tx - pose.x, tz - pose.z) - pose.heading_rad)
        abs_err = abs(heading_err)
        speed = 2.2
        if abs_err > math.radians(60.0):
            speed = 1.0
        elif abs_err > math.radians(20.0):
            speed = 1.4
        yaw_rate = clamp(1.3 * heading_err, -0.85, 0.85)
        self.publish_cmd(speed, yaw_rate)


def drive(machines: list[Machine], view: RerunView) -> None:
    print("driving tours, Ctrl-C to stop.")
    next_log = 0.0
    try:
        while not all(m.done for m in machines):
            for m in machines:
                m.step()
            if time.time() >= next_log:
                for m in machines:
                    pose = m.snapshot()
                    if pose.seen:
                        view.log_machine(m.idx, pose.x, pose.z, m.trail)
                next_log = time.time() + RERUN_LOG_PERIOD_S
            time.sleep(TICK_DT)
        print("all tours finished")
    except KeyboardInterrupt:
        pass
    finally:
        for m in machines:
            m.release()
        print("stopped all machines")


def parse_pairs(text: str, what: str) -> list[tuple[float, float]]:
    vals = [float(v) for v in text.replace(",", " ").split()]
    if len(vals) < 6 or len(vals) % 2 != 0:
        raise SystemExit(f"{what} needs at least three pairs: {text!r}")
    return list(zip(vals[0::2], vals[1::2], strict=True))


def load_geojson_field(path: Path) -> list[tuple[float, float]]:
    """First LineString/Polygon of a GeoJSON file as (lat, lon) corners."""
    import json

    data = json.loads(path.read_text())
    features = data.get("features", [data])
    for feature in features:
        geometry = feature.get("geometry", feature)
        coords = geometry.get("coordinates", [])
        if geometry.get("type") == "Polygon":
            coords = coords[0]
        elif geometry.get("type") != "LineString":
            continue
        ring = [(float(lat), float(lon)) for lon, lat in coords]
        if len(ring) > 1 and math.dist(ring[0], ring[-1]) < 1e-6:
            ring.pop()
        if len(ring) >= 3:
            return ring
    raise SystemExit(f"no LineString or Polygon with 3+ points in {path}")


def centroid(points: list[tuple[float, float]]) -> tuple[float, float]:
    return (sum(p[0] for p in points) / len(points), sum(p[1] for p in points) / len(points))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--machines", type=int, default=4, help="number of Oxbos (default 4)")
    parser.add_argument("--field-latlon", metavar='"lat lon lat lon ..."',
                        help="field corners as WGS84 pairs (default: field east of Wageningen)")
    parser.add_argument("--geojson", type=Path, help="field from a GeoJSON LineString or Polygon file")
    parser.add_argument("--field", metavar='"x z x z ..."', help="field corners in local world x z metres")
    parser.add_argument("--angle", type=float, default=90.0, help="swath angle in degrees (default 90)")
    parser.add_argument("--swath", type=float, default=SWATH_WIDTH_M, help=f"line spacing in metres (default {SWATH_WIDTH_M})")
    parser.add_argument("--rerun", choices=["spawn", "save", "off"], default="spawn",
                        help="open the rerun viewer, save an .rrd, or skip rerun (default spawn)")
    parser.add_argument("--rerun-out", type=Path, default=Path("/tmp/gearbox_maptrax_field.rrd"))
    parser.add_argument("--terrain-usd", default=DEFAULT_TERRAIN_USD)
    parser.add_argument("--oxbo-usd", default=DEFAULT_OXBO_USD)
    parser.add_argument("--lines-usd", type=Path, default=DEFAULT_LINES_USD,
                        help="where the generated ribbon layer is written")
    parser.add_argument("--lines-load-path", default=None,
                        help="path Gearbox loads the ribbon layer from (default: --lines-usd)")
    parser.add_argument("--prefix", default="oxbo", help="machine namespace prefix (default oxbo)")
    parser.add_argument("--no-clear", action="store_true", help="do not clear the sim first")
    parser.add_argument("--plan-only", action="store_true", help="plan, write the USD and log to rerun, do not drive")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    global DATUM_LAT_LON_ALT
    if args.field:
        polygon = parse_pairs(args.field, "field")
    else:
        if args.geojson:
            latlon = load_geojson_field(args.geojson)
        elif args.field_latlon:
            latlon = parse_pairs(args.field_latlon, "field-latlon")
        else:
            latlon = DEFAULT_FIELD_LATLON
        if latlon is not DEFAULT_FIELD_LATLON:
            DATUM_LAT_LON_ALT = (*centroid(latlon), DATUM_LAT_LON_ALT[2])
        polygon = latlon_to_local(latlon, DATUM_LAT_LON_ALT)
    plan = plan_field(polygon, args.machines, args.angle, args.swath)
    write_lines_usd(plan, args.lines_usd)
    view = RerunView(plan, args.rerun, args.rerun_out)
    if args.plan_only:
        return

    machines = [
        Machine(i, tour, f"{args.prefix}_{i}", args.oxbo_usd)
        for i, tour in enumerate(plan.tours)
        if len(tour) >= 2
    ]
    gb = Gearbox()
    gb.wait_ready()
    if not args.no_clear:
        gb.clear()
        time.sleep(0.3)
    gb.load("flatland_terrain", args.terrain_usd, category="terrain")
    gb.load("maptrax_field_lines", args.lines_load_path or str(args.lines_usd), category="world")
    for m in machines:
        print(f"spawning {m.namespace} at x={m.path[0][0]:.1f}, z={m.path[0][1]:.1f}")
        m.load(gb)

    missing = [m for m in machines if not m.attach(gb, POSE_TIMEOUT_S)]
    if missing:
        names = ", ".join(m.namespace for m in missing)
        raise SystemExit(f"no state from machine namespaces: {names}. Is Gearbox running?")
    drive(machines, view)


if __name__ == "__main__":
    main()
