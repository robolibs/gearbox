import math
from pathlib import Path
import sys
import types
import unittest
import tempfile
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts/control"))
import draw_mission as mission
from test_draw_world import CaptureDraw


class MissionTests(unittest.TestCase):
    def test_runtime_uses_async_error_property_and_releases_control(self):
        for heading in (0, float("nan")):
            calls = []
            robot = types.SimpleNamespace(
                claim=lambda **kw: calls.append("claim"),
                state=lambda **kw: types.SimpleNamespace(x=0, y=0, heading_rad=heading, linear_speed=0),
                cmd_vel=lambda v, w: types.SimpleNamespace(ok=True),
                stop=lambda: calls.append("stop"), release=lambda: calls.append("release"))
            gb = types.SimpleNamespace(wait_ready=lambda: None, machine=lambda *a, **kw: robot,
                                       play=lambda: None, draw=CaptureDraw())
            module = types.SimpleNamespace(Gearbox=lambda instance: gb)
            with tempfile.TemporaryDirectory() as directory, patch.dict(sys.modules, {"gearbox_client": module}), \
                    patch.object(mission.signal, "signal"), patch("builtins.print"):
                if math.isfinite(heading):
                    mission.run("draw-mission-test", "mission_husky", Path(directory), .06)
                    self.assertTrue((Path(directory) / "ready.json").is_file())
                else:
                    with self.assertRaisesRegex(RuntimeError, "non-finite odometry"):
                        mission.run("draw-mission-test", "mission_husky", Path(directory), .06)
                self.assertFalse(gb.draw.wait)
                self.assertEqual(calls, ["claim", "stop", "release"])

    def test_route_starts_at_robot_and_closes_smoothly(self):
        points = mission.route((3, -7))
        self.assertAlmostEqual(points[0][0], 3)
        self.assertAlmostEqual(points[0][1], -7)
        segments = [math.dist(points[i], points[(i + 1) % len(points)]) for i in range(len(points))]
        self.assertLess(max(segments), .35)
        self.assertGreater(min(segments), .2)

    def test_pursuit_completes_multiple_laps_with_bounded_commands(self):
        points = mission.route((0, 0))
        x = y = heading = 0.0
        laps, previous, worst = 0, 0, 0
        for _ in range(2200):
            index, target, error, speed, yaw = mission.steering(points, x, y, heading)
            self.assertGreaterEqual(speed, 0)
            self.assertLessEqual(speed, .95)
            self.assertLessEqual(abs(yaw), .75)
            self.assertNotEqual(index, target)
            worst = max(worst, error)
            if previous > 96 and index < 32:
                laps += 1
            previous = index
            x += speed * math.cos(heading) * .05
            y += speed * math.sin(heading) * .05
            heading += yaw * .05
        self.assertGreaterEqual(laps, 2)
        self.assertLess(worst, .3)

    def test_large_heading_error_turns_before_driving(self):
        *_, speed, yaw = mission.steering(mission.route((0, 0)), 0, 0, math.pi)
        self.assertEqual(speed, 0)
        self.assertEqual(abs(yaw), .65)

    def test_forecast_uses_supplied_command_not_fake_sensor_data(self):
        straight = mission.forecast(1, 2, 0, .5, 0)
        self.assertEqual(straight[0], (1, 2, .22))
        self.assertEqual(straight[-1], (2, 2, .22))
        turn = mission.forecast(1, 2, 0, 1, .5)
        self.assertAlmostEqual(turn[-1][0], 1 + 2 * math.sin(1))
        self.assertAlmostEqual(turn[-1][1], 2 + 2 * (1 - math.cos(1)))

    def test_drawings_use_live_data_frames_and_explicit_colors(self):
        draw = CaptureDraw()
        origin = (0, 0)
        points = mission.route(origin)
        state = types.SimpleNamespace(x=.2, y=.1, heading_rad=.05, linear_speed=.72)
        mission.draw_static(draw, "mission_husky", points, origin)
        mission.draw_trail(draw, "mission_husky", [(0, 0, .06), (.2, .1, .06)])
        mission.draw_live(draw, "mission_husky", points, origin, state,
                          mission.steering(points, state.x, state.y, state.heading_rad),
                          [.4, .72], 4.6, 2)
        specs = {item["id"]: item for item in draw.specs}
        self.assertIn("0.72 m/s", specs["telemetry"]["props"]["rows"])
        self.assertIn("Completed laps=2", specs["telemetry"]["props"]["rows"])
        self.assertEqual(specs["speed-history"]["values"], [.4, .72])
        self.assertEqual(specs["robot"]["props"]["frame"], "mission_husky")
        self.assertTrue(all(item["props"]["frame"] == "mission_husky/odom" for item in draw.specs if item["id"] != "robot"))
        self.assertEqual(specs["telemetry"]["props"]["panel_finish"], "white")
        self.assertEqual(specs["speed-history"]["props"]["panel_finish"], "black")
        self.assertNotEqual(specs["actual-trail"]["header"]["color"], specs["planned-route"]["header"]["color"])
        trail = specs["actual-trail"]["header"]
        self.assertEqual(trail["beam_height"], .45)
        self.assertEqual(trail["width"], .028)
        self.assertEqual(trail["pulse_hz"], .5)
        self.assertEqual(trail["fade"], .12)
        self.assertGreater(trail["glow"], .5)
        self.assertLess(trail["glow"], .8)
        self.assertEqual(mission.TRAIL_POINTS, 160)


if __name__ == "__main__":
    unittest.main()
