#!/usr/bin/env python3
"""A filmed tour of the Mars base in one take: the camera flies one smooth
path from robot to robot, into the greenhouses and out again, never stopping
on the way, recorded with a blending shutter as one video, then captioned
and cut down to a preview.

Run it against a sim with the base, its fleet and the farm runner working,
headless or not:

    gearbox run --headless 1920x1080 --planet mars --world bin/gearbox/assets/world/mars_base.usda
    gearbox spawn machine bin/gearbox/assets/world/mars_base_fleet.usda --machine base
    python3 scripts/control/mars_farm.py --digging base_fleet_digger3 \\
        --sway base_fleet_archbbot:trolley=0:0.12,mast=0:0.8,arm=0:0.5 \\
        --sway base_fleet_domebbot:carriage=0.1:0.9,mast=0:0.6 \\
        --sway base_fleet_domebwasher:arch=0.5:0.7,carriage=0.1:0.9
    python3 scripts/control/mars_tour.py --out .work/movie

The robots swayed are the ones whose reach would cross the camera's way
through arch B and dome B: each works the part of its room the camera
keeps out of. While it records, the sim's clock steps a sixtieth of a
second a frame of video, so the path's seconds below are seconds of video
however fast the GPU renders.
"""
from __future__ import annotations

import argparse
import json
import math
import re
import subprocess
import time
from pathlib import Path

from gearbox_client import Gearbox

ROOT = Path(__file__).resolve().parents[2]
GB = str(ROOT / "target" / "release" / "gearbox")
# The base frame's origin on Mars, and metres a degree of latitude spans.
LAT0, LON0, M_PER_DEG = 18.4447, 77.4508, 59158.0
# Frames rendered and blended into each frame of video.
SHUTTER = 2
CAPTION_S = 4.5
FONT = subprocess.run(["fc-match", "-f", "%{file}", "DejaVu Sans"], capture_output=True, text=True).stdout.strip()
# Arch B's axis, its north gable and the height of the gable pane's middle.
ARCH_B_Y, GABLE_X, PANE_Z = -77.0, 44.0, 3.95
# Dome B's middle, and the glass facets the camera flies in and out by:
# each facet's middle (x, y, z), compass azimuth out and tilt up, degrees.
DOME_B = (62.0, 20.0)
FACET_IN, FACET_OUT = (51.12, 15.53, 5.03, 157.7, 10.3), (51.66, 25.44, 5.45, 207.8, 12.3)
# A picker's lane, off the arch's axis, and how near the parked ridge robot it may be held.
PICKERS = {"archbpicker1": 1.35, "archbpicker2": -1.35}
HOLD_FROM_X = 25.0


def place(x: float, y: float, above: float, bearing: float, pitch: float, distance: float) -> str:
    """A view of the base-frame point (x north, y west) `above` m over the ground."""
    lat = LAT0 + x / M_PER_DEG
    lon = LON0 - y / (M_PER_DEG * math.cos(math.radians(LAT0)))
    return (f"look lat={lat:.7f} lon={lon:.7f} above={above:.2f} bearing={bearing % 360:.1f} "
            f"pitch={pitch} distance={distance}")


def seen_from(x: float, y: float, z: float, bearing: float, pitch: float, distance: float) -> str:
    """A view whose eye stands at (x, y) z m up, its target `distance` away
    the other way from `bearing`, `pitch` below it."""
    b, p = math.radians(bearing), math.radians(pitch)
    flat = distance * math.cos(p)
    return place(x - flat * math.cos(b), y + flat * math.sin(b), z - distance * math.sin(p), bearing, pitch, distance)


def machine(name: str, bearing: float, pitch: float, distance: float, link: str | None = None, above: float = 0.0) -> str:
    """A view round a fleet machine (or one of its links), following it as
    it moves, its bearing a compass bearing."""
    tail = f" link={link}" if link else ""
    return f"look base_fleet_{name}{tail} above={above} bearing={bearing % 360:.1f} pitch={pitch} distance={distance} compass=1"


