"""Builds the Mars base in Blender and writes it out:

    assets/mars_base/modules/<kind>.usdc   each kind of structure, a component
                                           model about its own origin, its
                                           shells marked to collide
    assets/mars_base/modules/textures/     each module's looks baked with Mars
                                           wear (weathering.bake_module)
    assets/mars_base/textures/             Poly Haven CC0 sets, baked and
                                           procedural maps, NASA's images
    machines/usd/<robot>.usdz              the robots that tend the crops
    bin/gearbox/assets/world/mars_base.usda        the world: every module
                                           placed, and the greenhouse rooms,
                                           trays, lamps, power and services
    bin/gearbox/assets/world/mars_base_fleet.usda  every robot placed
    bin/gearbox/assets/world/mars_base_farm.json   the work plan: each tending
                                           robot's joint targets for every
                                           plant place it reaches (plan.py)

Run in the open Blender (`exec(open(".../scripts/mars_base/build.py").read())`)
or headless: `blender --background --python scripts/mars_base/build.py`
(after `--`, `--no-export` only builds the scene and `--no-machines` leaves
the robot packages as they are).
kit, buildings, habitat, power, industry, spaceport and props build the
structures; layout places them; eden is the EDEN module's inside; robots
writes the machines; preview.sh renders one module or spot headless.
"""
import math
import os
import sys

import bpy
from mathutils import Euler, Matrix, Vector

ROOT = "/home/bresilla/data/code/robolibs/gearbox"
HERE = os.path.join(ROOT, "scripts", "mars_base")
sys.path.insert(0, HERE)
# A Blender session keeps modules from earlier builds: drop them so every
# build imports the scripts as they are now.
for name in ("kit", "patterns", "buildings", "habitat", "power", "industry", "spaceport", "props", "usda", "eden", "mechanics", "robots", "vehicles",
             "arms", "harvester", "vertical", "crane", "excavator", "washers", "layout", "plan"):
    sys.modules.pop(name, None)
import kit  # noqa: E402
import usda  # noqa: E402
import eden  # noqa: E402
import plan  # noqa: E402
import robots  # noqa: E402
import crane  # noqa: E402
import excavator  # noqa: E402
import harvester  # noqa: E402
import industry  # noqa: E402
import vertical  # noqa: E402
import vehicles  # noqa: E402
import washers  # noqa: E402
import weathering  # noqa: E402
import layout  # noqa: E402

ASSETS = os.path.join(ROOT, "assets", "mars_base")
MODULES = os.path.join(ASSETS, "modules")
MODULE_TEXTURES = os.path.join(MODULES, "textures")
TEXTURES = os.path.join(ASSETS, "textures")
WORLD_DIR = os.path.join(ROOT, "bin", "gearbox", "assets", "world")
MACHINES = os.path.join(ROOT, "machines", "usd")
WORK = os.path.join(ROOT, ".work", "mars_base")
LOOKS = "/MarsBase/Looks"

# What each greenhouse grows, ring by ring or row by row: (species, slots, ppfd, hours, days apart).
CROPS = {
    "DomeA": [("wheat", (4, 3), 0, 20, 9), ("potato", (3, 2), 0, 12, 11), ("wheat", (4, 3), 0, 20, 9)],
    "DomeB": [("lettuce", (4, 3), 0, 16, 3), ("tomato", (3, 2), 0, 12, 8), ("potato", (3, 2), 0, 12, 11)],
    "ArchA": [("lettuce", (4, 3), 300, 16, 2)],
    "ArchB": [("tomato", (3, 2), 600, 12, 5)],
    "ArchC": [("wheat", (4, 3), 600, 20, 4)],
    "VerticalFarm": [("lettuce", (6, 2), 250, 16, 2), ("lettuce", (10, 3), 150, 18, 1)],
}
GROWN_DAYS = {"lettuce": 40, "tomato": 110, "potato": 120, "wheat": 85}


def reset():
    """An empty scene: no objects, collections, meshes, materials or images."""
    for data in (bpy.data.objects, bpy.data.meshes, bpy.data.materials, bpy.data.images, bpy.data.collections, bpy.data.lights, bpy.data.cameras):
        for block in list(data):
            data.remove(block)


def relative(path, start):
    return os.path.relpath(path, start).replace(os.sep, "/")


