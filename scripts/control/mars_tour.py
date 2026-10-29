#!/usr/bin/env python3
"""A filmed tour of the Mars base: each shot recorded as its own clip with the
camera gliding on the sim clock, then the clips joined with crossfades and
captions into one video.

Run it against a sim with the base, its fleet and the farm runner working,
headless or not:

    gearbox run --headless 1920x1080 --planet mars --world bin/gearbox/assets/world/mars_base.usda
    gearbox spawn machine bin/gearbox/assets/world/mars_base_fleet.usda --machine base
    python3 scripts/control/mars_farm.py &
    python3 scripts/control/mars_tour.py --out .work/movie

While a clip records, the sim's clock steps a sixtieth of a second a frame,
so a shot's times below are seconds of video however fast the GPU renders.
"""
from __future__ import annotations

import argparse
import math
import subprocess
import time
from pathlib import Path

from gearbox_client import Gearbox

ROOT = Path(__file__).resolve().parents[2]
GB = str(ROOT / "target" / "release" / "gearbox")
# The base frame's origin on Mars, and metres a degree of latitude spans.
LAT0, LON0, M_PER_DEG = 18.4447, 77.4508, 59158.0
FADE_S = 0.6
FONT = subprocess.run(["fc-match", "-f", "%{file}", "DejaVu Sans"], capture_output=True, text=True).stdout.strip()


def place(x: float, y: float, above: float, bearing: float, pitch: float, distance: float, over: float | None = None) -> list[str]:
    """A camera look at the base-frame point (x north, y west) `above` m over the ground."""
    lat = LAT0 + x / M_PER_DEG
    lon = LON0 - y / (M_PER_DEG * math.cos(math.radians(LAT0)))
    args = ["look", "--lat", f"{lat:.7f}", "--lon", f"{lon:.7f}", "--above", f"{above}",
            "--bearing", f"{bearing % 360:.1f}", "--pitch", f"{pitch}", "--distance", f"{distance}"]
    return args + (["--over", f"{over}"] if over else [])


def machine(name: str, bearing: float, pitch: float, distance: float, over: float = 0.3) -> list[str]:
    """A camera look round a fleet machine, following it as it moves."""
    return ["look", f"base_fleet_{name}", "--bearing", f"{bearing % 360:.1f}", "--pitch", f"{pitch}",
            "--distance", f"{distance}", "--over", f"{over}"]


def inside(x: float, y: float, z: float, facing: float, pitch: float, turn: float, seconds: float) -> tuple[list[str], list]:
    """A view from (x, y, z) along base-frame angle `facing` (CCW from +X), turned `turn` degrees over the shot."""
    def view(angle: float, over: float | None) -> list[str]:
        a = math.radians(angle)
        return place(x + 3.0 * math.cos(a), y + 3.0 * math.sin(a), z, -(angle + 180.0), pitch, 3.0, over)
    return view(facing, None), [(0.3, view(facing + turn, seconds - 0.6))]


def shots() -> list[dict]:
    """The tour: (name, seconds, caption, the view it opens on, later moves as (t, look))."""
    s = []

    def shot(name, seconds, caption, start, moves=()):
        s.append({"name": name, "seconds": seconds, "caption": caption, "start": start, "moves": list(moves)})

    shot("opening", 14, "A Mars base for a crew of twelve, Jezero crater",
         place(5, -20, 0, 230, 34, 270), [(0.2, place(5, -30, 0, 295, 26, 190, 13.6))])
    # Harvest.
    shot("arch_b", 13, "Harvest: a UR10e on a mobile base picks tomatoes and peppers",
         machine("archbpicker1", 140, 22, 5.5), [(0.5, machine("archbpicker1", 220, 26, 5.0, 12.0))])
    shot("arch_c", 11, "Wheat and pulses in deep troughs",
         machine("archcpicker2", 0, 22, 6.0), [(0.5, machine("archcpicker2", -40, 26, 5.5, 10.0))])
    shot("arch_a", 10, "Salad in channels down the arch, a ridge gantry over them",
         place(31, -62, 2.0, 180, 12, 9), [(0.2, place(36, -62, 2.0, 175, 14, 8, 9.6))])
    shot("vertical", 10, "The vertical farm's stacker crane",
         place(21, -108, 1.6, 0, 8, 6), [(0.2, place(24, -108, 1.8, 0, 10, 7.5, 9.6))])
    shot("nursery", 9, "Seedlings start in the farm hall's nursery",
         machine("hallbot", 200, 25, 4.0), [(0.5, machine("hallbot", 250, 30, 3.6, 8.0))])
    shot("dome", 10, "A polar gantry over the dome's soil beds",
         machine("domeabot", 150, 40, 9.0), [(0.5, machine("domeabot", 215, 34, 8.0, 9.0))])
    # Glass and panels.
    shot("arch_glass", 12, "Brushing the dust off the arch glazing",
         place(29, -62, 2.5, 330, 28, 30), [(0.2, place(31, -62, 2.5, 300, 24, 26, 11.6))])
    shot("dome_glass", 11, "The dome washer sweeps the geodesic glass",
         place(62, 20, 4, 120, 38, 30), [(0.2, place(62, 20, 4, 170, 32, 26, 10.6))])
    shot("panels", 10, "Panel washers keep the arrays clean",
         machine("solarwasher2", 120, 25, 10.0), [(0.5, machine("solarwasher2", 60, 30, 9.0, 9.0))])
    # Soil.
    shot("digger", 16, "Excavators dig regolith for the soil works",
         machine("digger1", 160, 25, 9.0), [(0.5, machine("digger1", 230, 22, 8.0, 15.0))])
    shot("digger2", 10, "Loads tipped into the soil bays",
         machine("digger2", 30, 22, 7.0), [(0.5, machine("digger2", -20, 20, 6.5, 9.0))])
    shot("isru", 12, "Water from the ice well; oxygen and methane from the air",
         place(-99, 0, 1, 300, 28, 32), [(0.2, place(-120, -5, 2, 245, 20, 26, 11.6))])
    # Inside.
    for name, seconds, caption, (x, y, z, facing, pitch, turn) in (
        ("control", 6, "Inside: the command centre", (-7.41, -58.39, 7.2, 280.0, 8, -25)),
        ("commons", 6, "The commons, where all twelve eat", (-31.94, 21.28, 2.3, 92.6, 10, 30)),
        ("marsha", 6, "MARSHA, printed from regolith", (-33.77, -87.17, 1.9, 210.0, 10, -30)),
    ):
        start, moves = inside(x, y, z, facing, pitch, turn, seconds)
        shot(name, seconds, caption, start, moves)
    shot("spaceport", 10, "The Starship pad, fuelled from the base",
         place(-205, -150, 5, 230, 20, 70), [(0.2, place(-215, -155, 5, 285, 16, 55, 9.6))])
    shot("closing", 12, "",
         place(-10, -42, 0, 250, 30, 90), [(0.2, place(10, -30, 0, 300, 42, 360, 11.6))])
    return s


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


