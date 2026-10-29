#!/usr/bin/env python3
"""A filmed tour of the Mars base in one take: the camera glides from robot
to robot on the sim clock, panning between neighbours and craning up over
the base between the far apart, recorded as one video and captioned.

Run it against a sim with the base, its fleet and the farm runner working,
headless or not:

    gearbox run --headless 1920x1080 --planet mars --world bin/gearbox/assets/world/mars_base.usda
    gearbox spawn machine bin/gearbox/assets/world/mars_base_fleet.usda --machine base
    python3 scripts/control/mars_farm.py &
    python3 scripts/control/mars_tour.py --out .work/movie

While it records, the sim's clock steps a sixtieth of a second a frame, so
the beats' seconds below are seconds of video however fast the GPU renders.
"""
from __future__ import annotations

import argparse
import json
import math
import subprocess
import time
from pathlib import Path

from gearbox_client import Gearbox

ROOT = Path(__file__).resolve().parents[2]
GB = str(ROOT / "target" / "release" / "gearbox")
# The base frame's origin on Mars, and metres a degree of latitude spans.
LAT0, LON0, M_PER_DEG = 18.4447, 77.4508, 59158.0
# Between one beat's glide ending and the next starting: the sim reads
# camera requests four times a second.
SETTLE_S = 0.35
CAPTION_S = 4.5
FONT = subprocess.run(["fc-match", "-f", "%{file}", "DejaVu Sans"], capture_output=True, text=True).stdout.strip()


def place(x: float, y: float, above: float, bearing: float, pitch: float, distance: float) -> list[str]:
    """A camera look at the base-frame point (x north, y west) `above` m over the ground."""
    lat = LAT0 + x / M_PER_DEG
    lon = LON0 - y / (M_PER_DEG * math.cos(math.radians(LAT0)))
    return ["look", "--lat", f"{lat:.7f}", "--lon", f"{lon:.7f}", "--above", f"{above}",
            "--bearing", f"{bearing % 360:.1f}", "--pitch", f"{pitch}", "--distance", f"{distance}"]


def machine(name: str, bearing: float, pitch: float, distance: float, compass: bool = False) -> list[str]:
    """A camera look round a fleet machine, following it as it moves; its
    bearing from behind the machine, or with `compass` from north."""
    return ["look", f"base_fleet_{name}", "--bearing", f"{bearing % 360:.1f}", "--pitch", f"{pitch}",
            "--distance", f"{distance}"] + (["--compass"] if compass else [])


def tour() -> list[dict]:
    """The take as beats: a look the camera glides to over `over` seconds,
    the caption shown as that glide begins, if any."""
    beats = []

    def beat(over, look, caption=None):
        beats.append({"over": over, "look": look, "caption": caption})

    beat(0, place(5, -20, 0, 230, 34, 270))
    beat(9, place(5, -30, 0, 295, 26, 190), "A Mars base for a crew of twelve, Jezero crater")
    # Harvest: the dome, then the arches from C back to A. Each move keeps
    # the eye on the far side of where it is heading, so what it looks at
    # leaves a building's glass and comes in through the next before it does.
    beat(6, machine("domeabot", 290, 40, 9.0, compass=True))
    beat(7, machine("domeabot", 320, 34, 8.0, compass=True), "Harvest: a polar gantry over the dome's soil beds")
    beat(7.5, machine("archcpicker2", 300, 40, 4.5, compass=True))
    beat(8, machine("archcpicker2", 110, 34, 4.0, compass=True), "Wheat and pulses in deep troughs")
    beat(4.5, machine("archbpicker1", 110, 40, 4.0, compass=True))
    beat(8, machine("archbpicker1", 80, 36, 4.0, compass=True), "A UR10e on a mobile base picks tomatoes and peppers")
    beat(5, place(31, -62, 2.0, 165, 12, 8))
    beat(7, place(36, -62, 2.0, 150, 14, 7), "Salad in channels down an arch, a ridge gantry over them")
    # Out through the arch's west side, then glass and panels.
    beat(4, place(33, -51, 1.5, 110, 18, 5))
    beat(6.5, place(62, 20, 4, 120, 38, 30))
    beat(8, place(62, 20, 4, 170, 32, 26), "The dome washer sweeps the geodesic glass")
    beat(6, machine("solarwasher2", 120, 25, 10.0))
    beat(7, machine("solarwasher2", 60, 30, 9.0), "Panel washers keep the arrays clean")
    # Soil.
    beat(6, machine("digger3", 340, 32, 7.0, compass=True))
    beat(9, machine("digger3", 15, 28, 6.0, compass=True), "Excavators dig regolith for the soil works")
    beat(4, machine("digger2", 300, 24, 5.5, compass=True))
    beat(7, machine("digger2", 250, 22, 5.0, compass=True), "Loads tipped into the soil bays")
    beat(5, place(-99, 0, 1, 300, 28, 32))
    beat(8, place(-120, -5, 2, 245, 20, 26), "Water from the ice well; oxygen and methane from the air")
    # The habitat, the pad, and the base from above.
    beat(6, place(-39.5, -91.5, 6, 200, 16, 32))
    beat(7, place(-39.5, -91.5, 6, 245, 20, 29), "MARSHA, a habitat printed from regolith")
    beat(7, place(-205, -150, 5, 230, 20, 70))
    beat(8, place(-215, -155, 5, 285, 16, 55), "The Starship pad, fuelled from the base")
    beat(7, place(-10, -42, 0, 250, 30, 90))
    beat(10, place(10, -30, 0, 300, 42, 360))
    return beats


