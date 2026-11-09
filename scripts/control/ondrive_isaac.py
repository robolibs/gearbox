"""ondrive controllers driving the Gearbox Husky package in Isaac Sim: the
lab's scenarios, machine model, tracker set-up and scoring, with Isaac's
PhysX in place of Gearbox. Obstacles are kinematic cylinders, the movers
driven along their courses.

Runs inside Kit:

    isaacsim --no-ros-env --exec scripts/control/ondrive_isaac.py

    ONDRIVE_RUNS  controller:scenario pairs, comma separated
                  (default: the path followers on s_curve, the avoiders on head_on)
    ONDRIVE_OUT   results JSON (default: target/lab/isaac_husky.json)
    HUSKY_USD     machine package (default: assets/machines/usd/clearpath_husky.usdz)
    ONDRIVE_STAY  set to keep Isaac open after the runs (default: quit)
    ONDRIVE_WHEELS  sphere: one-point sphere contacts in place of the tyres'
                  convex hulls; mesh: as packaged (default: sphere)
    ONDRIVE_FRONT_MU  friction of the front sphere contacts (default: 0.3)
"""

import asyncio
import json
import math
import os
import sys
import time
import traceback

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
GEARBOX = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(GEARBOX, ".python-packages"))

import ondrive  # noqa: E402
from ondrive_lab import (  # noqa: E402
    MACHINES, PRED_DT, TICK, Reference, box_clearance, densify, predicted, scenario, tracker_for, wrap,
)

USD = os.environ.get("HUSKY_USD", os.path.join(GEARBOX, "assets/machines/usd/clearpath_husky.usdz"))
OUT = os.environ.get("ONDRIVE_OUT", os.path.join(GEARBOX, "target/lab/isaac_husky.json"))
WHEELS = os.environ.get("ONDRIVE_WHEELS", "sphere")
FRONT_MU = float(os.environ.get("ONDRIVE_FRONT_MU", "0.3"))
FRONT_MATERIAL = "/World/frontContact"
DEFAULT_RUNS = ",".join(
    [f"{k}:s_curve" for k in ("pure_pursuit", "regulated_pursuit", "stanley", "lqr", "kanayama", "mpc", "ilqr",
                               "mppi", "mca", "soc", "teb", "dwa")]
    + [f"{k}:head_on" for k in ("mca", "mppi", "mpc", "ilqr", "soc", "dwa", "apf", "teb")]
)
RUNS = [tuple(r.split(":")) for r in os.environ.get("ONDRIVE_RUNS", DEFAULT_RUNS).split(",") if ":" in r]

SPAWN = "/World/spawn"
ROOT = SPAWN + "/husky"
OBSTACLES = "/World/obstacles"
MARKERS = 8
WHEEL_RADIUS = 0.1651
TRACK_WIDTH = 0.5708
PHYSICS_HZ = 120.0
SETTLE_S = 1.5


def say(line):
    print(f"[ondrive] {line}", flush=True)


def arr(a):
    return a.numpy() if hasattr(a, "numpy") else np.asarray(a)


def heading(q_wxyz):
    from pxr import Gf

    w, x, y, z = (float(c) for c in q_wxyz)
    forward = Gf.Rotation(Gf.Quatd(w, Gf.Vec3d(x, y, z))).TransformDir(Gf.Vec3d(0, -1, 0))
    return math.atan2(forward[1], forward[0])


