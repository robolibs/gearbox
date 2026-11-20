"""Offline world-helper contracts; native transport/binding integration is not exercised."""

import dataclasses
import importlib.util
import json
import math
from pathlib import Path
import struct
import sys
import types
import unittest
from unittest.mock import patch


def _load_client():
    datapod = types.ModuleType("datapod")
    datapod.datapod_type = lambda *args, **kwargs: dataclasses.dataclass
    peerbus = types.ModuleType("peerbus")
    path = Path(__file__).resolve().parents[1] / "scripts/control/gearbox_client.py"
    spec = importlib.util.spec_from_file_location("_gearbox_draw_world_client", path)
    module = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {"datapod": datapod, "peerbus": peerbus, spec.name: module}):
        spec.loader.exec_module(module)
    return module


client = _load_client()


class CaptureDraw(client.Draw):
    """Capture production logical specs as JSON-compatible dictionaries before chunking."""

    def __init__(self, wait=True, fail_at=None):
        super().__init__(None, wait=wait)
        self.specs = []
        self.fail_at = fail_at

    def send(self, spec, op=0):
        message = spec.message(op=op)
        header = {key: getattr(message, key) for key in client._DRAW_HEADER}
        self.specs.append({
            "ns": spec.ns, "id": spec.id, "op": op, "header": header,
            "points": list(spec.points), "instances": list(spec.instances),
            "colors": list(spec.colors), "labels": list(spec.labels),
            "props": client.unpack_map(client.pack_map(spec.props)), "offsets": list(spec.offsets),
            "values": list(spec.values), "cells": list(spec.cells),
            "radii": list(spec.radii), "rings": list(spec.rings),
            "qpoints": list(spec.qpoints), "ranges": list(spec.ranges),
        })
        wait = self.wait if spec.wait is None else spec.wait
        return client.Status(code=int(self.fail_at == len(self.specs))) if wait else None

    def _deliver(self, key, messages, coalesce, wait):
        for message in messages:
            props = message.props
            spec = {"ns": props["ns"], "id": props["id"], "op": message.op,
                    "header": {name: getattr(message, name) for name in client._DRAW_HEADER}, "props": props}
            for name, fmt in client._SECTION_FMT.items():
                data = message.section(name)
                spec[name] = list(struct.unpack(f"<{len(data) // struct.calcsize(fmt)}{fmt}", data))
            labels = message.section("labels").decode()
            spec["labels"] = labels.split("\n") if labels else []
            self.specs.append(spec)
        wait = self.wait if wait is None else wait
        return client.Status(code=int(self.fail_at == len(self.specs))) if wait else None


def contract_specs():
    draw = CaptureDraw()
    common = {"frame": "world", "lifetime": 7, "drape": False, "color": 0xFF00B4E9, "blur": 0.6}
    pose = (10, 20, 30, math.cos(0.25), 0, 0, math.sin(0.25))
    draw.terrain_marks("contract", "terrain", [(1, 2, 3), (4, 5, 6)], glyph="triangle", size=0.4, **common)
    draw.route_stations("contract", "route", [(0, 0, 0), (0, 0, 3), (4, 0, 3)], spacing=2, size=0.4, **common)
    draw.destination("contract", "destination", pose, radius=0.8, label="Dock", **common)
    draw.clearance_box("contract", "clearance", pose, size=(2, 4, 6), label="Bounds", **common)
    draw.field_volume("contract", "field", pose, size=(2, 4, 6), spacing=0.5, **common)
    return draw.specs


def composite_contract_specs():
    draw = CaptureDraw()
    common = {"frame": "world", "lifetime": 7, "drape": False}
    pose = (10, 20, 30, math.cos(0.25), 0, 0, math.sin(0.25))
    draw.waypoints("contract", "waypoints", [(0, 0, 0), (0, 0, 3), (4, 0, 3)], current=1, **common)
    draw.detection3d("contract", "detections3d", [{"pose": pose, "size": (2, 4, 6), "class": "fixture\nupper",
                                                    "score": .82, "track": 17}], **common)
    draw.callout("contract", "callout", (1, 2, 3), "Supplied caption", offset=(.4, .5, .6), color=0xFF00B4E9, **common)
    draw.frame("contract", "base", (1, 2, 3, .5), parent=(-1, -2, -3), length=.5, label="Frame", **common)
    draw.detection2d("contract", "detections2d", pose, [(320, 240, 100, 80)], hfov=1, vfov=.8,
                     image=(640, 480), classes=["fixture"], scores=[.82], depth=2, **common)
    draw.detection3d("contract", "wire3d", [{"pose": pose, "size": (2, 4, 6), "class": "car",
                                          "score": .82, "track": 17}], only_edge=True, line_width=-3, **common)
    return draw.specs


