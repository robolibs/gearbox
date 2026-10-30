#!/usr/bin/env python3
"""A filmed tour of the Mars base in one take: the camera flies one smooth
path from robot to robot, never stopping on the way, recorded with a
blending shutter as one video, then captioned and cut down to a preview.

Run it against a sim with the base, its fleet and the farm runner working,
headless or not:

    gearbox run --headless 1920x1080 --planet mars --world bin/gearbox/assets/world/mars_base.usda
    gearbox spawn machine bin/gearbox/assets/world/mars_base_fleet.usda --machine base
    python3 scripts/control/mars_tour.py --out .work/movie

While it records, the sim's clock steps a sixtieth of a second a frame of
video, so the path's seconds below are seconds of video however fast the
GPU renders.
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
# Frames rendered and blended into each frame of video.
SHUTTER = 2
CAPTION_S = 4.5
FONT = subprocess.run(["fc-match", "-f", "%{file}", "DejaVu Sans"], capture_output=True, text=True).stdout.strip()


def place(x: float, y: float, above: float, bearing: float, pitch: float, distance: float) -> str:
    """A view of the base-frame point (x north, y west) `above` m over the ground."""
    lat = LAT0 + x / M_PER_DEG
    lon = LON0 - y / (M_PER_DEG * math.cos(math.radians(LAT0)))
    return (f"look lat={lat:.7f} lon={lon:.7f} above={above} bearing={bearing % 360:.1f} "
            f"pitch={pitch} distance={distance}")


def machine(name: str, bearing: float, pitch: float, distance: float) -> str:
    """A view round a fleet machine, following it as it moves, its bearing a compass bearing."""
    return f"look base_fleet_{name} bearing={bearing % 360:.1f} pitch={pitch} distance={distance} compass=1"


def tour() -> list[dict]:
    """The take as the views the path passes, each at its second, the
    caption shown from there, if any. Two views of a robot a few seconds
    apart hold it while the camera drifts round it; a view up high between
    far apart robots carries the camera over the base. The eye never goes
    into a building or near one: over the arches it stays 12 m up or more,
    clear of the washers' girders (7.6 m)."""
    keys = []

    def key(at, view, caption=None):
        keys.append({"at": at, "view": view, "caption": caption})

    key(0, place(5, -20, 0, 230, 34, 270))
    key(8, place(5, -30, 0, 295, 26, 190), "A Mars base for a crew of twelve, Jezero crater")
    # Harvest, the arches from C back to A, seen down through their roofs.
    key(17.5, machine("archcpicker2", 300, 70, 13.0), "Harvest: wheat and pulses in deep troughs")
    key(24.5, machine("archcpicker2", 330, 66, 13.0))
    key(30, machine("archbpicker1", 330, 70, 13.0), "A UR10e on a mobile base picks tomatoes and peppers")
    key(37, machine("archbpicker1", 300, 66, 13.0))
    key(43, place(29, -69.5, 6, 300, 50, 22), "Washers brush the dust off the arch glazing")
    key(49, place(31, -62, 5, 345, 55, 13))
    key(55, place(33, -62, 5, 0, 50, 13), "Salad in channels down an arch, a ridge gantry over them")
    # Glass and panels, between the domes and well up.
    key(62, place(45, -30, 8, 290, 45, 40))
    key(68, place(62, 20, 4, 250, 45, 34), "The dome washer sweeps the geodesic glass")
    key(74, place(62, 20, 4, 220, 32, 26))
    key(80, place(30, 55, 4, 240, 45, 50))
    key(86, machine("solarwasher2", 300, 28, 11.0), "Panel washers keep the arrays clean")
    key(92, machine("solarwasher2", 260, 30, 10.0))
    # Soil.
    key(99, place(-58, 63, 4, 330, 45, 50))
    key(106, machine("digger3", 340, 26, 7.0), "Excavators dig regolith for the soil works")
    key(113, machine("digger3", 15, 22, 6.0))
    key(118, machine("digger2", 10, 24, 6.0), "Loads tipped into the soil bays")
    key(124, machine("digger2", 340, 22, 6.0))
    key(130, place(-112, 30, 2, 340, 45, 40))
    key(137, place(-99, 0, 1, 300, 28, 32), "Water from the ice well; oxygen and methane from the air")
    key(143, place(-120, -5, 2, 245, 20, 26))
    # The habitat, the pad, and the base from above.
    key(150, place(-80, -48, 5, 220, 40, 55))
    key(157, place(-39.5, -91.5, 6, 200, 16, 32), "MARSHA, a habitat printed from regolith")
    key(163, place(-39.5, -91.5, 6, 250, 20, 29))
    key(170, place(-120, -120, 5, 240, 35, 70))
    key(177, place(-205, -150, 5, 230, 20, 70), "The Starship pad, fuelled from the base")
    key(183, place(-215, -155, 5, 285, 16, 55))
    key(191, place(-110, -100, 0, 265, 35, 150))
    key(198, place(-10, -42, 0, 250, 30, 90))
    key(207, place(10, -30, 0, 300, 42, 360))
    return keys


def camera(*args: str) -> None:
    subprocess.run([GB, "--timeout", "15", "instance", "camera", *args], check=True, capture_output=True)


def record(*args: str) -> None:
    subprocess.run([GB, "--timeout", "15", "instance", "record", *args], check=True, capture_output=True)


def as_args(view: str) -> list[str]:
    """A path view as `instance camera` arguments."""
    words = view.split()
    args = [words[0]] + [w for w in words[1:] if "=" not in w]
    for word in words[1:]:
        if "=" in word:
            name, value = word.split("=", 1)
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
    starts, the whole path sent once it has. Gives back the captions with
    the video time each view is reached at."""
    path.unlink(missing_ok=True)
    plan = path.with_suffix(".path")
    plan.write_text("".join(f"{k['at']:.2f} {k['view']}\n" for k in keys))
    camera(*as_args(keys[0]["view"]))
    time.sleep(2.0)
    record("start", "--shutter", str(SHUTTER), str(path))
    t0 = clock.clock().world_s
    camera("path", str(plan))
    # The sim takes camera requests four times a second.
    began = clock.clock().world_s - t0 + 0.125
    captions = [(round(began + k["at"], 2), k["caption"]) for k in keys if k["caption"]]
    end = began + keys[-1]["at"] + 1.0
    while (t := clock.clock().world_s - t0) < end:
        print(f"\r{t:6.1f} of {end:.0f} s", end="", flush=True)
        time.sleep(0.5)
    print(flush=True)
    record("stop")
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
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    keys = tour()
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