def sphere_wheels(stage, husky):
    """Swaps each tyre's convex-hull collider for a sphere of the wheel's
    radius at the tyre's centre, the rear ones with the tyre's physics
    material and the front ones with `FRONT_MU`, and raises the
    articulation's solver iterations."""
    from pxr import Gf, PhysxSchema, Usd, UsdGeom, UsdPhysics, UsdShade

    cache = UsdGeom.BBoxCache(0, [UsdGeom.Tokens.default_, UsdGeom.Tokens.render, UsdGeom.Tokens.guide])
    front = UsdShade.Material.Define(stage, FRONT_MATERIAL)
    front_physics = UsdPhysics.MaterialAPI.Apply(front.GetPrim())
    front_physics.CreateStaticFrictionAttr(FRONT_MU)
    front_physics.CreateDynamicFrictionAttr(FRONT_MU)
    front_physics.CreateRestitutionAttr(0.0)
    PhysxSchema.PhysxMaterialAPI.Apply(front.GetPrim()).CreateFrictionCombineModeAttr("min")
    for prim in Usd.PrimRange(husky):
        if prim.GetName() != "tire_collider" or not prim.HasAPI(UsdPhysics.CollisionAPI):
            continue
        wheel = prim.GetParent()
        centre = cache.ComputeRelativeBound(prim, wheel).ComputeAlignedRange().GetMidpoint()
        UsdPhysics.CollisionAPI(prim).GetCollisionEnabledAttr().Set(False)
        sphere = UsdGeom.Sphere.Define(stage, wheel.GetPath().AppendChild("contact_sphere"))
        sphere.CreateRadiusAttr(WHEEL_RADIUS)
        sphere.AddTranslateOp().Set(Gf.Vec3d(*centre))
        sphere.CreatePurposeAttr(UsdGeom.Tokens.guide)
        UsdPhysics.CollisionAPI.Apply(sphere.GetPrim())
        binding = UsdShade.MaterialBindingAPI(prim).GetDirectBindingRel("physics").GetTargets()
        if "front" in wheel.GetName():
            UsdShade.MaterialBindingAPI.Apply(sphere.GetPrim()).Bind(front, UsdShade.Tokens.weakerThanDescendants, "physics")
        elif binding:
            material = UsdShade.Material(stage.GetPrimAtPath(binding[0]))
            UsdShade.MaterialBindingAPI.Apply(sphere.GetPrim()).Bind(material, UsdShade.Tokens.weakerThanDescendants, "physics")
    articulation = PhysxSchema.PhysxArticulationAPI.Apply(husky)
    articulation.CreateSolverPositionIterationCountAttr(32)
    articulation.CreateSolverVelocityIterationCountAttr(4)


def build_world(stage):
    from pxr import Gf, UsdGeom, UsdLux, UsdPhysics, UsdShade

    UsdGeom.SetStageUpAxis(stage, UsdGeom.Tokens.z)
    UsdGeom.SetStageMetersPerUnit(stage, 1.0)
    UsdPhysics.SetStageKilogramsPerUnit(stage, 1.0)
    world = UsdGeom.Xform.Define(stage, "/World")
    stage.SetDefaultPrim(world.GetPrim())
    scene = UsdPhysics.Scene.Define(stage, "/World/physicsScene")
    scene.CreateGravityDirectionAttr(Gf.Vec3f(0, 0, -1))
    scene.CreateGravityMagnitudeAttr(9.81)

    ground = UsdGeom.Cube.Define(stage, "/World/ground")
    ground.CreateSizeAttr(1.0)
    ground.AddTranslateOp().Set(Gf.Vec3d(0, 0, -0.1))
    ground.AddScaleOp().Set(Gf.Vec3f(120, 120, 0.2))
    ground.CreateDisplayColorAttr([Gf.Vec3f(0.35, 0.37, 0.33)])
    UsdPhysics.CollisionAPI.Apply(ground.GetPrim())
    material = UsdShade.Material.Define(stage, "/World/groundMaterial")
    physics_material = UsdPhysics.MaterialAPI.Apply(material.GetPrim())
    physics_material.CreateStaticFrictionAttr(1.0)
    physics_material.CreateDynamicFrictionAttr(0.8)
    physics_material.CreateRestitutionAttr(0.0)
    UsdShade.MaterialBindingAPI.Apply(ground.GetPrim()).Bind(material, UsdShade.Tokens.weakerThanDescendants, "physics")

    sun = UsdLux.DistantLight.Define(stage, "/World/sun")
    sun.CreateIntensityAttr(3000)
    UsdGeom.Xformable(sun).AddRotateXYZOp().Set(Gf.Vec3f(-40, 20, 0))
    UsdLux.DomeLight.Define(stage, "/World/sky").CreateIntensityAttr(800)

    markers = []
    for k in range(MARKERS):
        marker = UsdGeom.Cylinder.Define(stage, f"{OBSTACLES}/o_{k}")
        marker.CreateRadiusAttr(0.6)
        marker.CreateHeightAttr(0.5)
        marker.CreateAxisAttr("Z")
        marker.CreateDisplayColorAttr([Gf.Vec3f(0.9, 0.2, 0.3)])
        marker.AddTranslateOp().Set(Gf.Vec3d(0, 0, -5.0))
        markers.append(marker)

    spawn = UsdGeom.Xform.Define(stage, SPAWN)
    husky = stage.DefinePrim(ROOT, "Xform")
    husky.GetReferences().AddReference(USD)
    if WHEELS == "sphere":
        sphere_wheels(stage, husky)
    purposes = [UsdGeom.Tokens.default_, UsdGeom.Tokens.render, UsdGeom.Tokens.guide]
    low = UsdGeom.BBoxCache(0, purposes).ComputeWorldBound(husky).ComputeAlignedRange().GetMin()[2]
    spawn.AddTranslateOp().Set(Gf.Vec3d(0, 0, 0.01 - low))
    return markers