def export_module(building, path, weather=True):
    """Writes one building's collection about its own origin as a component
    model, its shells marked to collide; with `weather` its looks are first
    baked with Mars wear into textures of its own. Returns how many collide."""
    if weather and not getattr(building, "natural", False):
        os.makedirs(MODULE_TEXTURES, exist_ok=True)
        looks = [o for o in building.collection.objects if o.type == "MESH" and not o.get("gearbox_hidden")]
        weathering.bake_module(looks, MODULE_TEXTURES, building.kind)
    root = building.root
    at, turn = root.location.copy(), root.rotation_euler.copy()
    root.location, root.rotation_euler = (0, 0, 0), (0, 0, 0)
    prim = f"/{building.kind}"
    try:
        bpy.ops.wm.usd_export(
            filepath=path, check_existing=False, collection=building.collection.name, export_animation=False, export_hair=False,
            export_uvmaps=True, rename_uvmaps=True, export_mesh_colors=False, export_normals=True, export_materials=True,
            generate_preview_surface=True, generate_materialx_network=False, export_textures_mode="KEEP",
            relative_paths=True, xform_op_mode="TRS", root_prim_path=prim, export_custom_properties=True,
            convert_world_material=False, export_lights=False, export_cameras=False, export_curves=False, export_points=False,
            export_volumes=False, merge_parent_xform=True, convert_scene_units="METERS", meters_per_unit=1.0,
            triangulate_meshes=False, evaluation_mode="RENDER", use_instancing=False,
        )
    finally:
        root.location, root.rotation_euler = at, turn
    from pxr import Kind, Sdf, Usd, UsdGeom, UsdPhysics
    stage = Usd.Stage.Open(path)
    top = stage.GetPrimAtPath(prim)
    stage.SetDefaultPrim(top)
    UsdGeom.SetStageUpAxis(stage, UsdGeom.Tokens.z)
    UsdGeom.SetStageMetersPerUnit(stage, 1.0)
    model = Usd.ModelAPI(top)
    model.SetKind(Kind.Tokens.component)
    model.SetAssetName(building.kind)
    model.SetAssetIdentifier(Sdf.AssetPath(f"./{building.kind}.usdc"))
    marked = 0
    for p in stage.Traverse():
        flag = p.GetAttribute("userProperties:gearbox_collide")
        if flag and flag.Get():
            UsdPhysics.CollisionAPI.Apply(p)
            UsdPhysics.MeshCollisionAPI.Apply(p).CreateApproximationAttr("none")
            marked += 1
        hidden = p.GetAttribute("userProperties:gearbox_hidden")
        if hidden and hidden.Get():
            UsdGeom.Imageable(p).MakeInvisible()
    stage.GetRootLayer().Save()
    return marked


def world_of(building):
    """The building's frame in the base: translation and yaw."""
    return Vector(building.root.location), building.root.rotation_euler.z


def header(u, creator, default):
    u.emit(0, "#usda 1.0")
    u.emit(0, "(")
    u.emit(1, "customLayerData = {")
    u.emit(2, f'string creator = "{creator}"')
    u.emit(1, "}")
    u.emit(1, f'defaultPrim = "{default}"')
    u.emit(1, "metersPerUnit = 1")
    u.emit(1, 'upAxis = "Z"')
    u.emit(0, ")")
    u.emit(0, "")
    u.emit(0, "# Generated: edit scripts/mars_base and build again.")


def rocks_layer(u, placed):
    """The rocks round the base: each boulder a reference of its scan's
    module, the stones drawn through one point instancer of the stone scans."""
    module = lambda scan: relative(os.path.join(MODULES, f"rock_{scan}.usdc"), WORLD_DIR)
    u.emit(1, 'def Xform "Rocks" (kind = "group")')
    u.emit(1, "{")
    for k, (scan, at, turn, scale) in enumerate(placed["boulder"]):
        u.emit(2, f'def Xform "Boulder{k:03d}" (')
        u.emit(3, f"prepend references = @{module(scan)}@")
        u.emit(2, ")")
        u.emit(2, "{")
        u.emit(3, f"double3 xformOp:translate = {usda.fmt(at)}")
        u.emit(3, f"float3 xformOp:rotateXYZ = {usda.fmt(turn)}")
        u.emit(3, f"float3 xformOp:scale = ({scale}, {scale}, {scale})")
        u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ", "xformOp:scale"]')
        u.emit(2, "}")
    stones = placed["stone"]
    scans = sorted({scan for scan, *_ in stones})
    quat = lambda turn: Euler(tuple(math.radians(a) for a in turn), "XYZ").to_quaternion()
    u.emit(2, 'def PointInstancer "Stones"')
    u.emit(2, "{")
    u.emit(3, "rel prototypes = [" + ", ".join(f"</MarsBase/Rocks/Stones/Prototypes/{scan}>" for scan in scans) + "]")
    u.emit(3, "int[] protoIndices = [" + ", ".join(str(scans.index(scan)) for scan, *_ in stones) + "]")
    u.emit(3, "point3f[] positions = [" + ", ".join(usda.fmt(at) for _, at, _, _ in stones) + "]")
    u.emit(3, "quath[] orientations = [" + ", ".join(usda.fmt(tuple(quat(turn))) for _, _, turn, _ in stones) + "]")
    u.emit(3, "float3[] scales = [" + ", ".join(f"({s}, {s}, {s})" for *_, s in stones) + "]")
    u.emit(3, 'def Scope "Prototypes"')
    u.emit(3, "{")
    for scan in scans:
        u.emit(4, f'def Xform "{scan}" (')
        u.emit(5, f"prepend references = @{module(scan)}@")
        u.emit(4, ")")
        u.emit(4, "{")
        u.emit(4, "}")
    u.emit(3, "}")
    u.emit(2, "}")
    u.emit(1, "}")