def helper_contract_specs():
    draw = CaptureDraw()
    common = {"frame": "world", "lifetime": 7, "drape": False}
    pose = (1, 2, 3, .5)
    poses = [pose, (4, 5, 6), (7, 8, 9)]
    xyz = [(0, 0, 0), (2, 1, .5), (4, 3, 1)]
    outer = [(0, 0, 0), (6, 0, 0), (6, 6, 0), (0, 6, 0)]
    inner = [(1, 1, 0), (2, 1, 0), (2, 2, 0), (1, 2, 0)]
    strips = [xyz, [(1, 2, 0), (3, 4, 0)]]
    tint = 0x806f32d1
    draw.cloud("contract", "cloud", xyz, values=[0, .25, 1], **common)
    draw.cloud("contract", "cloud-quant", xyz, values=[0, .25, 1], quant=.01, **common)
    draw.scan("contract", "scan", [1, 2.5, float("inf"), 4], pose=pose, angle_min=-.5, angle_increment=.25, range_max=5, **common)
    draw.range_cone("contract", "range", pose, fov=1, min_range=.25, max_range=4, range=2, **common)
    draw.camera_frustum("contract", "camera", pose, hfov=1, vfov=.75, near=.25, far=5, **common)
    draw.pose_array("contract", "poses", poses, glyph="chevron", length=.5, **common)
    draw.particles("contract", "particles", poses, [0, .5, 1], min_length=.1, max_length=.5, color=tint, **common)
    draw.field("contract", "field-hole", outer, holes=[inner], color=tint, **common)
    draw.polygon("contract", "multi-polygon", polygons=[[outer, inner], [xyz]], colors=[tint, 0xff00b4e9], **common)
    draw.headland("contract", "headland", outer, inner, **common)
    draw.swaths("contract", "swaths", strips, width=2, done=1, color=tint, **common)
    draw.turns("contract", "turns", strips, width=2, **common)
    draw.ab_line("contract", "ab", xyz[0], xyz[1], extend=8, **common)
    draw.coverage("contract", "coverage", [0, 1, 2, 4], 2, 2, cell=.5, origin=(1, 2, 3), **common)
    draw.implement("contract", "implement", 3, [True, False, True], depth=.6, **common)
    draw.zone("contract", "zone", [outer, inner], kind="stop", **common)
    draw.graph("contract", "graph", xyz, [(0, 1), (1, 2)], labels=["A", "B", "C"], **common)
    draw.ghost_trail("contract", "ghost", "rover", poses, step=2, joints=["steer", "wheel"],
                     values=[[.1, .2], [.3, .4], [.5, .6]], color=tint, **common)
    for kind in ("sparkline", "bars"):
        draw.panel("contract", kind, xyz[1], kind, values=[.25, -1.5, 2], labels=["A", "B", "C"], title="Sample", **common)
    draw.gauge("contract", "gauge", xyz[1], 42, lo=0, hi=100, title="Battery", unit="%", **common)
    draw.chip("contract", "chip", xyz[1], "Running", color=tint, **common)
    draw.table("contract", "table", xyz[1], [("Speed", "2 m/s"), ("State", "Ready")], title="Robot", **common)
    draw.legend("contract", "legend", "viridis", -2, 8, title="Height", unit="m")
    draw.badge("contract", "badge", "rover", "Ready", color=tint, height=2, drape=False)
    draw.voxels("contract", "voxels", xyz, size=.2, values=[.25, .5, 1], holo=True, **common)
    draw.wrench("contract", "wrench", pose, (2, -1, 3), (1, 2, .5), scale=.1, **common)
    draw.contact("contract", "contact", xyz[1], (1, 2, 3), 8, scale=.05, **common)
    draw.compass("contract", "compass", xyz[1], size=4, color=tint, **common)
    draw.angle("contract", "angle", xyz[1], -.5, 1, radius=2, color=tint, **common)
    draw.ruler("contract", "ruler", xyz[0], xyz[2], spacing=.5, **common)
    draw.scale_bar("contract", "scale", xyz[1], 5, **common)
    draw.covariance("contract", "covariance", pose, [.25, 0, 0, 1], sigmas=[1, 2], color=tint, **common)
    draw.covariance("contract", "covariance3", pose, [.25, 0, 0, 0, 1, 0, 0, 0, .5], sigmas=[1, 2], color=tint, **common)
    six = [0] * 36
    six[0], six[7], six[14], six[35] = .25, 1, .5, .09
    draw.covariance("contract", "covariance6", (1, 2, 3, math.cos(.25), 0, 0, math.sin(.25)), six, sigmas=[1, 2], color=tint, **common)
    draw.odometry("contract", "odometry", pose, velocity=(1, 2, 0), cov=[.25, 0, 0, 1], **common)
    draw.odometry("contract", "odometry-omitted", pose, **common)
    draw.odometry("contract", "odometry-empty", pose, cov=[], **common)
    draw.band("contract", "band", xyz, [.1, .2, .3], **common)
    draw.rollouts("contract", "rollouts", [*strips, xyz], costs=[0, .5, 1], collisions=[False, True, False], **common)
    draw.path("contract", "path", xyz, pose_markers=1, marker="arrow", **common)
    draw.tf_tree("rover")
    draw.points("contract", "payload-controls", xyz, radii=[.1, -2, .3], colors=[tint, 1, 0xff00b4e9],
                color=tint, color2=0x40cc7733, style="point_glow", width=-3, glow=-1, pulse_hz=.75,
                flow_speed=.5, spacing=.25, beam_height=1.5, fade=.3, urgency=.4, opacity=.75, values=[0, .5, 1],
                colormap="viridis", vmin=-1, vmax=2, **common)
    draw.machine_ring("contract", "ring-hover", xyz[1], radius=3, mark="hover", **common)
    draw.machine_ring("contract", "ring-selected", xyz[1], radius=3, mark="selected", color=tint, **common)
    draw.machine_ring("contract", "ring-following", xyz[1], radius=3, mark="following", **common)
    draw.machine_ring("contract", "ring-small", xyz[1], radius=0.4, mark="following", **common)
    return draw.specs


def points(spec):
    flat = spec["points"]
    return [tuple(flat[i:i + 3]) for i in range(0, len(flat), 3)]