async def main():
    import omni.kit.app
    import omni.timeline
    import omni.usd
    from isaacsim.core.experimental.prims import Articulation
    from isaacsim.core.experimental.utils.backend import use_backend
    from isaacsim.core.rendering_manager import ViewportManager
    from isaacsim.core.simulation_manager import SimulationManager
    from isaacsim.robot.experimental.wheeled_robots.controllers import DifferentialController
    from pxr import Gf

    app = omni.kit.app.get_app()
    for _ in range(10):
        await app.next_update_async()
    context = omni.usd.get_context()
    await context.new_stage_async()
    for _ in range(3):
        await app.next_update_async()
    markers = build_world(context.get_stage())
    marker_at = [mk.GetPrim().GetAttribute("xformOp:translate") for mk in markers]
    marker_radius = [mk.GetPrim().GetAttribute("radius") for mk in markers]

    def place(k, x, y, r):
        if k < len(marker_at):
            marker_at[k].Set(Gf.Vec3d(x, y, 0.25))
            marker_radius[k].Set(r)
    for _ in range(10):
        await app.next_update_async()

    import carb.settings

    carb.settings.get_settings().set_bool("/physics/collisionApproximateCylinders", True)
    SimulationManager.set_physics_sim_device("cpu")
    SimulationManager.set_physics_dt(1.0 / PHYSICS_HZ)
    timeline = omni.timeline.get_timeline_interface()
    articulation = Articulation(ROOT)
    timeline.play()
    for _ in range(15):
        await app.next_update_async()
    if not articulation.is_physics_tensor_entity_valid():
        articulation = Articulation(ROOT)
        for _ in range(5):
            await app.next_update_async()
    names = list(articulation.dof_names)
    left = [i for i, n in enumerate(names) if n.endswith("_left")]
    right = [i for i, n in enumerate(names) if n.endswith("_right")]
    wheels = DifferentialController(wheel_radius=WHEEL_RADIUS, wheel_base=TRACK_WIDTH)
    camera = "/OmniverseKit_Persp"

    def pose():
        with use_backend("tensor"):
            p, q = articulation.get_world_poses()
        return arr(p)[0], arr(q)[0]

    def velocity():
        with use_backend("tensor"):
            lin, ang = articulation.get_velocities()
        return arr(lin)[0], arr(ang)[0]

    def drive(v, w):
        target = wheels.forward(np.array([v, w]))
        targets = [0.0] * len(names)
        for i in left:
            targets[i] = float(target[0])
        for i in right:
            targets[i] = float(target[1])
        with use_backend("tensor"):
            articulation.set_dof_velocity_targets(targets)

    start_p, start_q = pose()

    def reset():
        with use_backend("tensor"):
            articulation.set_world_poses(positions=[start_p.tolist()], orientations=[start_q.tolist()])
            articulation.set_velocities(linear_velocities=[[0.0, 0.0, 0.0]], angular_velocities=[[0.0, 0.0, 0.0]])
            articulation.set_dof_velocities([0.0] * len(names))

    m = MACHINES["husky"]
    results = []
    for kind, sname in RUNS:
        reset()
        drive(0.0, 0.0)
        for at in marker_at:
            at.Set(Gf.Vec3d(0, 0, -5.0))
        t_settle = timeline.get_current_time() + SETTLE_S
        while timeline.get_current_time() < t_settle:
            await app.next_update_async()
        p0, q0 = pose()
        ox, oy, oyaw = float(p0[0]), float(p0[1]), heading(q0)
        c, s = math.cos(oyaw), math.sin(oyaw)
        here = lambda x, y: (ox + c * x - s * y, oy + s * x + c * y)  # noqa: E731
        sc = scenario(sname, m.scale)
        pts = [here(x, y) for x, y in sc.points]
        path = ondrive.Path(is_closed=sc.closed)
        for i, (x, y) in enumerate(pts):
            nx, ny = pts[min(i + 1, len(pts) - 1)]
            px, py = pts[max(i - 1, 0)]
            path.add_waypoint_xy(x, y, yaw=math.atan2(ny - py, nx - px))
        path.smoothen(0.2)
        ref = Reference(densify(pts, 0.1), sc.closed)
        t = tracker_for(kind, m, sc, {})
        t.set_path(path)
        statics = [(*here(x, y), r) for x, y, r in sc.static]
        movers = []
        for mv in sc.movers:
            gx, gy = here(*mv["start"][:2])
            movers.append({"spec": mv, "at": [gx, gy, wrap(oyaw + mv["start"][2])], "going": mv.get("meet") is None})
        for k, (sx, sy, r) in enumerate(statics):
            place(k, sx, sy, r)
        limit = ref.lap_len / (0.5 * 0.8 * m.vmax) + 15.0
        t0 = timeline.get_current_time()
        last_tick, errs, clear, status, flips, last_w = t0, [], float("inf"), "timeout", 0, 0.0
        log = []
        lost_since = None
        while True:
            await app.next_update_async()
            now = timeline.get_current_time()
            if now - t0 > limit:
                break
            if now - last_tick < TICK:
                continue
            dt, last_tick = now - last_tick, now
            p, q = pose()
            lin, ang = velocity()
            x, y, yaw = float(p[0]), float(p[1]), heading(q)
            v_fwd = float(lin[0] * math.cos(yaw) + lin[1] * math.sin(yaw))
            world = ondrive.World()
            obs = [(sx, sy, 0.0, 0.0, r) for sx, sy, r in statics]
            for k, mv in enumerate(movers):
                spec, (ix, iy, iyaw) = mv["spec"], mv["at"]
                if not mv["going"]:
                    mx, my = here(*spec["meet"])
                    mv["going"] = math.hypot(mx - ix, my - iy) / spec["speed"] + 1.2 >= math.hypot(mx - x, my - y) / max(abs(v_fwd), 0.24)
                speed = spec["speed"] if mv["going"] else 0.0
                ivx, ivy = speed * math.cos(iyaw), speed * math.sin(iyaw)
                obs.append((ix, iy, ivx, ivy, 0.6))
                mv["at"] = [ix + ivx * dt, iy + ivy * dt, iyaw]
                place(len(statics) + k, ix, iy, 0.6)
            now_clear = float("inf")
            for i, o in enumerate(obs):
                world.add_obstacle(i, o[4], [predicted(o)], dt=PRED_DT)
                now_clear = min(now_clear, box_clearance(x, y, yaw, m.length, m.width, o[0], o[1], o[4], m.body_offset))
            clear = min(clear, now_clear)
            state = ondrive.RobotState(pose=((x, y, float(p[2])), yaw), velocity=(v_fwd, float(ang[2]), 0.0), allow_move=True)
            cmd = t.tick(state, dt, world)
            v, w = (cmd.linear_velocity, cmd.angular_velocity) if cmd.valid else (0.0, 0.0)
            drive(v, w)
            if abs(w) > 0.2 * m.wmax and last_w * w < 0:
                flips += 1
            if abs(w) > 0.2 * m.wmax:
                last_w = w
            err = ref.error(x, y)
            errs.append(err)
            log.append([round(now - t0, 2), round(x - ox, 3), round(y - oy, 3), round(wrap(yaw - oyaw), 3), round(v_fwd, 3),
                        round(float(ang[2]), 3), round(v, 3), round(w, 3), round(err, 3),
                        None if now_clear == float("inf") else round(now_clear, 3), cmd.status_message])
            back = np.array([math.cos(yaw), math.sin(yaw), 0.0])
            ViewportManager.set_camera_view(camera, eye=(p - 4.0 * back + np.array([0, 0, 2.5])).tolist(),
                                            target=(p + 1.5 * back + np.array([0, 0, 0.2])).tolist())
            if t.is_path_completed():
                status = "completed"
                break
            lost_since = (lost_since or now) if err > max(3.0, 2.0 * m.length) else None
            if lost_since is not None and now - lost_since > 3.0:
                status = f"lost {err:.1f} m"
                break
        drive(0.0, 0.0)
        elapsed = timeline.get_current_time() - t0
        rms = math.sqrt(sum(e * e for e in errs) / max(1, len(errs)))
        row = {"controller": kind, "scenario": sname, "status": status, "time": round(elapsed, 1),
               "cte_rms": round(rms, 3), "cte_max": round(max(errs, default=0.0), 3),
               "min_clearance": None if clear == float("inf") else round(clear, 3),
               "flips_per_min": round(flips / max(elapsed / 60.0, 1e-9), 1), "log": log}
        results.append(row)
        say(f"isaac husky {kind:18} {sname:10} {status:12} {row['time']:6.1f} rms {row['cte_rms']:5.2f} "
            f"max {row['cte_max']:5.2f} clear {row['min_clearance']} flips {row['flips_per_min']}")
        os.makedirs(os.path.dirname(OUT), exist_ok=True)
        with open(OUT, "w") as f:
            json.dump(results, f, indent=1)
    say(f"all {len(results)} runs done; results in {OUT}")


async def guarded():
    try:
        await main()
    except Exception:
        say("error:\n" + traceback.format_exc())
    if os.environ.get("ONDRIVE_STAY", "") == "":
        import omni.kit.app

        omni.kit.app.get_app().post_quit()


asyncio.ensure_future(guarded())
