"""Isaac side of the robot-contract conformance comparison.

Runs inside Kit: `isaacsim --no-window --no-ros-env --exec isaac_probe.py`.
Arguments come from the environment:

    PROBE_SCHEDULE  command schedule JSON (names the fixture, overlay, root, tool)
    PROBE_OUT       output directory
    PROBE_TIMEOUT   wall-clock limit in seconds (default 300)

Writes preflight.json, overlay.json, structure.json, trace.jsonl and
status.json. Exit codes: 0 completed, 2 startup failure, 3 absent feature,
4 rejected asset, 5 runtime error, 6 timeout.
"""

import asyncio
import hashlib
import json
import math
import os
import sys
import threading
import time
import traceback

EXIT = {"completed": 0, "startup_failure": 2, "absent_feature": 3, "rejected_asset": 4, "runtime_error": 5, "timeout": 6}
OUT = os.environ.get("PROBE_OUT", "")
STARTED = time.time()


def write(name, value):
    with open(os.path.join(OUT, name), "w") as f:
        json.dump(value, f, indent=1, sort_keys=True, default=str)


def finish(result, **extra):
    status = {"result": result, "exit_code": EXIT[result], "wall_s": round(time.time() - STARTED, 3), **extra}
    try:
        write("status.json", status)
    finally:
        print(f"[robot-contract] isaac probe {result}", flush=True)
        os._exit(EXIT[result])


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def watchdog(seconds):
    def fire():
        time.sleep(seconds)
        finish("timeout", error=f"probe exceeded {seconds} s")

    threading.Thread(target=fire, daemon=True).start()


def preflight():
    import omni.kit.app
    from pxr import Usd

    app = omni.kit.app.get_app()
    root = os.environ.get("ISAAC_PATH") or os.path.dirname(os.path.dirname(os.path.abspath(sys.executable)))
    version = None
    for candidate in (root, "/pkg/isaacsim/6.2.0", os.environ.get("CARB_APP_PATH", "")):
        path = os.path.join(candidate, "VERSION") if candidate else ""
        if path and os.path.isfile(path):
            version = open(path).read().strip()
            break
    manager = app.get_extension_manager()
    wanted = ("omni.physics", "omni.physx", "omni.usd.schema", "isaacsim.core", "isaacsim.robot.schema", "omni.usd")
    extensions = sorted(
        f"{e['name']}-{e['version'] if isinstance(e['version'], str) else '.'.join(map(str, e['version'][:3]))}"
        for e in manager.get_extensions()
        if e.get("enabled") and e["name"].startswith(wanted)
    )
    return {
        "isaac_version": version,
        "kit_version": app.get_build_version(),
        "usd_version": list(Usd.GetVersion()),
        "python": sys.version.split()[0],
        "executable": sys.executable,
        "enabled_extensions": extensions,
    }


def overlay_differences(mechanical, overlay):
    """Every spec the overlay authors, and whether any of it touches physics."""
    from pxr import Sdf

    if not overlay:
        return {"overlay": None, "specs": [], "mechanical_changes": []}
    layer = Sdf.Layer.FindOrOpen(overlay)
    specs = []

    def visit(path):
        spec = layer.GetObjectAtPath(path)
        if isinstance(spec, Sdf.PrimSpec):
            schemas = spec.GetInfo("apiSchemas") if spec.HasInfo("apiSchemas") else None
            added = list(schemas.prependedItems) + list(schemas.appendedItems) + list(schemas.explicitItems) if schemas else []
            specs.append({"path": str(path), "kind": "prim", "specifier": str(spec.specifier), "api_schemas": added})
        elif isinstance(spec, Sdf.AttributeSpec):
            specs.append({"path": str(path), "kind": "attribute", "value": spec.default})
        elif isinstance(spec, Sdf.RelationshipSpec):
            specs.append({"path": str(path), "kind": "relationship", "targets": [str(t) for t in spec.targetPathList.GetAddedOrExplicitItems()]})

    layer.Traverse(Sdf.Path.absoluteRootPath, visit)
    physics = ("physics:", "drive:", "limit:", "physxJoint:", "physxArticulation:", "physxRigidBody:")
    changes = [
        s
        for s in specs
        if (s["kind"] != "prim" and s["path"].rsplit(".", 1)[-1].startswith(physics))
        or any(a.startswith(("Physics", "Physx")) for a in s.get("api_schemas", []))
    ]
    return {"overlay": os.path.basename(overlay), "overlay_sha256": sha256(overlay), "specs": specs, "mechanical_changes": changes}