class DrawWorldTests(unittest.TestCase):
    def test_odometry_without_covariance_removes_only_its_covariance_parts(self):
        six = [0] * 36
        six[0], six[7], six[14], six[35] = .25, 1, .5, .09
        for empty in (None, []):
            for cov in ([.25, 0, 0, 1], six, [.25, 0, 0, 0, 1, 0, 0, 0, .5]):
                with self.subTest(empty=empty, dimension=len(cov)):
                    draw = CaptureDraw()
                    preserved = {(ns, "custom") for ns in ("n/odom/cov", "n/odom/cov/nested", "n/odometry/cov")}
                    retained = set(preserved)
                    for values in (cov, empty):
                        draw.specs.clear()
                        draw.odometry("n", "odom", (0, 0, 0), cov=values, velocity=(1, 0, 0))
                        for spec in draw.specs:
                            key = spec["ns"], spec["id"]
                            if spec["op"] == client.DRAW_OP["delete"]:
                                retained.discard(key)
                            else:
                                self.assertEqual(spec["op"], client.DRAW_OP["upsert"])
                                retained.add(key)
                        self.assertTrue(preserved.issubset(retained))
                        if values is empty:
                            self.assertEqual(retained, preserved | {("n/odom", "pose"), ("n/odom", "twist")})
                        else:
                            self.assertGreater(len(retained), 5)

    def test_covariance_dimension_changes_remove_obsolete_components(self):
        draw = CaptureDraw()
        six = [0] * 36
        six[0], six[7], six[14], six[35] = .25, 1, .5, .09
        retained = {}
        for cov, expected in [(six, {"ellipsoid", "yaw"}), ([.25, 0, 0, 1], {"mean", "rings"}),
                              ([.25, 0, 0, 0, 1, 0, 0, 0, .5], {"ellipsoid"}), (six, {"ellipsoid", "yaw"})]:
            draw.specs.clear()
            draw.covariance("n", "cov", (0, 0, 0), cov)
            for spec in draw.specs:
                if spec["op"] == client.DRAW_OP["delete"]:
                    retained.pop(spec["id"], None)
                else:
                    retained[spec["id"]] = spec
            self.assertEqual(set(retained), expected)

    def test_sector_default_inner_extent_and_explicit_scale(self):
        draw = CaptureDraw()
        draw.shapes("n", "fan", "sector", sizes=[(2, 1.5, 0)])
        draw.shapes("n", "ring", "sector", sizes=[(2, 1.5, 0)], size=(1, 1, .5))
        draw.shapes("n", "box", "box")
        self.assertEqual([e["header"]["sz"] for e in draw.specs], [0, .5, 1])

    def test_parity_rejects_missing_fields_and_single_bit_color_errors(self):
        from compare_draw_world import compare
        for expected, actual in [({"color2": 12}, {}), ({}, {"rings": [2]}),
                                 (0xFF00B4E9, 0xFF00B4E8), (1, True), ([1], (1,))]:
            with self.subTest(expected=expected, actual=actual), self.assertRaises(AssertionError):
                compare(expected, actual)
        compare(0.1, 0.10000000149011612)

    def test_helper_payload_fixture_covers_all_sections_and_header_fields(self):
        specs = helper_contract_specs()
        for spec in specs:
            self.assertEqual(set(spec["header"]), set(client._DRAW_HEADER))
            self.assertTrue(set(client._SECTIONS).issubset(spec))
            self.assertEqual(spec["header"]["last"], 1)
        for field in client._SECTIONS:
            self.assertTrue(any(spec[field] for spec in specs), field)

    def test_field_style_is_additive_to_existing_ids(self):
        self.assertEqual(client.DRAW_STYLE["ruler"], 19)
        self.assertEqual(client.DRAW_STYLE["field"], 20)
        self.assertEqual(client.DRAW_KIND["interact"], 13)

    def test_terrain_shape_spans_and_depth(self):
        for glyph, segments in (("square", 4), ("cross", 2), ("triangle", 3), ("diamond", 4)):
            with self.subTest(glyph=glyph):
                draw = CaptureDraw()
                self.assertTrue(draw.terrain_marks("n/", "marks", [(2, 3, 4)], glyph=glyph, size=2, drape=False).ok)
                spec = draw.specs[0]
                xyz = points(spec)
                self.assertEqual((spec["ns"], spec["id"]), ("n/marks", "marks"))
                self.assertEqual(len(xyz), segments * 2)
                self.assertEqual({p[2] for p in xyz}, {4})
                self.assertEqual((min(p[0] for p in xyz), max(p[0] for p in xyz)), (1, 3))
                self.assertEqual((min(p[1] for p in xyz), max(p[1] for p in xyz)), (2, 4))
                self.assertEqual(spec["header"]["flags"], 0)
                self.assertEqual(spec["header"]["sub"], client.LINE_MODE["list"])
                self.assertEqual(spec["header"]["style"], client.DRAW_STYLE["plain"])
                self.assertEqual(spec["header"]["glow"], 0)

    def test_segment_glow_defaults_and_explicit_controls(self):
        for glow in (None, 0, -1, 0.8):
            with self.subTest(glow=glow):
                draw = CaptureDraw()
                common = {} if glow is None else {"glow": glow}
                draw.terrain_marks("n", "terrain", [(0, 0, 0)], **common)
                draw.route_stations("n", "route", [(0, 0, 0), (1, 0, 0)], **common)
                draw.destination("n", "destination", label="Dock", **common)
                draw.clearance_box("n", "clearance", label="Bounds", **common)
                for spec in draw.specs:
                    if spec["header"]["kind"] == client.DRAW_KIND["lines"]:
                        self.assertEqual(spec["header"]["glow"], 0 if glow is None else glow)
                    elif glow is None:
                        self.assertEqual(spec["header"]["glow"], 0)

    def test_route_stations_use_equal_3d_distance(self):
        draw = CaptureDraw()
        draw.route_stations("n", "route", [(0, 0, 0), (0, 0, 3), (4, 0, 3)], spacing=2, size=0.4)
        vertices = points(draw.specs[1])
        centres = [tuple((a + b) / 2 for a, b in zip(vertices[i], vertices[i + 1])) for i in range(0, len(vertices), 6)]
        self.assertEqual(centres, [(0, 0, 0), (0, 0, 2), (1, 0, 3), (3, 0, 3)])
        self.assertEqual(len(points(draw.specs[0])), 4)

    def test_route_duplicate_points_and_nonfinite_gaps(self):
        draw = CaptureDraw()
        draw.route_stations("n", "route", [(0, 0, 0), (0, 0, 0), (1, 0, 0),
                                              (math.nan, 0, 0), (10, 0, 0), (11, 0, 0)], spacing=1)
        self.assertEqual(points(draw.specs[0]), [(0, 0, 0), (1, 0, 0), (10, 0, 0), (11, 0, 0)])
        self.assertEqual(len(points(draw.specs[1])), 4 * 6)

    def test_generation_has_input_and_station_bounds(self):
        def bounded_input():
            for _ in range(client._WORLD_MAX_POINTS):
                yield (math.nan, 0, 0)
            raise AssertionError("read beyond bounded inputs")
        draw = CaptureDraw()
        draw.terrain_marks("n", "a", bounded_input())
        draw.route_stations("n", "b", bounded_input())
        self.assertTrue(all(s["op"] == client.DRAW_OP["delete"] for s in draw.specs))
        draw = CaptureDraw()
        draw.route_stations("n", "r", [(0, 0, 0), (client._F32_MAX, 0, 0)], spacing=1e-40)
        self.assertEqual(len(points(draw.specs[1])), client._WORLD_MAX_STATIONS * 6)

    def test_invalid_dimensions_clear_stable_children(self):
        for invalid in (0, -1, math.nan, math.inf, 1e40):
            with self.subTest(invalid=invalid):
                draw = CaptureDraw()
                draw.terrain_marks("n", "a", [(0, 0, 0)], size=invalid)
                draw.destination("n", "b", radius=invalid, label="x")
                draw.clearance_box("n", "c", size=(1, invalid, 1), label="x")
                draw.field_volume("n", "d", spacing=invalid)
                self.assertEqual([s["id"] for s in draw.specs],
                                 ["marks", "rings", "marker", "leader", "label", "brackets", "label", "volume"])
                self.assertTrue(all(s["op"] == client.DRAW_OP["delete"] for s in draw.specs))
                draw = CaptureDraw()
                draw.route_stations("n", "r", [(0, 0, 0), (1, 0, 0)], spacing=invalid)
                self.assertEqual([s["op"] for s in draw.specs], [0, client.DRAW_OP["delete"]])

    def test_empty_and_coincident_geometry_deletes(self):
        draw = CaptureDraw()
        draw.terrain_marks("n", "a", [])
        draw.route_stations("n", "b", [(0, 0, 0)] * 3)
        self.assertTrue(all(s["op"] == client.DRAW_OP["delete"] for s in draw.specs))

    def test_native_f32_collapsed_geometry_and_underflow(self):
        draw = CaptureDraw()
        draw.route_stations("n", "r", [(1e20, 0, 0), (1e20 + 16384, 0, 0)])
        draw.terrain_marks("n", "t", [(0, 0, 0)], size=1e-50)
        draw.field_volume("n", "v", spacing=1e-50)
        self.assertTrue(all(s["op"] == client.DRAW_OP["delete"] for s in draw.specs))

    def test_common_frame_lifetime_and_negative_controls(self):
        for frame, draped in (("world", True), ("robot/odom", True), ("robot/link", False)):
            draw = CaptureDraw()
            common = dict(frame=frame, lifetime=7, glow=-1, pulse_hz=-2, flow_speed=-3,
                          beam_height=-4, snapshot=True, flat=True, additive=True)
            draw.terrain_marks("n", "a", [(0, 0, 0)], **common)
            draw.route_stations("n", "b", [(0, 0, 0), (1, 0, 0)], **common)
            draw.destination("n", "c", label="Dock", **common)
            draw.clearance_box("n", "d", label="Bounds", **common)
            draw.field_volume("n", "e", **common)
            for spec in draw.specs:
                h = spec["header"]
                self.assertEqual(spec["props"]["frame"], frame)
                self.assertEqual(h["lifetime_s"], 7)
                self.assertEqual((h["glow"], h["pulse_hz"], h["flow_speed"], h["beam_height"]), (-1, -2, -3, -4))
                self.assertEqual(bool(h["flags"] & client.DRAW_FLAG["drape"]), draped)
                for name in ("snapshot", "flat", "additive"):
                    self.assertTrue(h["flags"] & client.DRAW_FLAG[name])
                self.assertFalse(h["flags"] & client.DRAW_FLAG["on_top"])

    def test_destination_pose_geometry_and_rigid_drape_props(self):
        draw = CaptureDraw()
        draw.destination("n", "goal", (10, 20, 30, 0, 0, 0, 3), radius=2, label="Dock")
        for spec in draw.specs:
            h = spec["header"]
            self.assertEqual((h["px"], h["py"], h["pz"]), (10, 20, 30))
            self.assertEqual((h["qw"], h["qx"], h["qy"], h["qz"]), (0, 0, 0, 1))
            self.assertEqual(spec["props"]["drape_pose_height"], "true")
        rings, marker, leader, label = draw.specs
        self.assertNotIn("drape_anchor", rings["props"])
        self.assertEqual([s["id"] for s in draw.specs], ["rings", "marker", "leader", "label"])
        self.assertEqual(len(points(rings)), 96)
        for x, y, z in points(rings):
            self.assertEqual(z, 0)
            self.assertAlmostEqual(math.hypot(x, y), 2, places=5)
        for spec in (marker, leader, label):
            self.assertEqual(spec["props"]["drape_anchor"], "true")
        self.assertEqual(len(points(marker)), 8)
        self.assertEqual(points(label), [(1.5, 0, 4)])

    def test_machine_ring_is_one_ring_entity_carrying_its_mark(self):
        for mark in ("hover", "selected", "following"):
            draw = CaptureDraw()
            draw.machine_ring("n", "tractor", (10, -4, 0.1), radius=3, mark=mark)
            [ring] = draw.specs
            self.assertEqual((ring["ns"], ring["id"], ring["op"]), ("n/tractor", "ring", client.DRAW_OP["upsert"]))
            h = ring["header"]
            self.assertEqual((h["kind"], h["style"], h["width"]), (client.DRAW_KIND["points"], client.DRAW_STYLE["ring"], 6))
            self.assertEqual((h["color"], ring["props"]["mark"]), (client.RING_MARKS[mark], mark))
            self.assertEqual([tuple(round(v, 5) for v in p) for p in points(ring)], [(10, -4, 0.1)])
        draw = CaptureDraw()
        draw.machine_ring("n", "tractor", radius=0.4)
        self.assertEqual([(s["id"], s["op"]) for s in draw.specs], [("ring", client.DRAW_OP["delete"])])
        with self.assertRaises(ValueError):
            draw.machine_ring("n", "tractor", mark="spinning")

    def test_box_extents_and_field_wire_properties(self):
        draw = CaptureDraw()
        draw.clearance_box("n", "box", size=(2, 4, 6), label="Bounds", drape=False)
        brackets = points(draw.specs[0])
        self.assertEqual(len(brackets), 48)
        for axis, half in enumerate((1, 2, 3)):
            self.assertEqual(max(abs(p[axis]) for p in brackets), half)
        draw.field_volume("n", "field", size=(2, 4, 6), spacing=0.5, drape=False)
        spec = draw.specs[-1]
        self.assertEqual(spec["instances"], [0, 0, 0, 1, 0, 0, 0, 2, 4, 6])
        self.assertEqual((spec["header"]["kind"], spec["header"]["sub"], spec["header"]["style"]), (4, 0, 20))
        self.assertEqual(spec["header"]["dash"], 0.5)
        self.assertEqual(spec["header"]["flags"], client.DRAW_FLAG["dashed"])
        self.assertNotIn("drape_anchor", spec["props"])

    def test_labels_are_deleted_when_removed(self):
        draw = CaptureDraw()
        draw.destination("n", "a", label="")
        draw.clearance_box("n", "b", label="")
        self.assertEqual([s["ns"] for s in draw.specs if s["op"] == client.DRAW_OP["delete"]], ["n/a", "n/b"])

    def test_transparent_zero_never_requests_auto_color(self):
        draw = CaptureDraw()
        draw.terrain_marks("n", "a", [(0, 0, 0)], color=0)
        draw.route_stations("n", "b", [(0, 0, 0), (1, 0, 0)], color="#00000000")
        draw.destination("n", "c", color=0, label="Dock")
        draw.clearance_box("n", "d", color=0, label="Bounds")
        draw.field_volume("n", "e", color=0)
        self.assertTrue(all(s["header"]["color"] == 1 for s in draw.specs))

    def test_pose_normalization_and_nonfinite_pose(self):
        draw = CaptureDraw()
        draw.destination("n", "a", (0, 0, 0, 0, 0, 0, 0))
        self.assertEqual(draw.specs[0]["header"]["qw"], 1)
        draw = CaptureDraw()
        draw.destination("n", "a", (0, 0, 0, 1e308, 0, 0, 1e308))
        self.assertAlmostEqual(draw.specs[0]["header"]["qw"], math.sqrt(0.5))
        draw = CaptureDraw()
        draw.destination("n", "a", (0, 0, math.nan))
        self.assertTrue(all(s["op"] == client.DRAW_OP["delete"] for s in draw.specs))

    def test_invalid_input_types_fail_before_sending(self):
        calls = [lambda d: d.terrain_marks("n", "a", [(0, 0)], glyph="Square"),
                 lambda d: d.terrain_marks("n", "a", [(0,)]),
                 lambda d: d.terrain_marks("n", "a", ["123"]),
                 lambda d: d.destination("n", "a", (0, 0, 0, 0, 0)),
                 lambda d: d.clearance_box("n", "a", size=(1, 2)),
                 lambda d: d.field_volume("n", "a", size="123")]
        for call in calls:
            draw = CaptureDraw()
            with self.assertRaises((TypeError, ValueError)):
                call(draw)
            self.assertEqual(draw.specs, [])

    def test_status_failure_short_circuits_and_queued_returns_none(self):
        draw = CaptureDraw(fail_at=2)
        self.assertFalse(draw.destination("n", "a", label="Dock").ok)
        self.assertEqual(len(draw.specs), 2)
        draw = CaptureDraw(wait=False)
        self.assertIsNone(draw.destination("n", "a", label="Dock"))
        self.assertEqual(len(draw.specs), 4)
        draw = CaptureDraw()
        self.assertIsNone(draw.clearance_box("n", "a", wait=False))

    def test_production_packer_chunks_segments_without_reordering(self):
        class Gateway:
            addr = "offline"
            def __init__(self):
                self.messages = []
            def call(self, addr, topic, msg, result):
                self.messages.append(msg)
                return client.Status()
        gateway = Gateway()
        draw = client.Draw(gateway)
        draw.terrain_marks("n", "a", ((i, 0, 0) for i in range(1000)), glyph="square", drape=False)
        messages = gateway.messages
        self.assertGreater(len(messages), 1)
        self.assertEqual({m.op for m in messages}, {client.DRAW_OP["append"]})
        self.assertEqual({m.sub for m in messages}, {client.LINE_MODE["list"]})
        self.assertEqual({m.props["ns"] for m in messages}, {"n/a"})
        self.assertEqual({m.props["id"] for m in messages}, {"marks"})
        self.assertEqual(len({m.seq for m in messages}), 1)
        self.assertEqual([m.last for m in messages], [0] * (len(messages) - 1) + [1])
        count = 0
        for msg in messages:
            self.assertEqual(msg.chunk_offset, count)
            self.assertEqual(msg.points_len % 24, 0)
            self.assertLessEqual(msg.points_len, client.DRAW_CHUNK_BYTES)
            data = struct.unpack(f"<{msg.points_len // 4}f", msg.section("points"))
            self.assertTrue(all(math.isfinite(v) for v in data))
            count += msg.points_len // 12
        self.assertEqual(count, 8000)

    def test_contract_fixture_has_all_ten_stable_children(self):
        specs = contract_specs()
        self.assertEqual(len(specs), 10)
        self.assertEqual([(s["ns"], s["id"]) for s in specs], [
            ("contract/terrain", "marks"), ("contract/route", "route"), ("contract/route", "stations"),
            ("contract/destination", "rings"), ("contract/destination", "marker"),
            ("contract/destination", "leader"), ("contract/destination", "label"),
            ("contract/clearance", "brackets"), ("contract/clearance", "label"), ("contract/field", "volume")])

    def test_composite_contract_fixture_has_all_twenty_four_children(self):
        specs = composite_contract_specs()
        self.assertEqual(len(specs), 24)
        self.assertEqual(len({(s["ns"], s["id"]) for s in specs}), 24)
        self.assertEqual(sum(s["op"] == client.DRAW_OP["delete"] for s in specs), 6)

    def test_waypoints_use_shared_station_geometry_and_clear_selection(self):
        draw = CaptureDraw()
        draw.waypoints("n", "route", [(0, 0, 0), (3, 0, 0)], current=1, frame="robot/link", lifetime=7)
        children = {s["id"]: s for s in draw.specs}
        self.assertEqual(set(children), {"route", "pillars", "stations", "current", "selection"})
        self.assertEqual(children["pillars"]["header"]["kind"], client.DRAW_KIND["points"])
        self.assertEqual(children["pillars"]["colors"][1], 1)
        self.assertEqual(children["current"]["props"]["label_selected"], "true")
        self.assertEqual(children["current"]["labels"], ["2"])
        self.assertEqual(len(points(children["selection"])), 16)
        reference = CaptureDraw()
        reference.route_stations("n", "reference", [(0, 0, 0), (3, 0, 0)], spacing=3, size=.45)
        self.assertEqual(children["stations"]["points"], reference.specs[1]["points"])
        for s in children.values():
            self.assertEqual(s["props"]["frame"], "robot/link")
            self.assertEqual(s["header"]["lifetime_s"], 7)
            self.assertEqual(s["header"]["glow"], 0)
        draw.specs.clear()
        draw.waypoints("n", "route", [], current=0)
        self.assertEqual({s["id"] for s in draw.specs}, set(children))
        self.assertTrue(all(s["op"] == client.DRAW_OP["delete"] for s in draw.specs))
        draw.specs.clear()
        draw.waypoints("n", "route", [(0, 0, 0)], current=9)
        self.assertEqual({s["id"] for s in draw.specs if s["op"] == client.DRAW_OP["delete"]}, {"current", "selection"})

    def test_structural_helper_corner_weights_and_overrides(self):
        draw = CaptureDraw()
        draw.clearance_box("n", "bounds", size=(2, 4, 6), color=0x8050c080)
        self.assertEqual(draw.specs[0]["header"]["width"], -client.STRUCTURE_CORNER_GAIN)
        self.assertEqual(draw.specs[0]["header"]["color"], 0x8050c080)
        self.assertEqual(len(points(draw.specs[0])), 48)
        draw.specs.clear()
        draw.waypoints("n", "route", [(0, 0, 0), (3, 0, 0)], current=1)
        children = {s["id"]: s for s in draw.specs}
        self.assertEqual(children["selection"]["header"]["width"], -client.STRUCTURE_CORNER_GAIN)
        self.assertEqual(children["stations"]["header"]["width"], -1.25)
        for width in (0.06, -4.0):
            draw.specs.clear()
            draw.clearance_box("n", "bounds", width=width)
            self.assertEqual(draw.specs[0]["header"]["width"], width)
            draw.specs.clear()
            draw.waypoints("n", "route", [(0, 0, 0)], width=width)
            self.assertEqual(next(s for s in draw.specs if s["id"] == "selection")["header"]["width"], width)
        for override in ({}, {"corner_frame": "false", "caller": "kept"}):
            draw.specs.clear()
            draw.detection3d("n", "det", [{"pose": (0, 0, 0), "size": (2, 4, 6)}], only_edge=True,
                             line_width=-3, props=override)
            box = draw.specs[0]
            self.assertEqual(box["header"]["width"], -3)
            self.assertEqual(box["props"]["corner_frame"], override.get("corner_frame", "true"))
            draw.specs.clear()
            draw.detection2d("n", "image", (0, 0, 0), [(320, 240, 100, 80)], hfov=1, vfov=.8,
                             image=(640, 480), props=override)
            box = draw.specs[0]
            self.assertEqual(box["header"]["width"], -1)
            self.assertEqual(len(points(box)), 4)
            self.assertEqual(box["props"]["corner_frame"], override.get("corner_frame", "true"))
            for key, value in override.items():
                self.assertEqual(box["props"][key], value)

    def test_detection_keeps_rigid_box_extents_and_clears_stale_brackets(self):
        draw = CaptureDraw()
        pose, size = (3, 4, 5, .3), (2, 4, 6)
        draw.detection3d("n", "detection", [{"pose": pose, "size": size, "class": "fixture", "score": .82, "track": 17}],
                         frame="robot/link", lifetime=7, only_edge=True)
        children = {s["id"]: s for s in draw.specs}
        self.assertEqual(children["brackets"]["op"], client.DRAW_OP["delete"])
        self.assertEqual(children["boxes"]["instances"][-3:], list(size))
        self.assertEqual(children["tags"]["header"]["kind"], client.DRAW_KIND["text"])
        self.assertEqual(json.loads(children["tags"]["labels"][0]), ["fixture", "score 0.82 / track #17"])
        self.assertEqual(children["tags"]["props"]["label_parts"], "identity-detail-json")
        self.assertEqual(children["tags"]["props"]["label_neutral_identity"], "true")
        self.assertEqual(children["tags"]["header"]["width"], -16)
        self.assertEqual(children["tags"]["props"]["label_icon"], "sensor")
        self.assertEqual(children["boxes"]["header"]["flags"] & client.DRAW_FLAG["additive"], 0)
        draw.specs.clear()
        draw.detection3d("n", "detection", [], frame="robot/link", lifetime=7)
        self.assertEqual({s["id"] for s in draw.specs}, set(children))
        self.assertTrue(all(s["op"] == client.DRAW_OP["delete"] for s in draw.specs))

    def test_legacy_composites_keep_explicit_effects_and_sizes(self):
        draw = CaptureDraw()
        common = {"glow": -1, "pulse_hz": -2, "flow_speed": -3, "width": -2.5,
                  "style": "plain", "frame": "robot/link", "lifetime": 7, "drape": False}
        draw.waypoints("n", "route", [(0, 0, 0), (2, 0, 0)], **common)
        draw.callout("n", "callout", (0, 0, 0), "Supplied text", **common)
        draw.frame("n", "frame", (0, 0, 0), parent=(1, 0, 0), **common)
        draw.detection3d("n", "detection", [{"pose": (0, 0, 0)}], **common)
        for s in draw.specs:
            if s["op"] == client.DRAW_OP["delete"]:
                continue
            h = s["header"]
            self.assertEqual((h["glow"], h["pulse_hz"], h["flow_speed"], h["width"]), (-1, -2, -3, -2.5))
            self.assertEqual(s["props"]["frame"], "robot/link")
            self.assertEqual(h["lifetime_s"], 7)
        anchor = next(s for s in draw.specs if s["id"] == "anchor")
        self.assertEqual(anchor["header"]["kind"], client.DRAW_KIND["lines"])
        self.assertEqual(len(points(anchor)), 8)

    def test_text_and_particle_defaults_defer_to_viewer_without_glow_override(self):
        draw = CaptureDraw()
        draw.text("n", "default", [(0, 0, 0)], text="Default")
        draw.text("n", "pixels", [(0, 0, 0)], text="Pixels", size=23)
        draw.text("n", "metres", [(0, 0, 0)], text="Metres", size_m=.4)
        draw.particles("n", "particles", [(0, 0, 0)], [1])
        self.assertEqual([s["header"]["width"] for s in draw.specs[:3]], [-16, -23, .4])
        self.assertTrue(all(s["header"]["glow"] == 0 for s in draw.specs))

    def test_annotation_metadata_is_supplied_not_inferred_from_text(self):
        draw = CaptureDraw()
        draw.text("n", "annotation", [(1, 2, 3)], text="Supplied caption", label_category="INSPECTION",
                  label_detail="0.4 m", label_icon="measurement", label_selected=True)
        props = draw.specs[0]["props"]
        self.assertEqual({k: props[k] for k in ("label_category", "label_detail", "label_icon", "label_selected")},
                         {"label_category": "INSPECTION", "label_detail": "0.4 m", "label_icon": "measurement", "label_selected": "true"})
        draw.text("n", "plain", [(0, 0, 0)], text="ROBOT / selected", label_icon="unknown")
        self.assertFalse(any(k.startswith("label_") for k in draw.specs[1]["props"]))
        draw.waypoints("n", "route", [(0, 0, 0)], label_selected=False)
        current = next(s for s in draw.specs if s["id"] == "current")
        self.assertEqual(current["props"]["label_selected"], "false")

    def test_detection_identity_detail_encoding_is_chunk_safe(self):
        draw = CaptureDraw()
        names = ["fixture\nupper", "value\x1fwith separator", 'quote " and \\ slash', "点検センサー"]
        draw.detection3d("n", "detections", [{"pose": (0, 0, 0), "class": name, "score": .5} for name in names])
        tags = next(s for s in draw.specs if s["id"] == "tags")
        self.assertEqual([json.loads(label)[0] for label in tags["labels"]], names)
        self.assertTrue(all("\n" not in label for label in tags["labels"]))
        self.assertEqual([json.loads(label)[1] for label in tags["labels"]], ["score 0.50"] * len(names))

    def test_panel_helpers_merge_optional_annotation_and_world_anchor_props(self):
        draw = CaptureDraw()
        draw.panel("n", "panel", (1, 2, 3), "table", rows=[("Source", "Fixture")],
                   props={"label_category": "INSPECTION", "label_accent": client.rgba("plan"), "panel_theme": "light"})
        draw.badge("n", "badge", "robot", "Fixture caption", props={"label_detail": "Supplied detail"})
        draw.legend("n", "legend", "viridis", 0, 1, pose=(4, 5, 6), drape=True,
                    props={"world_anchor": "true", "label_max_distance": 70})
        self.assertEqual(draw.specs[0]["props"]["rows"], "Source=Fixture")
        self.assertEqual(draw.specs[0]["props"]["label_category"], "INSPECTION")
        self.assertEqual(draw.specs[0]["props"]["panel_theme"], "light")
        self.assertEqual(draw.specs[1]["props"]["text"], "Fixture caption")
        self.assertEqual(draw.specs[1]["props"]["label_detail"], "Supplied detail")
        self.assertEqual(draw.specs[1]["props"]["frame"], "robot")
        self.assertEqual(draw.specs[2]["props"]["world_anchor"], "true")
        self.assertEqual(draw.specs[2]["props"]["label_max_distance"], "70")
        self.assertEqual([draw.specs[2]["header"][k] for k in ("px", "py", "pz")], [4, 5, 6])
        self.assertTrue(draw.specs[2]["header"]["flags"] & client.DRAW_FLAG["drape"])
        with patch.object(draw, "_deliver", return_value=client.Status()) as deliver:
            draw.sparkline("n", "spark", (1, 2, 3), .4, props={"label_category": "SAMPLE"})
            message = deliver.call_args.args[1][0]
            self.assertEqual(message.op, client.DRAW_OP["extend"])
            self.assertEqual(message.props["label_category"], "SAMPLE")

    def test_panel_finishes_preserve_caller_color_alpha_and_data(self):
        for finish in ("color", "white", "black"):
            draw = CaptureDraw()
            draw.panel("n", "panel", (1, 2, 3), "bars", values=[.25, .75], color="#cf72a880",
                       props={"panel_finish": finish, "panel_theme": "light"})
            spec = draw.specs[0]
            self.assertEqual(spec["props"]["panel_finish"], finish)
            self.assertEqual(spec["props"]["panel_theme"], "light")
            self.assertEqual(spec["header"]["color"], client.rgba("#cf72a880"))
            self.assertEqual(spec["values"], [.25, .75])

    def test_explicit_text_color_is_not_replaced_by_annotation_accent(self):
        draw = CaptureDraw()
        tint = client.rgba("magenta", .6)
        draw.text("n", "text", [(0, 0, 0)], text="Supplied", color=tint, props={"label_accent": client.rgba("plan")})
        draw.destination("n", "destination", color=tint, label="Supplied")
        draw.clearance_box("n", "bounds", color=tint, label="Supplied")
        labels = [s for s in draw.specs if s["header"]["kind"] == client.DRAW_KIND["text"]]
        self.assertEqual(len(labels), 3)
        for label in labels:
            self.assertEqual(label["header"]["color"], tint)
            self.assertNotEqual(label["props"].get("label_neutral_identity"), "true")

    def test_tintable_helpers_forward_explicit_colors_and_transparency(self):
        pts = [(0, 0, 0), (2, 0, 0), (2, 2, 0)]
        cases = [
            ("lines", lambda d, c: d.lines("n", "t", pts, color=c)),
            ("turns", lambda d, c: d.turns("n", "t", [pts], color=c)),
            ("contact", lambda d, c: d.contact("n", "t", pts[0], (0, 0, 1), color=c)),
            ("range", lambda d, c: d.range_cone("n", "t", pts[0], fov=1, min_range=0, max_range=3, range=1, color=c)),
            ("scan", lambda d, c: d.scan("n", "t", [1, 2], angle_min=0, angle_increment=.1, color=c)),
            ("ab", lambda d, c: d.ab_line("n", "t", pts[0], pts[1], color=c)),
            ("angle", lambda d, c: d.angle("n", "t", pts[0], 0, 1, color=c)),
            ("field", lambda d, c: d.field("n", "t", pts, color=c)),
            ("band", lambda d, c: d.band("n", "t", pts, [.1, .2, .1], color=c)),
            ("graph", lambda d, c: d.graph("n", "t", pts, [(0, 1)], color=c)),
            ("path", lambda d, c: d.path("n", "t", pts, pose_markers=1, marker="arrow", color=c)),
            ("odometry", lambda d, c: d.odometry("n", "t", pts[0], velocity=(1, 0, 0), color=c)),
            ("covariance", lambda d, c: d.covariance("n", "t", pts[0], [1, 0, 0, 1], color=c)),
            ("particles", lambda d, c: d.particles("n", "t", [pts[0], pts[1]], [.2, 1], color=c)),
            ("panel", lambda d, c: d.bars("n", "t", pts[0], [.2, .7], color=c)),
            ("ghost", lambda d, c: d.ghost("n", "t", "robot", [pts[0]], color=c)),
            ("destination", lambda d, c: d.destination("n", "t", color=c, label="Fixture")),
            ("clearance", lambda d, c: d.clearance_box("n", "t", color=c, label="Fixture")),
        ]
        for name, emit in cases:
            for color in ("magenta", "#00000000", 0, client.rgba("green", .5)):
                with self.subTest(helper=name, color=color):
                    draw = CaptureDraw()
                    with patch.object(draw, "delete", return_value=client.Status()):
                        emit(draw, color)
                    self.assertTrue(draw.specs)
                    supplied = client.rgba(color)
                    for spec in draw.specs:
                        if spec["op"] == client.DRAW_OP["delete"]:
                            continue
                        for actual in spec["colors"] or [spec["header"]["color"]]:
                            self.assertNotEqual(actual, 0)
                            self.assertLessEqual(actual >> 24, supplied >> 24)
                            self.assertEqual(actual & 0xFFFFFF, (supplied or 1) & 0xFFFFFF)

    def test_navigation_color_override_keeps_semantic_states(self):
        for tint in (client.rgba("magenta"), client.rgba("green", .5), 0):
            with self.subTest(tint=tint):
                draw = CaptureDraw()
                pts = [(0, 0, 0), (2, 0, 0), (4, 0, 0)]
                draw.waypoints("n", "waypoints", pts, current=1, color=tint)
                draw.swaths("n", "swaths", [pts, pts], done=1, color=tint)
                draw.compass("n", "compass", color=tint)
                by_id = {(s["ns"], s["id"]): s for s in draw.specs}
                route = by_id[("n/waypoints", "route")]["colors"]
                self.assertEqual(route[2], tint or 1)
                self.assertEqual(route[0] & 0xFFFFFF, client.rgba("white") & 0xFFFFFF)
                self.assertEqual(by_id[("n/waypoints", "current")]["header"]["color"] & 0xFFFFFF, client.rgba("goal") & 0xFFFFFF)
                self.assertEqual(by_id[("n/swaths", "todo")]["header"]["color"], tint or 1)
                self.assertEqual(by_id[("n/swaths", "done")]["header"]["color"] & 0xFFFFFF, client.rgba("history") & 0xFFFFFF)
                self.assertEqual(by_id[("n/compass", "letters")]["colors"][1:], [tint or 1] * 3)
                for s in draw.specs:
                    for color in s["colors"] or [s["header"]["color"]]:
                        if color:
                            self.assertLessEqual(color >> 24, tint >> 24)

    def test_explicit_palettes_and_scalar_maps_survive_theme_tint(self):
        draw = CaptureDraw()
        palette = [client.rgba("green"), client.rgba("orange"), 0]
        draw.bars("n", "bars", (0, 0, 0), [.2, .5, .8], colors=palette, color="magenta")
        draw.cloud("n", "cloud", [(0, 0, 0), (1, 0, 0)], values=[.1, .9], colormap="viridis", color="magenta")
        draw.legend("n", "legend", "viridis", 0, 1, color="magenta")
        draw.implement("n", "implement", 3, [True, False], color="magenta")
        draw.detection3d("n", "detection", [{"pose": (0, 0, 0), "class": "fixture", "score": .5}],
                         class_colors={"fixture": 0}, color="magenta")
        self.assertEqual(draw.specs[0]["colors"], [*palette[:2], 1])
        self.assertEqual(draw.specs[0]["values"], [.2, .5, .8])
        self.assertEqual(draw.specs[1]["header"]["colormap"], client.COLORMAP["viridis"])
        self.assertEqual(draw.specs[1]["values"], [.1, .9])
        self.assertEqual(draw.specs[2]["header"]["colormap"], client.COLORMAP["viridis"])
        self.assertEqual(draw.specs[3]["colors"], [client.rgba("green"), client.rgba("grey", .5)])
        tags = next(s for s in draw.specs if s["id"] == "tags")
        boxes = next(s for s in draw.specs if s["id"] == "boxes")
        self.assertEqual(tags["colors"], [1])
        self.assertEqual(boxes["colors"], [1])

    def test_default_and_explicit_zero_are_distinct_and_updates_preserve_alpha(self):
        draw = CaptureDraw()
        draw.lines("n", "default", [(0, 0, 0), (1, 0, 0)])
        draw.lines("n", "clear", [(0, 0, 0), (1, 0, 0)], color=0)
        self.assertEqual([s["header"]["color"] for s in draw.specs], [0, 1])
        draw.lines("n", "clear-outline", [(0, 0, 0), (1, 0, 0)], color2=0)
        self.assertEqual(draw.specs[-1]["header"]["color2"], 1)
        with patch.object(draw, "_deliver", return_value=client.Status()) as deliver:
            draw.update("n", "clear", color="#00000000")
            self.assertEqual(deliver.call_args.args[1][0].color, 1)

    def test_callout_neutral_identity_tracks_caller_opacity_and_accent(self):
        for tint in (client.rgba("magenta"), client.rgba("green", .5), 0):
            draw = CaptureDraw()
            draw.callout("n", "callout", (0, 0, 0), "Supplied", color=tint)
            leader, anchor, label = draw.specs
            self.assertEqual([s["header"]["color"] for s in (leader, anchor)], [tint or 1] * 2)
            self.assertEqual(label["header"]["color"] >> 24, tint >> 24)
            self.assertEqual(label["header"]["color"] & 0xFFFFFF, 0xFFFFFF)
            self.assertEqual(label["props"]["accent"], str(tint or 1))

    def test_updates_select_only_supplied_scalars_including_zero(self):
        draw = CaptureDraw()
        cases = [
            ("color", "tint", "color", 1), ("color2", "color2", "color2", 1),
            ("glow", "glow", "glow", 0), ("pulse_hz", "pulse_hz", "pulse_hz", 0),
            ("flow_speed", "flow_speed", "flow_speed", 0), ("spacing", "dash", "dash", 0),
            ("beam_height", "beam_height", "beam_height", 0), ("fade", "fade", "fade", 0),
            ("urgency", "urgency", "urgency", 0), ("opacity", "opacity", "opacity", 0),
            ("colormap", "palette", "colormap", 0),
            ("vmin", "vmin", "vmin", 0), ("vmax", "vmax", "vmax", 0),
        ]
        with patch.object(draw, "_deliver", return_value=client.Status()) as deliver:
            for arg, mask, field, expected in cases:
                with self.subTest(arg=arg):
                    draw.update("n", "id", **{arg: 0})
                    msg = deliver.call_args.args[1][0]
                    self.assertEqual(msg.props["set"], mask)
                    self.assertEqual(getattr(msg, field), expected)
            draw.update("n", "id", glow=-1, flow_speed=2.5, panel_finish="white")
            msg = deliver.call_args.args[1][0]
            self.assertEqual(msg.props["set"], "glow,flow_speed")
            self.assertEqual((msg.glow, msg.flow_speed), (-1, 2.5))

    def test_blur_is_common_metadata_and_zero_is_an_explicit_update(self):
        draw = CaptureDraw()
        draw.lines("n", "line", [(0, 0), (1, 0)], blur=.6, glow=.2)
        draw.shapes("n", "sphere", "ellipsoid", [(0, 0, 1)], blur=.6)
        draw.table("n", "table", (0, 0, 1), [("Name", "Value")], blur=.6)
        for spec in draw.specs:
            self.assertEqual(float(spec["props"]["blur"]), .6)
            self.assertNotIn("blur", spec["header"])
        draw.update("n", "line", blur=0)
        self.assertEqual(float(draw.specs[-1]["props"]["blur"]), 0)
        self.assertEqual(draw.specs[-1]["props"]["set"], "props")
        for value in (-.1, 1.1, float("nan"), float("inf")):
            with self.assertRaisesRegex(ValueError, "blur"):
                draw.lines("n", "bad", [(0, 0), (1, 0)], blur=value)
            with self.assertRaisesRegex(ValueError, "blur"):
                draw.update("n", "bad", blur=value)

    def test_metadata_only_and_empty_updates_do_not_replace_headers(self):
        draw = CaptureDraw()
        with patch.object(draw, "_deliver", return_value=client.Status()) as deliver:
            for props in ({}, {"panel_finish": "white"}, {"set": "", "panel_theme": "light"}):
                draw.update("n", "id", **props)
                msg = deliver.call_args.args[1][0]
                self.assertEqual(msg.props["set"], "props")
            with self.assertRaisesRegex(ValueError, "alpha requires color"):
                draw.update("n", "id", alpha=.5)

    def test_ghost_per_pose_colors_follow_sampling_and_keep_explicit_zero(self):
        draw = CaptureDraw()
        palette = ["green", "orange", 0, "cyan", "magenta"]
        poses = [(i, 0, 0) for i in range(5)]
        draw.ghost_trail("n", "ghost", "robot", poses, step=3, colors=palette, color="white", fade=-1)
        spec = draw.specs[0]
        self.assertEqual(spec["instances"][::10], [0, 3, 4])
        self.assertEqual(spec["colors"], [client.rgba(palette[i]) for i in (0, 3, 4)])
        self.assertEqual(spec["header"]["fade"], -1)
        draw.ghost("n", "clear", "robot", [poses[0]], colors=[0])
        self.assertEqual(draw.specs[1]["colors"], [1])


if __name__ == "__main__":
    unittest.main()