def pickers() -> dict[str, float]:
    """How far north each of arch B's pickers stands."""
    out = {}
    for name in PICKERS:
        state = subprocess.run([GB, "--timeout", "20", "machine", "state", f"base_fleet_{name}"], capture_output=True, text=True).stdout
        pos = re.search(r"pos \(([-+0-9.]+), ([-+0-9.]+)", state)
        if pos:
            out[name] = float(pos.group(2))
    return out


def tour(north: dict[str, float]) -> list[dict]:
    """The take as the views the path passes, each at its second, the
    caption shown from there, and the machines whose TF tree is drawn from
    there (none: off). Inside arch B the eye keeps to the walkway's middle,
    passes the pickers there, turns round in place past the one held and
    leaves by the pane it came in by; through dome B it flies a straight
    chord from one glass facet to another, clear of the cistern."""
    keys = []
    t = 0.0

    def key(dt, view, caption=None, tf=None):
        nonlocal t
        t += dt
        keys.append({"at": round(t, 2), "view": view, "caption": caption, "tf": tf})

    key(0, place(5, -20, 0, 230, 34, 270))
    key(8, place(5, -30, 0, 295, 26, 190), "A Mars base for a crew of twelve, Jezero crater")
    # Down to arch B's north gable and in through its pane, along the walkway.
    key(9, place(40, ARCH_B_Y, 2, 10, 28, 45))
    key(7, seen_from(GABLE_X + 10, ARCH_B_Y, PANE_Z, 0, 10, 6))
    key(5, seen_from(GABLE_X, ARCH_B_Y, PANE_Z, 0, 10, 6))
    key(3, seen_from(GABLE_X - 5.5, ARCH_B_Y, 3.2, 0, 12, 6), "Inside the arches: tomatoes and peppers on table-top gutters",
        ["archbpicker1", "archbpicker2", "archbbot"])
    key(4, seen_from(GABLE_X - 11, ARCH_B_Y, 2.1, 0, 14, 5))
    # Past the pickers to the one furthest along, round in place, and held on it.
    held = max(north, key=north.get) if north else "archbpicker1"
    lane = PICKERS[held]
    at = max(north.get(held, 30.0), HOLD_FROM_X)
    turn_x = at - 2.4
    key(max(4.0, (GABLE_X - 11 - turn_x) / 1.3), seen_from(turn_x, ARCH_B_Y, 1.75, 0, 16, 3))
    face = 150 if lane > 0 else 210
    for k, b in enumerate((45, 100, 150)):
        key(3.6, seen_from(turn_x, ARCH_B_Y, 1.75, b if lane > 0 else -b, 18, 2.8),
            "Robot arms on mobile bases pick the ripe fruit; every joint is tracked" if k == 2 else None)
    key(4, machine(held, face, 20, 2.7, above=0.7))
    key(5, machine(held, face + (15 if lane > 0 else -15), 22, 2.9, above=0.7))
    # Back up the walkway and out by the pane.
    key(5, seen_from(at + 3, ARCH_B_Y, 1.9, 180, 12, 5))
    key(max(4.0, (GABLE_X - 5.5 - at - 3) / 1.6), seen_from(GABLE_X - 5.5, ARCH_B_Y, 3.3, 180, 10, 6))
    key(3, seen_from(GABLE_X, ARCH_B_Y, PANE_Z, 180, 10, 6), None, [])
    key(4, seen_from(GABLE_X + 6, ARCH_B_Y, PANE_Z + 1.5, 165, 14, 7))
    # Up over the arches, round past dome A on the skyline, to their washers,
    # then the dome washer, and on high and wide of dome A, whose washer arch
    # stands round it somewhere.
    key(5, place(62, -20, 6, 100, 8, 58))
    key(5, place(48, -84, 5, 95, 34, 20))
    key(6, machine("archbwasher", 70, 32, 22, "portal", 3), "Washers brush the dust off the arch roofs", ["archbwasher"])
    key(7, machine("archbwasher", 95, 30, 20, "portal", 3))
    key(9, machine("domeawasher", 200, 32, 30, "carriage"), "The dome washer sweeps the geodesic glass", ["domeawasher"])
    key(7, machine("domeawasher", 225, 28, 28, "carriage"))
    key(6, seen_from(46, -4, 19, 140, 32, 16), None, [])
    # Into dome B square through one glass facet, round the cistern's south
    # side and out square through another, looking 3 m ahead.
    def off_facet(facet, s, bearing, pitch):
        """A view whose eye is `s` m out from a facet's middle along its normal."""
        x, y, z, az, tilt = facet
        a, t = math.radians(az), math.radians(tilt)
        return seen_from(x + s * math.cos(t) * math.cos(a), y - s * math.cos(t) * math.sin(a), z + s * math.sin(t), bearing, pitch, 3)

    def round_cistern(r, az, z, bearing, pitch):
        """A view whose eye is `r` m from dome B's middle at compass azimuth `az`."""
        a = math.radians(az)
        return seen_from(DOME_B[0] + r * math.cos(a), DOME_B[1] - r * math.sin(a), z, bearing, pitch, 3)

    inward, outward = FACET_IN[3], FACET_OUT[3] - 180
    key(7, off_facet(FACET_IN, 12, inward, FACET_IN[4]))
    key(6, off_facet(FACET_IN, 1.2, inward, FACET_IN[4]))
    key(2, off_facet(FACET_IN, -1.2, inward, FACET_IN[4]), "Inside a dome: printed regolith terraces round a water cistern", ["domebbot"])
    key(1.5, off_facet(FACET_IN, -2.5, inward, 12))
    for k, (r, az, bearing, pitch) in enumerate(((7.8, 160.5, 140, 16), (6.4, 170, 115, 20), (6.0, 182.75, 90, 22), (6.4, 195.5, 65, 18), (7.8, 205, 45, 10))):
        key(2.1 if k in (0, 4) else 2.4, round_cistern(r, az, 4.6, bearing, pitch))
    key(1.5, off_facet(FACET_OUT, -2.5, outward, 3))
    key(1.2, off_facet(FACET_OUT, -1.2, outward, 3))
    key(2, off_facet(FACET_OUT, 1.2, outward, 3))
    key(3.5, off_facet(FACET_OUT, 5, outward, 3), None, [])
    # Over to the solar field's panel washers, met from the east between two rows.
    key(8, place(30, 58, 3, 100, 34, 36))
    key(8, machine("solarwasher2", 115, 28, 9, "frame"), "Panel washers keep the arrays clean", ["solarwasher2"])
    key(6, machine("solarwasher2", 130, 30, 10, "frame"))
    # The crews' rovers driving their loops beside the hab, then the soil
    # works and its diggers.
    key(8, place(-42, 50, 1, 250, 32, 50), "Pressurised rovers carry the crews out to the field", ["rover1", "rover2"])
    key(6, machine("rover1", 200, 38, 16, "base_link", 1.0))
    key(8, place(-58, 63, 4, 330, 45, 50), None, [])
    key(10, machine("digger3", 340, 22, 3.6, "base_link", 0.3), "RASSOR excavators dig regolith with counter-rotating drums", ["digger3"])
    key(7, machine("digger3", 20, 20, 3.4, "base_link", 0.3))
    key(7, place(-112, 30, 2, 340, 45, 40), "Loads tipped into the soil bays", [])
    key(7, place(-99, 0, 1, 300, 28, 32), "Water from the ice well; oxygen and methane from the air")
    key(6, place(-120, -5, 2, 245, 20, 26))
    # MARSHA, and its printer laying the first courses of a new one.
    key(7, place(-80, -48, 5, 220, 40, 55))
    key(7, place(-39.5, -91.5, 6, 200, 16, 32), "MARSHA, a habitat printed from regolith")
    key(6, seen_from(-30, -75, 16, 160, 30, 20))
    key(6, place(-17, -99, 2, 320, 24, 26), "Its printer has begun the next: basalt fibre and bioplastic, a course at a time",
        ["printer"])
    key(6, machine("printer", 330, 22, 6.5, "head", 0.3))
    key(6, machine("printer", 300, 26, 7, "head", 0.3))
    # Up and away from the printer's boom, high over MARSHA, to the launch
    # site: Ariane 6 on its table, Honda's rocket on the hopper pad.
    key(6, seen_from(-2, -90, 18, 210, 35, 24), None, [])
    key(8, place(-120, -120, 12, 240, 30, 80))
    key(8, place(-220, -160, 28, 230, 12, 98), "Ariane 6 on its launch table")
    key(6, place(-220, -160, 26, 270, 10, 88))
    key(8, place(-150, -232, 19, 250, 12, 85), "Honda's reusable rocket, scaled up as the base's hopper")
    key(6, place(-150, -232, 16, 300, 10, 72))
    # Out over the crater floor to Perseverance, far from the base, and in
    # on it as it takes a sample.
    key(6, place(-260, -290, 35, 332, 20, 140))
    key(7, place(-560, -450, 5, 332, 14, 120))
    key(6, machine("percy", 320, 18, 30, "base_link", 0.8), "Far out on the crater floor, Perseverance takes a core", ["percy"])
    key(6, machine("percy", 270, 20, 10, "base_link", 0.8))
    key(5, machine("percy", 235, 22, 6, "base_link", 0.8))
    return keys