def film(shot: dict, out: Path, clock: Gearbox) -> Path:
    """Records one shot: its opening view set before the recording starts,
    its moves sent on the sim clock, stopped when its seconds are up."""
    path = out / f"{shot['index']:02d}_{shot['name']}.mp4"
    path.unlink(missing_ok=True)
    camera(*shot["start"])
    time.sleep(2.0)
    record("start", str(path))
    t0 = None
    moves = list(shot["moves"])
    while True:
        now = clock.clock().world_s
        if t0 is None:
            t0 = now
        t = now - t0
        while moves and t >= moves[0][0]:
            camera(*moves.pop(0)[1])
        if t >= shot["seconds"] + 0.1:
            break
        time.sleep(0.05)
    record("stop")
    for _ in range(300):
        if duration(path):
            return path
        time.sleep(0.2)
    raise RuntimeError(f"{path} was never finished")


def escape(text: str) -> str:
    return text.replace("\\", "\\\\").replace("'", "’").replace(":", "\\:").replace(",", "\\,").replace(";", "\\;")


def assemble(clips: list[tuple[Path, str]], out: Path) -> None:
    """Joins the clips with crossfades, each captioned in its lower left as it opens."""
    inputs, chains, lengths = [], [], []
    for k, (path, caption) in enumerate(clips):
        inputs += ["-i", str(path)]
        lengths.append(duration(path) or 0.0)
        label = f"[c{k}]"
        if caption:
            alpha = "if(lt(t,0.8),0,if(lt(t,1.4),(t-0.8)/0.6,if(lt(t,4.6),1,if(lt(t,5.2),(5.2-t)/0.6,0))))"
            chains.append(f"[{k}:v]drawtext=fontfile={FONT}:text='{escape(caption)}':x=90:y=h-150:fontsize=46:"
                          f"fontcolor=white:alpha='{alpha}':shadowcolor=black@0.7:shadowx=2:shadowy=2,setpts=PTS-STARTPTS{label}")
        else:
            chains.append(f"[{k}:v]setpts=PTS-STARTPTS{label}")
    joined, offset = "[c0]", 0.0
    for k in range(1, len(clips)):
        offset += lengths[k - 1] - FADE_S
        chains.append(f"{joined}[c{k}]xfade=transition=fade:duration={FADE_S}:offset={offset:.3f}[x{k}]")
        joined = f"[x{k}]"
    total = sum(lengths) - FADE_S * (len(clips) - 1)
    chains.append(f"{joined}fade=t=in:st=0:d=1.0,fade=t=out:st={total - 1.5:.3f}:d=1.5,format=yuv420p[v]")
    subprocess.run(["ffmpeg", "-y", "-v", "error", *inputs, "-filter_complex", ";".join(chains), "-map", "[v]",
                    "-c:v", "h264_nvenc", "-preset", "p5", "-rc", "vbr", "-cq", "19", "-b:v", "0",
                    "-movflags", "+faststart", str(out)], check=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", default=str(ROOT / ".work" / "movie"))
    ap.add_argument("--only", help="comma-separated shot names to film; the rest are taken from earlier takes")
    ap.add_argument("--assemble-only", action="store_true", help="join the clips already filmed")
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    plan = shots()
    for k, shot in enumerate(plan):
        shot["index"] = k
    wanted = set(args.only.split(",")) if args.only else {s["name"] for s in plan}
    if not args.assemble_only:
        clock = Gearbox()
        clock.wait_ready()
        # World time runs while a clip records: the shots of places go first,
        # so the robots are at work by the time theirs are filmed.
        for shot in sorted(plan, key=lambda s: s["start"][1].startswith("base_fleet_")):
            if shot["name"] in wanted:
                started = time.time()
                path = film(shot, out, clock)
                print(f"{shot['name']}: {duration(path):.1f} s of video in {time.time() - started:.0f} s", flush=True)
    clips = [(out / f"{s['index']:02d}_{s['name']}.mp4", s["caption"]) for s in plan]
    missing = [str(p) for p, _ in clips if not p.exists()]
    if missing:
        raise SystemExit(f"not filmed yet: {', '.join(missing)}")
    assemble(clips, out / "mars_base_tour.mp4")
    print(f"tour: {duration(out / 'mars_base_tour.mp4'):.1f} s -> {out / 'mars_base_tour.mp4'}", flush=True)


if __name__ == "__main__":
    main()