def camera(*args: str) -> None:
    subprocess.run([GB, "--timeout", "15", "instance", "camera", *args], check=True, capture_output=True)


def record(*args: str) -> None:
    subprocess.run([GB, "--timeout", "15", "instance", "record", *args], check=True, capture_output=True)


def duration(path: Path) -> float | None:
    """A finished video's length in seconds, or None while it is still being written."""
    probe = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(path)],
                           capture_output=True, text=True)
    try:
        return float(probe.stdout.strip())
    except ValueError:
        return None


def film(beats: list[dict], path: Path, clock: Gearbox) -> list[tuple[float, str]]:
    """Records the beats in one take: the first look set before the recording
    starts, each later one sent once the glide before it has settled. Gives
    back the captions with the video time each was sent at."""
    path.unlink(missing_ok=True)
    camera(*beats[0]["look"])
    time.sleep(2.0)
    record("start", str(path))
    t0, due, captions = clock.clock().world_s, 0.0, []
    for b in beats[1:]:
        while clock.clock().world_s - t0 < due:
            time.sleep(0.02)
        camera(*b["look"], "--over", f"{b['over']}")
        t = clock.clock().world_s - t0
        if b["caption"]:
            captions.append((round(t, 2), b["caption"]))
        print(f"{t:7.2f} s  {' '.join(b['look'][:2])}  over {b['over']}", flush=True)
        due = t + b["over"] + SETTLE_S
    while clock.clock().world_s - t0 < due + 0.5:
        time.sleep(0.05)
    record("stop")
    for _ in range(300):
        if duration(path):
            return captions
        time.sleep(0.2)
    raise RuntimeError(f"{path} was never finished")


def escape(text: str) -> str:
    return text.replace("\\", "\\\\").replace("'", "’").replace(":", "\\:").replace(",", "\\,").replace(";", "\\;")


def caption(take: Path, captions: list[tuple[float, str]], out: Path) -> None:
    """Captions the take in its lower left, each faded in as its beat starts
    and out a few seconds on, with the whole faded in and out at its ends."""
    chain = []
    for start, text in captions:
        a, b = start + 0.4, start + 0.4 + CAPTION_S
        alpha = f"if(lt(t,{a}),0,if(lt(t,{a + 0.6}),(t-{a})/0.6,if(lt(t,{b - 0.6}),1,if(lt(t,{b}),({b}-t)/0.6,0))))"
        chain.append(f"drawtext=fontfile={FONT}:text='{escape(text)}':x=90:y=h-150:fontsize=46:fontcolor=white:"
                     f"alpha='{alpha}':enable='between(t,{a},{b})':shadowcolor=black@0.7:shadowx=2:shadowy=2")
    total = duration(take) or 0.0
    chain += ["fade=t=in:st=0:d=1.0", f"fade=t=out:st={total - 1.5:.3f}:d=1.5", "format=yuv420p"]
    subprocess.run(["ffmpeg", "-y", "-v", "error", "-i", str(take), "-vf", ",".join(chain),
                    "-c:v", "h264_nvenc", "-preset", "p5", "-rc", "vbr", "-cq", "19", "-b:v", "0",
                    "-movflags", "+faststart", str(out)], check=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", default=str(ROOT / ".work" / "movie"))
    ap.add_argument("--beats", help="FIRST:LAST beats to film alone, as a trial of their moves, from the look before FIRST")
    ap.add_argument("--caption-only", action="store_true", help="caption the take already filmed")
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    beats = tour()
    if args.beats:
        first, last = (int(k) for k in args.beats.split(":"))
        clock = Gearbox()
        clock.wait_ready()
        trial = out / f"trial_{first:02d}_{last:02d}.mp4"
        film(beats[first - 1:last + 1], trial, clock)
        print(f"trial: {duration(trial):.1f} s -> {trial}", flush=True)
        return
    take, cues = out / "mars_base_take.mp4", out / "mars_base_take.json"
    if not args.caption_only:
        clock = Gearbox()
        clock.wait_ready()
        cues.write_text(json.dumps(film(beats, take, clock)))
    final = out / "mars_base_tour.mp4"
    caption(take, [tuple(c) for c in json.loads(cues.read_text())], final)
    print(f"tour: {duration(final):.1f} s -> {final}", flush=True)


if __name__ == "__main__":
    main()