def camera(*args: str) -> None:
    subprocess.run([GB, "--timeout", "15", "instance", "camera", *args], check=True, capture_output=True)


def record(*args: str) -> None:
    subprocess.run([GB, "--timeout", "15", "instance", "record", *args], check=True, capture_output=True)


def overlay(machines: list[str]) -> None:
    """The TF tree drawn on these machines, or switched off."""
    state = ["on", *(f"base_fleet_{m}" for m in machines)] if machines else ["off"]
    subprocess.run([GB, "--timeout", "15", "instance", "overlay", "tf", *state], check=True, capture_output=True)


def as_args(view: str) -> list[str]:
    """A path view as `instance camera` arguments."""
    words = view.split()
    args = [words[0]] + [w for w in words[1:] if "=" not in w]
    for word in words[1:]:
        if "=" in word:
            name, value = word.split("=", 1)
            if name == "link":
                continue
            args += ["--compass"] if name == "compass" else [f"--{name}", value]
    return args


def duration(path: Path) -> float | None:
    """A finished video's length in seconds, or None while it is still being written."""
    probe = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(path)],
                           capture_output=True, text=True)
    try:
        return float(probe.stdout.strip())
    except ValueError:
        return None


def film(keys: list[dict], path: Path, clock: Gearbox) -> list[tuple[float, str]]:
    """Records the path in one take: the first view set before the recording
    starts, the whole path sent once it has, with the TF tree's switches the
    sim makes as it reaches each view that changes it. Gives back the
    captions with the video time each view is reached at, and writes them
    beside the take as soon as they are known."""
    path.unlink(missing_ok=True)
    plan = path.with_suffix(".path")
    views = [f"{k['at']:.2f} {k['view']}\n" for k in keys]
    switches = [f"{k['at']:.2f} tf " + (" ".join(["on", *(f"base_fleet_{m}" for m in k["tf"])]) if k["tf"] else "off") + "\n"
                for k in keys if k["tf"] is not None]
    plan.write_text("".join(views + switches))
    overlay([])
    camera(*as_args(keys[0]["view"]))
    time.sleep(2.0)
    record("start", "--shutter", str(SHUTTER), str(path))
    t0 = clock.clock().world_s
    camera("path", str(plan))
    # The sim takes camera requests four times a second.
    began = clock.clock().world_s - t0 + 0.125
    captions = [(round(began + k["at"], 2), k["caption"]) for k in keys if k["caption"]]
    path.with_suffix(".json").write_text(json.dumps(captions))
    end = began + keys[-1]["at"] + 1.0

    def now() -> float:
        """Seconds since the recording began, waiting out a sim slow to answer."""
        for _ in range(30):
            try:
                return clock.clock().world_s - t0
            except RuntimeError:
                time.sleep(1.0)
        raise RuntimeError("the sim's clock no longer answers")

    while (t := now()) < end:
        print(f"\r{t:6.1f} of {end:.0f} s", end="", flush=True)
        time.sleep(0.5)
    print(flush=True)
    record("stop")
    overlay([])
    for _ in range(600):
        if duration(path):
            return captions
        time.sleep(0.2)
    raise RuntimeError(f"{path} was never finished")