def trays_of(b):
    """A greenhouse building's trays, each a dict of its place in the
    building, its crop and how long ago it was sown, and where the building
    stands (`origin`, `turn`) in the base."""
    crops = CROPS.get(b.name)
    room = next((p for kind, p in b.places if kind == "room"), None)
    if crops is None or room is None:
        return []
    at, yaw = world_of(b)
    out = []
    for kind, p in b.places:
        if kind != "tray":
            continue
        species, slots, ppfd, hours, apart = crops[min(p.get("ring", 0), len(crops) - 1)]
        out.append({"name": f"{b.name}_{len(out):03d}", "at": p["at"], "yaw": p["yaw"], "size": p["size"], "slots": slots, "species": species,
                    "ppfd": ppfd, "hours": hours, "lamp": p.get("lamp", 0.6), "lit": "lamp" in p, "days": (len(out) * apart) % GROWN_DAYS[species],
                    "room": room["name"], "origin": tuple(at), "turn": yaw})
    return out


def farm_plan(path, built):
    """Writes the work plan of the robots that tend plants (see plan.py)."""
    trays = [t for b in built for t in trays_of(b)] + [dict(t, origin=tuple(layout.EDEN), turn=0.0) for t in eden.trays()]
    eden_rail, arch_rail, polar = gantries()
    robots_ = []
    arches = dict(layout.ARCHES)
    others = {}
    for name, file, at, yaw, _ in layout.fleet_places(robots.rail_anchor(eden_rail), robots.rail_anchor(arch_rail), robots.polar_anchor(polar)):
        machine = f"base_fleet_{name.lower()}"
        if file == "eden_gantry.usdz":
            robots_.append(plan.rail_gantry(eden_rail, machine, "grow", at, yaw))
        elif file == "arch_gantry.usdz":
            robots_.append(plan.rail_gantry(arch_rail, machine, name[:-3].lower(), at, yaw))
        elif file == "dome_gantry.usdz":
            robots_.append(plan.polar_gantry(polar, machine, name[:-3].lower(), at, yaw))
        elif file == "vertical_crane.usdz":
            robots_.append(plan.stacker_crane(crane.CRANE, machine, "verticalfarm", at, yaw, 6.5 - vertical.CRANE_X0, vertical.FLOOR))
        elif file == "harvester.usdz":
            arch = name[:5]
            stops = sorted({t["at"][0] for t in trays if t["room"] == arch.lower()})
            side = 1 if name.endswith("1") else -1
            robots_.append(plan.mobile_harvester(harvester.HARVESTER, machine, arch.lower(), tuple(arches[arch]), 0.0, side, stops, layout.HARVEST_LANE))
        elif file == "arch_washer.usdz":
            others[machine] = plan.arch_washer(washers.arch(layout.ARCH_LEN), washers.arch_press, name[:5].lower())
        elif file == "dome_washer.usdz":
            others[machine] = plan.dome_washer(name[:5].lower())
        elif file == "panel_washer.usdz":
            others[machine] = plan.panel_washer(washers.panel(layout.SOLAR_PANELS, layout.SOLAR_TILT).span)
        elif file == "regolith_excavator.usdz":
            k = int(name[-1]) - 1
            bays = [(layout.WORKS.x + x, layout.WORKS.y + y) for x, y in industry.SOIL_BAYS]
            lane = [(layout.WORKS.x + x, layout.WORKS.y + y) for x, y in industry.SOIL_LANE]
            others[machine] = plan.excavator(machine, at, tuple(layout.DIG[k]), bays, lane)
    return plan.write(path, robots_, trays, others)


