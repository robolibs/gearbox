"""Verify Python selective updates against a named native viewer."""

import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts/control"))
from gearbox_client import COLORMAP, DrawEntity, Gearbox, HOST, _DRAW_HEADER, _SECTION_FMT, _Spec


def verify(instance):
    gb = Gearbox(instance=instance)
    gb.wait_ready()
    ns = f"vfx-gallery/python-update-{os.getpid()}"

    def retained():
        entries = gb.query(gb.addr, HOST["draw_list"], _Spec(0, ns, "").message(op=3), DrawEntity)
        entity = next(e for e in entries if e.props["ns"] == ns and e.props["id"] == "live")
        return {**{field: getattr(entity, field) for field in _DRAW_HEADER}, "props": entity.props,
                "sections": {name: entity.section(name) for name in (*_SECTION_FMT, "labels")}}

    def check(status):
        assert status is not None and status.code == 0, status

    def equal(expected, stage):
        actual = retained()
        for field in expected:
            assert actual[field] == expected[field], (stage, field, expected[field], actual[field])
        print(json.dumps({"passed": stage}), flush=True)

    try:
        check(gb.draw.lines(ns, "live", [(0, 0, 1), (1, 2, 1)], color="#40b0e0c0", color2="#e09040c0",
                            style="light_wall", glow=.5, blur=.5, pulse_hz=.75, flow_speed=1.5, spacing=2,
                            beam_height=3, fade=.25, urgency=.5, width=.125, drape=False,
                            pose=(.5, .25, 1, .2), props={"panel_finish": "black"}, colormap="turbo", vmin=-2, vmax=10,
                            wait=True))
        expected = retained()
        assert float(expected["props"]["blur"]) == .5
        for blur in (0, 1, 0):
            check(gb.draw.update(ns, "live", blur=blur, wait=True))
            expected["props"]["blur"] = str(float(blur))
            equal(expected, f"Python blur={blur} preserves geometry, colors and energy")
        binary = Path(__file__).resolve().parents[1] / "target/release/gearbox"
        for blur in (1, 0):
            subprocess.run([str(binary), "--instance", instance, "draw", "update", ns, "live", "--blur", str(blur)], check=True)
            expected["props"]["blur"] = str(blur)
            equal(expected, f"CLI blur={blur} preserves geometry, colors and energy")
        for options, fields, stage in [
            ({"glow": -.5}, {"glow": -.5}, "Python glow preserves other effects"),
            ({"pulse_hz": 0}, {"pulse_hz": 0}, "Python explicit zero only resets supplied pulse"),
            ({"spacing": 4}, {"dash": 4}, "Python spacing preserves flow"),
            ({"color": "#00000000"}, {"color": 1}, "Python transparent tint preserves secondary color"),
            ({"color2": "#00000000"}, {"color2": 1}, "Python transparent outline preserves primary color"),
            ({"colormap": "viridis"}, {"colormap": COLORMAP["viridis"]}, "Python palette preserves value range"),
            ({"vmin": 0}, {"vmin": 0}, "Python lower bound preserves upper bound"),
        ]:
            check(gb.draw.update(ns, "live", **options, wait=True))
            for field, value in fields.items():
                expected[field] = value
            equal(expected, stage)
        check(gb.draw.update(ns, "live", panel_finish="white", wait=True))
        expected["props"]["panel_finish"] = "white"
        equal(expected, "Python metadata-only update preserves all headers")
        check(gb.draw.update(ns, "live", wait=True))
        equal(expected, "Python empty update is header-safe")
        covariance_ns = f"{ns}/odom/cov"
        sentinels = {covariance_ns, f"{covariance_ns}/nested", f"{ns}/odometry/cov"}
        for sibling in sentinels:
            check(gb.draw.points(sibling, "custom", [(0, 0, 0)], wait=True))
        six = [0] * 36
        six[0], six[7], six[14], six[35] = .25, 1, .5, .09
        for empty in (None, []):
            for cov, parts in [([.25, 0, 0, 1], {"mean", "rings"}), (six, {"ellipsoid", "yaw"}),
                               ([.25, 0, 0, 0, 1, 0, 0, 0, .5], {"ellipsoid"})]:
                for values in (cov, empty):
                    gb.draw.odometry(ns, "odom", (0, 0, 0), velocity=(1, 0, 0), cov=values, wait=True)
                    entries = gb.query(gb.addr, HOST["draw_list"], _Spec(0, ns, "").message(op=3), DrawEntity)
                    keys = {(e.props["ns"], e.props["id"]) for e in entries}
                    actual = {id for namespace, id in keys if namespace == covariance_ns}
                    assert actual == ({"custom"} if values is empty else parts | {"custom"}), (len(cov), actual)
                    assert {(sibling, "custom") for sibling in sentinels}.issubset(keys)
                    assert {(f"{ns}/odom", id) for id in ("pose", "twist")}.issubset(keys)
        print(json.dumps({"passed": "Python omitted and empty odometry covariance clears every dimension without touching neighbors"}), flush=True)
        for options in ({"pose": (float("nan"), 0, 1)}, {"scale": (1, float("inf"), 1)}, {"lifetime": float("nan")}):
            rejected = gb.draw.update(ns, "live", **options, wait=True)
            assert rejected is not None and rejected.code != 0, ("invalid update accepted", options)
            equal(expected, "Python non-finite update rejected without changing the retained entity")
    finally:
        check(gb.draw.clear(ns, wait=True))
    assert not gb.draw.list(ns), "Python fixture left retained entities"


if __name__ == "__main__":
    verify(sys.argv[1])