def escape(text: str) -> str:
    return text.replace("\\", "\\\\").replace("'", "’").replace(":", "\\:").replace(",", "\\,").replace(";", "\\;")


def caption(take: Path, captions: list[tuple[float, str]], out: Path) -> None:
    """Captions the take in its lower left, each faded in as its view is
    reached and out a few seconds on, with the whole faded in and out at its
    ends."""
    chain = []
    for start, text in captions:
        a, b = start + 0.4, start + 0.4 + CAPTION_S
        alpha = f"if(lt(t,{a}),0,if(lt(t,{a + 0.6}),(t-{a})/0.6,if(lt(t,{b - 0.6}),1,if(lt(t,{b}),({b}-t)/0.6,0))))"
        chain.append(f"drawtext=fontfile={FONT}:text='{escape(text)}':x=90:y=h-150:fontsize=46:fontcolor=white:"
                     f"alpha='{alpha}':enable='between(t,{a},{b})':shadowcolor=black@0.7:shadowx=2:shadowy=2")
    total = duration(take) or 0.0
    chain += ["fade=t=in:st=0:d=1.0", f"fade=t=out:st={total - 1.5:.3f}:d=1.5", "format=yuv420p"]
    subprocess.run(["ffmpeg", "-y", "-v", "error", "-i", str(take), "-vf", ",".join(chain),
                    "-c:v", "libx264", "-preset", "slow", "-tune", "film", "-crf", "16",
                    "-x264-params", "keyint=600:min-keyint=600:scenecut=0:ipratio=1.1:pbratio=1.1:aq-mode=3",
                    "-movflags", "+faststart", str(out)], check=True)