def world_layer(path, built, base=None):
    """The world file: every module placed, the rocks strewn round them, and
    the greenhouse gearbox runs."""
    u = usda.Usda(LOOKS)
    header(u, "scripts/mars_base/build.py: a Mars base that grows its food", "MarsBase")
    u.emit(0, "# Load it: `gearbox run --planet mars --world bin/gearbox/assets/world/mars_base.usda`; then spawn the")
    u.emit(0, "# robots with `gearbox spawn machine bin/gearbox/assets/world/mars_base_fleet.usda --machine base`.")
    u.emit(0, 'def Xform "MarsBase" (kind = "assembly")')
    u.emit(0, "{")
    u.emit(1, 'def Xform "Modules" (kind = "group")')
    u.emit(1, "{")
    for b in built:
        if getattr(b, "prototype", False):
            continue
        at, yaw = world_of(b)
        u.emit(2, f'def Xform "{b.name}" (')
        u.emit(3, f"prepend references = @{relative(os.path.join(MODULES, b.kind + '.usdc'), WORLD_DIR)}@")
        u.emit(2, ")")
        u.emit(2, "{")
        u.emit(3, f"double3 xformOp:translate = {usda.fmt(at)}")
        u.emit(3, f"float3 xformOp:rotateXYZ = (0, 0, {math.degrees(yaw):.4f})")
        u.emit(3, f"float3 xformOp:scale = {usda.fmt(b.root.scale)}")
        u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ", "xformOp:scale"]')
        u.emit(2, "}")
    u.emit(1, "}")
    placed = next((p for kind, p in base.places if kind == "rocks"), None) if base else None
    if placed:
        rocks_layer(u, placed)
    usda.materials(u, 1, eden.EDEN_LOOKS)
    usda.xform(u, 1, "Greenhouse", (0, 0, 0))
    eden.module(u, 2, tuple(layout.EDEN))
    rooms = trays = 0
    solar = 0.0
    station = None
    lights = []
    for b in built:
        at, yaw = world_of(b)
        to_base = Matrix.Translation(at) @ Matrix.Rotation(yaw, 4, "Z")
        room = next((p for kind, p in b.places if kind == "room"), None)
        for kind, p in b.places:
            if kind == "solar":
                solar += p["area"]
            elif kind == "power":
                station = to_base @ Vector(p["at"])
            elif kind == "light":
                lights.append((to_base @ Vector(p["at"]), p["kind"], yaw + p.get("yaw", 0.0)))
        if room is None or b.name not in CROPS:
            continue
        rooms += 1
        name = room["name"]
        usda.zone(u, 2, b.name, name, tuple(at), math.degrees(yaw), room["volume"], room["centre"], room["size"],
                  tank=room["volume"] * 2.0, glazing=room.get("glazing", 0.0))
        for t in trays_of(b):
            usda.tray(u, 3, t["name"], t["at"], t["species"], t["slots"], name, t["ppfd"], t["hours"], t["lamp"], t["days"], (*t["size"], 0.08),
                      yaw_deg=math.degrees(t["yaw"]), lamp=t["lit"])
            trays += 1
        spots = {}
        for kind, p in b.places:
            if kind == "service":
                # A second spot for the same fault, like an arch's far air handler, is numbered.
                seen = spots.get(p["fault"], 0)
                spots[p["fault"]] = seen + 1
                usda.service(u, 3, f"service_{p['fault']}" + (f"_{seen}" if seen else ""), p["at"], p["fault"], name)
        usda.end(u, 2)
    if station is not None:
        usda.xform(u, 2, "Power", tuple(station), ["GearboxPowerAPI"], [
            f"float gearbox:power:reactor = {10000 * len(layout.REACTORS)}",
            f"float gearbox:power:solarArea = {solar:.0f}",
            "float gearbox:power:battery = 400000",
        ])
        usda.end(u, 2)
        usda.service(u, 2, "power_panel", tuple(station), "power", "")
    usda.end(u, 1)
    u.emit(1, 'def Scope "Lights"')
    u.emit(1, "{")
    for k, (p, kind, yaw) in enumerate(lights):
        lumens, cone_deg, colour = {"flood": (24.0, 55.0, (1.0, 0.95, 0.86)), "door": (3.0, 70.0, (1.0, 0.92, 0.8)), "bay": (9.0, 75.0, (1.0, 0.96, 0.9))}[kind]
        aim = Vector((math.cos(yaw), math.sin(yaw), -0.7)) if kind == "flood" else Vector((0, 0, -1.0))
        euler = aim.normalized().to_track_quat("-Z", "Z").to_euler("XYZ")
        u.emit(2, f'def SphereLight "{kind}{k:02d}" (')
        u.emit(3, 'prepend apiSchemas = ["ShapingAPI"]')
        u.emit(2, ")")
        u.emit(2, "{")
        u.emit(3, f"float inputs:intensity = {lumens}")
        u.emit(3, "float inputs:radius = 0.1")
        u.emit(3, f"color3f inputs:color = {usda.fmt(colour)}")
        u.emit(3, f"float inputs:shaping:cone:angle = {cone_deg}")
        u.emit(3, f"double3 xformOp:translate = {usda.fmt(p)}")
        u.emit(3, f"float3 xformOp:rotateXYZ = {usda.fmt([math.degrees(a) for a in euler])}")
        u.emit(3, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]')
        u.emit(2, "}")
    u.emit(1, "}")
    u.emit(0, "}")
    with open(path, "w") as f:
        f.write(u.text())
    return rooms, trays, len(lights)