def usd_structure(stage):
    """Bodies, joints and sites read with the installed USD runtime, in SI."""
    from pxr import Gf, Usd, UsdGeom, UsdPhysics

    mpu = UsdGeom.GetStageMetersPerUnit(stage)
    kpu = UsdPhysics.GetStageKilogramsPerUnit(stage)
    cache = UsdGeom.XformCache(Usd.TimeCode.Default())

    def pose(prim):
        m = cache.GetLocalToWorldTransform(prim)
        t = m.ExtractTranslation()
        q = m.ExtractRotationQuat()
        r = [q.GetReal(), *q.GetImaginary()]
        if r[0] < 0:
            r = [-c for c in r]
        return {"translation": [t[0] * mpu, t[1] * mpu, t[2] * mpu], "rotation_wxyz": r}

    def get(prim, name, default=None):
        attr = prim.GetAttribute(name)
        if attr and attr.HasAuthoredValue():
            v = attr.Get()
            return list(v) if isinstance(v, (Gf.Vec3f, Gf.Vec3d)) else v
        return default

    bodies, joints, sites = [], [], []
    body_paths = set()
    for prim in stage.Traverse():
        if prim.HasAPI(UsdPhysics.RigidBodyAPI):
            body_paths.add(str(prim.GetPath()))
            inertia = get(prim, "physics:diagonalInertia")
            bodies.append({
                "path": str(prim.GetPath()),
                "mass": None if get(prim, "physics:mass") is None else get(prim, "physics:mass") * kpu,
                "diagonal_inertia": None if inertia is None else [v * kpu * mpu * mpu for v in inertia],
                "initial_pose": pose(prim),
            })
    for prim in stage.Traverse():
        if not prim.IsA(UsdPhysics.Joint):
            continue
        joint = UsdPhysics.Joint(prim)
        kind = prim.GetTypeName()
        rotational = kind == "PhysicsRevoluteJoint"
        token = "angular" if rotational else "linear"
        targets = lambda rel: [str(t) for t in rel.GetTargets()] if rel else []
        record = {
            "path": str(prim.GetPath()),
            "type": kind,
            "axis": get(prim, "physics:axis"),
            "body0": targets(joint.GetBody0Rel()),
            "body1": targets(joint.GetBody1Rel()),
            "local_pos0": [v * mpu for v in (get(prim, "physics:localPos0") or [0, 0, 0])],
            "local_pos1": [v * mpu for v in (get(prim, "physics:localPos1") or [0, 0, 0])],
        }
        lower, upper = get(prim, "physics:lowerLimit"), get(prim, "physics:upperLimit")
        scale = math.radians(1.0) if rotational else mpu
        record["limit"] = [None if lower is None or abs(lower) == math.inf else lower * scale, None if upper is None or abs(upper) == math.inf else upper * scale]
        if prim.HasAPI(UsdPhysics.DriveAPI, token):
            drive_type = get(prim, f"drive:{token}:physics:type", "force")
            k, d = get(prim, f"drive:{token}:physics:stiffness", 0.0), get(prim, f"drive:{token}:physics:damping", 0.0)
            cap = get(prim, f"drive:{token}:physics:maxForce", math.inf)
            gain = (kpu * mpu * mpu * 180.0 / math.pi) if (rotational and drive_type == "force") else (kpu if drive_type == "force" else 1.0)
            record["drive"] = {
                "type": drive_type,
                "authored": {"stiffness": k, "damping": d, "max_force": None if cap == math.inf else cap},
                "si_force_rule": {"stiffness": k * gain, "damping": d * gain} if drive_type == "force" else None,
            }
        joints.append(record)
    for prim in stage.Traverse():
        path = str(prim.GetPath())
        parent = prim.GetParent()
        if path not in body_paths and parent and str(parent.GetPath()) in body_paths and prim.IsA(UsdGeom.Xform):
            sites.append({"path": path, "body": str(parent.GetPath()), "initial_pose": pose(prim)})
    return {"meters_per_unit": mpu, "kilograms_per_unit": kpu, "up_axis": str(UsdGeom.GetStageUpAxis(stage)), "bodies": bodies, "joints": joints, "sites": sites}