def preview(tour: Path, out: Path) -> None:
    """A 720p preview under 30 MiB: each two frames blended into one of
    half the rate, two-pass, with keyframes no sharper than the frames
    between them so nothing pops."""
    rate = 8 * 28 * 2 ** 20 / (duration(tour) or 200.0)
    params = "keyint=600:min-keyint=600:scenecut=0:ipratio=1.1:pbratio=1.1:aq-mode=3:qcomp=0.7"
    common = ["-vf", "tmix=frames=2,fps=30,scale=1280:720", "-sws_flags", "lanczos", "-c:v", "libx264", "-preset",
              "slower", "-tune", "film", "-b:v", f"{rate / 1000:.0f}k", "-x264-params", params, "-an"]
    log = str(out.with_suffix(""))
    subprocess.run(["ffmpeg", "-y", "-v", "error", "-i", str(tour), *common, "-pass", "1", "-passlogfile", log,
                    "-f", "mp4", "/dev/null"], check=True)
    subprocess.run(["ffmpeg", "-y", "-v", "error", "-i", str(tour), *common, "-pass", "2", "-passlogfile", log,
                    "-movflags", "+faststart", str(out)], check=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", default=str(ROOT / ".work" / "movie"))
    ap.add_argument("--keys", help="FIRST:LAST views to fly alone, as a trial, from the view before FIRST")
    ap.add_argument("--caption-only", action="store_true", help="caption the take already filmed")
    ap.add_argument("--print", action="store_true", help="print the path and quit")
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    keys = tour(pickers())
    if args.print:
        for k in keys:
            print(f"{k['at']:7.2f} {k['view']}" + (f"  # {k['caption']}" if k["caption"] else "") + (f"  tf={k['tf']}" if k["tf"] is not None else ""))
        return
    if args.keys:
        first, last = (int(k) for k in args.keys.split(":"))
        trial_keys = keys[max(first - 1, 0):last + 1]
        start = trial_keys[0]["at"]
        trial_keys = [{**k, "at": k["at"] - start} for k in trial_keys]
        clock = Gearbox()
        clock.wait_ready()
        trial = out / f"trial_{first:02d}_{last:02d}.mp4"
        film(trial_keys, trial, clock)
        print(f"trial: {duration(trial):.1f} s -> {trial}", flush=True)
        return
    take, cues = out / "mars_base_take.mp4", out / "mars_base_take.json"
    if not args.caption_only:
        clock = Gearbox()
        clock.wait_ready()
        cues.write_text(json.dumps(film(keys, take, clock)))
    final = out / "mars_base_tour.mp4"
    caption(take, [tuple(c) for c in json.loads(cues.read_text())], final)
    preview(final, out / "mars_base_tour_preview.mp4")
    print(f"tour: {duration(final):.1f} s -> {final}", flush=True)


if __name__ == "__main__":
    main()