def gantries():
    """The sizes of the EDEN, arch and dome gantries."""
    rail = layout.dome_gantry_rail()
    return robots.EDEN_RAIL, robots.arch_rail(layout.ARCH_LEN), robots.Polar(leg_r=rail, travel=rail - 4.4)


def write_machines():
    """Writes each robot's machine layer and geometry and packs them as a usdz among the machines."""
    from pxr import Sdf, UsdUtils
    eden_rail, arch_rail, polar = gantries()
    work = os.path.join(WORK, "machines")
    os.makedirs(work, exist_ok=True)
    layers = {
        "eden_gantry": robots.build("eden_gantry", eden_rail, "the rail robot of the EDEN growth room", work),
        "arch_gantry": robots.build("arch_gantry", arch_rail, "the ridge robot of an arch greenhouse", work),
        "dome_gantry": robots.build("dome_gantry", polar, "the polar gantry of a dome", work),
    }
    layers[vehicles.SEV.kind] = vehicles.build(vehicles.SEV, layout.NASA, MACHINES, work)
    layers.update(crane.build(work))
    layers.update(harvester.build(work))
    layers.update(excavator.build(work))
    layers.update(washers.build(work, layout.ARCH_LEN, layout.DOME_R, layout.SOLAR_PANELS, layout.SOLAR_TILT))
    for name, layer in layers.items():
        UsdUtils.CreateNewUsdzPackage(Sdf.AssetPath(layer), os.path.join(MACHINES, f"{name}.usdz"))
    return list(layers)


def write_fleet(path):
    eden_rail, arch_rail, polar = gantries()
    places = layout.fleet_places(robots.rail_anchor(eden_rail), robots.rail_anchor(arch_rail), robots.polar_anchor(polar))
    with open(path, "w") as f:
        f.write(robots.fleet(places, relative(MACHINES, WORLD_DIR)))
    return len(places)


def main(export=True, weather=True, machines=True):
    reset()
    looks = kit.Looks(TEXTURES)
    base, built = layout.build(looks, strewn=True)
    report = {"buildings": len(built), "kinds": len({b.kind for b in built}), "objects": len(bpy.data.objects)}
    if export:
        for folder in (MODULES, MODULE_TEXTURES):
            os.makedirs(folder, exist_ok=True)
            for old in os.listdir(folder):
                if os.path.isfile(os.path.join(folder, old)):
                    os.remove(os.path.join(folder, old))
        done, colliders = set(), 0
        for b in built:
            if b.kind not in done:
                colliders += export_module(b, os.path.join(MODULES, f"{b.kind}.usdc"), weather)
                done.add(b.kind)
        report["colliders"] = colliders
        report["rooms"], report["trays"], report["lights"] = world_layer(os.path.join(WORLD_DIR, "mars_base.usda"), built, base)
        if machines:
            report["machines"] = write_machines()
        report["robots"] = write_fleet(os.path.join(WORLD_DIR, "mars_base_fleet.usda"))
        report["plan"] = farm_plan(os.path.join(WORLD_DIR, "mars_base_farm.json"), built)
    return report


if __name__ == "__main__":
    print(main("--no-export" not in sys.argv, machines="--no-machines" not in sys.argv))