def schema_fallbacks():
    """Fallback values the installed schemas give unauthored properties."""
    from pxr import Usd

    registry = Usd.SchemaRegistry()
    wanted = {
        "PhysxRigidBodyAPI": ["physxRigidBody:linearDamping", "physxRigidBody:angularDamping"],
        "PhysicsDriveAPI": ["drive:__INSTANCE_NAME__:physics:type", "drive:__INSTANCE_NAME__:physics:maxForce"],
        "PhysxJointAPI": ["physxJoint:jointFriction", "physxJoint:armature", "physxJoint:maxJointVelocity"],
    }
    found = {}
    for api, names in wanted.items():
        definition = registry.FindAppliedAPIPrimDefinition(api)
        for name in names:
            if definition is None:
                found[f"{api}.{name}"] = "schema not registered"
                continue
            spec = definition.GetSchemaAttributeSpec(name)
            found[f"{api}.{name}"] = None if spec is None else spec.default
    return found


def numbers(array):
    values = array.numpy().tolist() if hasattr(array, "numpy") else list(array)
    clean = lambda v: None if isinstance(v, float) and (math.isinf(v) or abs(v) > 1e30) else v
    walk = lambda v: [walk(x) for x in v] if isinstance(v, list) else clean(v)
    return walk(values)


def mapping(schedule, schedule_path):
    """E03: Isaac's own wheeled-robot controllers on the schedule's cases."""
    import omni.kit.app

    extension = "isaacsim.robot.experimental.wheeled_robots"
    manager = omni.kit.app.get_app().get_extension_manager()
    if not manager.is_extension_enabled(extension):
        manager.set_extension_enabled_immediate(extension, True)
    try:
        from isaacsim.robot.experimental.wheeled_robots.controllers import AckermannController, DifferentialController
    except Exception:
        finish("absent_feature", error=traceback.format_exc())
    cases = []
    for case in schedule["cases"]:
        v, w = case["cmd"]["linear_mps"], case["cmd"]["angular_rps"]
        p = case["parameters"]
        if case["controller"] == "ackermann":
            controller = AckermannController(
                wheel_base=p["wheel_base"],
                track_width=p["track_width"],
                front_wheel_radius=p["wheel_radius"],
                back_wheel_radius=p["wheel_radius"],
                max_wheel_rotation_angle=math.radians(p["max_steer_deg"]),
            )
            steering = math.atan(p["wheel_base"] * w / v) if v != 0 else 0.0
            angles, speeds = controller.forward([steering, 0.0, v, 0.0, 0.0])
            if angles is None:
                finish("runtime_error", error=f"AckermannController rejected case {case['name']}")
            targets = {
                "steer_left": ("position", angles[0]),
                "steer_right": ("position", angles[1]),
                "wheel_front_left": ("velocity", speeds[0]),
                "wheel_front_right": ("velocity", speeds[1]),
                "wheel_back_left": ("velocity", speeds[2]),
                "wheel_back_right": ("velocity", speeds[3]),
            }
            inputs = {"steering_angle": steering, "speed": v, "rule": "steering_angle = atan(wheel_base * angular / linear)"}
        elif case["controller"] == "differential":
            controller = DifferentialController(wheel_radius=p["wheel_radius"], wheel_base=p["wheel_base"])
            left, right = (float(x) for x in controller.forward([v, w]))
            targets = {"wheel_left": ("velocity", left), "wheel_right": ("velocity", right)}
            inputs = {"linear": v, "angular": w}
        else:
            finish("rejected_asset", error=f"unknown controller {case['controller']}")
        cases.append({
            "name": case["name"],
            "controller": case["controller"],
            "isaac_inputs": inputs,
            "dofs": {role: {"mode": mode, "target": float(target)} for role, (mode, target) in targets.items()},
        })
    module = sys.modules[AckermannController.__module__].__file__
    write("mapping.json", {"harness": "isaac", "controllers": os.path.dirname(module), "cases": cases})
    finish("completed", mode="mapping", cases=len(cases), schedule=os.path.basename(schedule_path), schedule_sha256=sha256(schedule_path))


async def main():
    import omni.kit.app
    import omni.usd

    app = omni.kit.app.get_app()
    try:
        info = preflight()
    except Exception:
        finish("startup_failure", error=traceback.format_exc())
    write("preflight.json", info)

    schedule_path = os.environ.get("PROBE_SCHEDULE")
    if not schedule_path or not os.path.isfile(schedule_path):
        finish("startup_failure", error=f"PROBE_SCHEDULE `{schedule_path}` is not a file")
    schedule = json.load(open(schedule_path))
    if schedule.get("mapping"):
        mapping(schedule, schedule_path)
    fixtures = os.path.dirname(os.path.dirname(os.path.abspath(schedule_path)))
    fixture = os.path.join(fixtures, schedule["fixture"])
    overlay = os.path.join(fixtures, schedule["overlay"]) if schedule.get("overlay") else None
    root, tool = schedule["root"], schedule.get("tool")
    hz, ticks = float(schedule["physics_hz"]), int(schedule["ticks"])

    try:
        from isaacsim.core.experimental.prims import Articulation, RigidPrim
        from isaacsim.core.experimental.utils.backend import use_backend
        from isaacsim.core.simulation_manager import SimulationManager
    except Exception:
        finish("absent_feature", error=traceback.format_exc())

    differences = overlay_differences(fixture, overlay)
    write("overlay.json", differences)
    if differences["mechanical_changes"]:
        finish("rejected_asset", error="the Gearbox overlay authors physics properties", changes=differences["mechanical_changes"])

    context = omni.usd.get_context()
    ok, error = await context.open_stage_async(fixture)
    if not ok:
        finish("rejected_asset", error=f"open_stage failed: {error}")
    for _ in range(5):
        await app.next_update_async()
    stage = context.get_stage()
    structure = {"fixture": os.path.basename(fixture), "fixture_sha256": sha256(fixture), "usd": usd_structure(stage)}
    if not stage.GetPrimAtPath(root).IsValid():
        finish("rejected_asset", error=f"no articulation root {root}")

    from pxr import UsdPhysics

    scene = next((p for p in stage.Traverse() if p.IsA(UsdPhysics.Scene)), None)
    gravity = None
    if scene is not None:
        s = UsdPhysics.Scene(scene)
        gravity = {"magnitude": s.GetGravityMagnitudeAttr().Get(), "direction": list(s.GetGravityDirectionAttr().Get())}

    try:
        SimulationManager.set_physics_sim_device("cpu")
        SimulationManager.set_physics_dt(1.0 / hz)
        articulation = Articulation(root)
        SimulationManager.initialize_physics()
        for _ in range(2):
            await app.next_update_async()
        if not articulation.is_physics_tensor_entity_valid():
            finish("absent_feature", error="articulation tensor view did not initialize")
        carriage = None
        if tool:
            tool_body = os.path.dirname(tool)
            carriage = RigidPrim(tool_body)
    except Exception:
        finish("runtime_error", stage="physics initialization", error=traceback.format_exc())

    from pxr import Gf, Usd, UsdGeom

    mpu = UsdGeom.GetStageMetersPerUnit(stage)
    tool_offset = None
    if tool:
        cache = UsdGeom.XformCache(Usd.TimeCode.Default())
        relative, _ = cache.ComputeRelativeTransform(stage.GetPrimAtPath(tool), stage.GetPrimAtPath(os.path.dirname(tool)))
        tool_offset = relative.ExtractTranslation()

    with use_backend("tensor", raise_on_unsupported=True, raise_on_fallback=True):
        names = list(articulation.dof_names)
        stiffness, damping = (numbers(a)[0] for a in articulation.get_dof_gains())
        lower_upper = articulation.get_dof_limits()
        runtime = {
            "engine": SimulationManager.get_active_physics_engine(),
            "physics_dt": SimulationManager.get_physics_dt(),
            "link_names": list(articulation.link_names),
            "joint_names": list(articulation.joint_names),
            "dof_names": names,
            "dof_paths": [list(p) for p in articulation.dof_paths] if articulation.dof_paths else None,
            "dof_types": [str(t) for t in articulation.dof_types] if articulation.dof_types else None,
            "drive_types": articulation.get_dof_drive_types()[0],
            "stiffness": stiffness,
            "damping": damping,
            "max_effort": numbers(articulation.get_dof_max_efforts())[0],
            "limits": [numbers(lower_upper[0])[0], numbers(lower_upper[1])[0]],
            "link_masses": numbers(articulation.get_link_masses())[0],
            "link_inertias": numbers(articulation.get_link_inertias())[0],
            "dof_positions": numbers(articulation.get_dof_positions())[0],
        }
    structure["runtime"] = runtime
    structure["gravity"] = gravity
    structure["schema_fallbacks"] = schema_fallbacks()
    write("structure.json", structure)

    dof = {n: i for i, n in enumerate(names)}
    missing = sorted({c["dof"] for c in schedule["commands"]} - set(dof))
    if missing:
        finish("rejected_asset", error=f"schedule names DOFs the articulation does not report: {missing}")
    k0, d0 = list(stiffness), list(damping)
    mode = ["free"] * len(names)
    target = [math.nan] * len(names)
    effort = [0.0] * len(names)
    trace = []

    def q_now():
        with use_backend("tensor", raise_on_unsupported=True, raise_on_fallback=True):
            return numbers(articulation.get_dof_positions())[0]

    def authored_target(path):
        """Drive targets as authored, or the UsdPhysics fallback 0, in SI."""
        prim = stage.GetPrimAtPath(path)
        rotational = prim.GetTypeName() == "PhysicsRevoluteJoint"
        token, scale = ("angular", math.radians(1.0)) if rotational else ("linear", mpu)
        value = lambda name: (lambda a: a.Get() if a and a.HasAuthoredValue() else 0.0)(prim.GetAttribute(f"drive:{token}:physics:{name}"))
        return value("targetPosition") * scale, value("targetVelocity") * scale

    load_targets = [authored_target(p) for p in list(articulation.dof_paths)[0]]

    try:
        for tick in range(ticks):
            for event in (e for e in schedule.get("events", []) if e["tick"] == tick):
                if event["event"] == "pause":
                    before, steps = q_now(), SimulationManager.get_num_physics_steps()
                    for _ in range(event.get("frames", 0)):
                        await app.next_update_async()
                    trace.append({
                        "tick": tick,
                        "event": "pause",
                        "frames": event.get("frames", 0),
                        "q_before": dict(zip(names, before)),
                        "q_after": dict(zip(names, q_now())),
                        "ticks_advanced": SimulationManager.get_num_physics_steps() - steps,
                    })
                elif event["event"] == "reset":
                    positions, velocities = zip(*load_targets)
                    with use_backend("tensor", raise_on_unsupported=True, raise_on_fallback=True):
                        articulation.set_dof_gains(k0, d0)
                        articulation.set_dof_position_targets(list(positions))
                        articulation.set_dof_velocity_targets(list(velocities))
                    effort, mode, target = [0.0] * len(names), ["uncommanded"] * len(names), [math.nan] * len(names)
                    trace.append({"tick": tick, "event": "reset", "load_targets": dict(zip(names, positions))})
                else:
                    finish("rejected_asset", error=f"unknown schedule event {event['event']}")
            for command in (c for c in schedule["commands"] if c["tick"] == tick):
                i, value = dof[command["dof"]], command.get("value", 0.0)
                m = command["mode"]
                with use_backend("tensor", raise_on_unsupported=True, raise_on_fallback=True):
                    if m in ("position", "hold"):
                        value = q_now()[i] if m == "hold" else value
                        articulation.set_dof_gains(k0[i], d0[i], dof_indices=[i])
                        articulation.set_dof_position_targets(value, dof_indices=[i])
                        articulation.set_dof_velocity_targets(0.0, dof_indices=[i])
                        effort[i], mode[i], target[i] = 0.0, "position", value
                    elif m == "velocity":
                        articulation.set_dof_gains(0.0, d0[i], dof_indices=[i])
                        articulation.set_dof_velocity_targets(value, dof_indices=[i])
                        effort[i], mode[i], target[i] = 0.0, "velocity", value
                    elif m == "effort":
                        articulation.set_dof_gains(0.0, 0.0, dof_indices=[i])
                        effort[i], mode[i], target[i] = value, "effort", value
                    elif m == "free":
                        articulation.set_dof_gains(0.0, 0.0, dof_indices=[i])
                        effort[i], mode[i], target[i] = 0.0, "free", math.nan
                    else:
                        finish("rejected_asset", error=f"unknown schedule mode {m}")
            with use_backend("tensor", raise_on_unsupported=True, raise_on_fallback=True):
                articulation.set_dof_efforts(effort)
            SimulationManager.step(steps=1)
            with use_backend("tensor", raise_on_unsupported=True, raise_on_fallback=True):
                q = numbers(articulation.get_dof_positions())[0]
                qdot = numbers(articulation.get_dof_velocities())[0]
            for n, i in dof.items():
                t = None if math.isnan(target[i]) else target[i]
                trace.append({"tick": tick + 1, "dof": n, "mode": mode[i], "target": t, "q": q[i], "qdot": qdot[i]})
            if carriage is not None:
                with use_backend("tensor", raise_on_unsupported=True, raise_on_fallback=True):
                    positions, orientations = carriage.get_world_poses()
                p = numbers(positions)[0]
                w, x, y, z = numbers(orientations)[0]
                rotated = Gf.Rotation(Gf.Quatd(w, Gf.Vec3d(x, y, z))).TransformDir(Gf.Vec3d(tool_offset))
                trace.append({"tick": tick + 1, "frame": os.path.basename(tool), "translation": [(p[j] + rotated[j]) * mpu for j in range(3)]})
    except Exception:
        finish("runtime_error", stage="schedule", error=traceback.format_exc(), ticks_recorded=len(trace))

    with open(os.path.join(OUT, "trace.jsonl"), "w") as f:
        for line in trace:
            f.write(json.dumps(line) + "\n")
    finish("completed", ticks=ticks, physics_hz=hz, schedule=os.path.basename(schedule_path), schedule_sha256=sha256(schedule_path))


if not OUT:
    print("[robot-contract] PROBE_OUT is not set", flush=True)
    os._exit(EXIT["startup_failure"])
os.makedirs(OUT, exist_ok=True)
watchdog(float(os.environ.get("PROBE_TIMEOUT", "300")))


def guarded():
    async def run():
        try:
            await main()
        except SystemExit:
            raise
        except Exception:
            finish("runtime_error", error=traceback.format_exc())

    return run()


asyncio.ensure_future(guarded())
