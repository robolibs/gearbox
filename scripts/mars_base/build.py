"""Builds the Mars base in Blender and writes it out:

    assets/mars_base/modules/<kind>.usdc   each kind of structure, about its
                                           own origin, textured, its shells
                                           marked to collide
    assets/mars_base/textures/             Poly Haven CC0 sets, baked and
                                           procedural maps, NASA's images
    machines/usd/<robot>.usdz              the robots that tend the crops
    bin/gearbox/assets/world/mars_base.usda        the world: every module
                                           placed, and the greenhouse rooms,
                                           trays, lamps, power and services
    bin/gearbox/assets/world/mars_base_fleet.usda  every robot placed

Run in the open Blender (`exec(open(".../scripts/mars_base/build.py").read())`)
or headless: `blender --background --python scripts/mars_base/build.py`.
kit, buildings, habitat, power, industry, spaceport and props build the
structures; layout places them; eden is the EDEN module's inside; robots
writes the machines.
"""
import math
import os
import sys

import bpy
from mathutils import Matrix, Vector

ROOT = "/home/bresilla/data/code/robolibs/gearbox"
HERE = os.path.join(ROOT, "scripts", "mars_base")
sys.path.insert(0, HERE)
# A Blender session keeps modules from earlier builds: drop them so every
# build imports the scripts as they are now.
for name in ("kit", "patterns", "buildings", "habitat", "power", "industry", "spaceport", "props", "usda", "eden", "robots", "vehicles", "layout"):
    sys.modules.pop(name, None)
import kit  # noqa: E402
import usda  # noqa: E402
import eden  # noqa: E402
import robots  # noqa: E402
import vehicles  # noqa: E402
import layout  # noqa: E402

ASSETS = os.path.join(ROOT, "assets", "mars_base")
MODULES = os.path.join(ASSETS, "modules")
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
}
GROWN_DAYS = {"lettuce": 40, "tomato": 110, "potato": 120, "wheat": 85}


def reset():
    """An empty scene: no objects, collections, meshes, materials or images."""
    for data in (bpy.data.objects, bpy.data.meshes, bpy.data.materials, bpy.data.images, bpy.data.collections, bpy.data.lights, bpy.data.cameras):
        for block in list(data):
            data.remove(block)


def relative(path, start):
    return os.path.relpath(path, start).replace(os.sep, "/")


def export_module(building, path):
    """Writes one building's collection about its own origin, its shells
    marked to collide; returns how many collide."""
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
    from pxr import Usd, UsdGeom, UsdPhysics
    stage = Usd.Stage.Open(path)
    stage.SetDefaultPrim(stage.GetPrimAtPath(prim))
    UsdGeom.SetStageUpAxis(stage, UsdGeom.Tokens.z)
    UsdGeom.SetStageMetersPerUnit(stage, 1.0)
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


def world_layer(path, built):
    """The world file: every module placed, and the greenhouse gearbox runs."""
    u = usda.Usda(LOOKS)
    header(u, "scripts/mars_base/build.py: a Mars base that grows its food", "MarsBase")
    u.emit(0, "# Load it: `gearbox run --planet mars --world bin/gearbox/assets/world/mars_base.usda`; then spawn the")
    u.emit(0, "# robots with `gearbox spawn machine bin/gearbox/assets/world/mars_base_fleet.usda --machine base`.")
    u.emit(0, 'def Xform "MarsBase"')
    u.emit(0, "{")
    u.emit(1, 'def Scope "Modules"')
    u.emit(1, "{")
    for b in built:
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
        crops = CROPS.get(b.name)
        if room is None or crops is None:
            continue
        rooms += 1
        name = room["name"]
        usda.zone(u, 2, b.name, name, tuple(at), math.degrees(yaw), room["volume"], room["centre"], room["size"],
                  tank=room["volume"] * 2.0, glazing=room.get("glazing", 0.0))
        count, spots = 0, {}
        for kind, p in b.places:
            if kind == "tray":
                species, slots, ppfd, hours, apart = crops[min(p.get("ring", 0), len(crops) - 1)]
                days = (count * apart) % GROWN_DAYS[species]
                usda.tray(u, 3, f"{b.name}_{count:03d}", p["at"], species, slots, name, ppfd, hours, p.get("lamp", 0.6), days, (*p["size"], 0.08),
                          yaw_deg=math.degrees(p["yaw"]), lamp="lamp" in p)
                count += 1
            elif kind == "service":
                # A second spot for the same fault, like an arch's far air handler, is numbered.
                seen = spots.get(p["fault"], 0)
                spots[p["fault"]] = seen + 1
                usda.service(u, 3, f"service_{p['fault']}" + (f"_{seen}" if seen else ""), p["at"], p["fault"], name)
        trays += count
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
    """Writes each robot's USDA and packs it as a usdz among the machines."""
    from pxr import Sdf, UsdUtils
    eden_rail, arch_rail, polar = gantries()
    sources = {
        "eden_gantry": robots.rail_gantry(eden_rail, "the rail robot of the EDEN growth room"),
        "arch_gantry": robots.rail_gantry(arch_rail, "the ridge robot of an arch greenhouse"),
        "dome_gantry": robots.polar_gantry(polar, "the polar gantry of a dome"),
    }
    work = os.path.join(WORK, "machines")
    os.makedirs(work, exist_ok=True)
    layers = {}
    for name, text in sources.items():
        layers[name] = os.path.join(work, f"{name}.usda")
        with open(layers[name], "w") as f:
            f.write(text)
    layers[vehicles.SEV.kind] = vehicles.build(vehicles.SEV, layout.NASA, MACHINES, work)
    for name, layer in layers.items():
        UsdUtils.CreateNewUsdzPackage(Sdf.AssetPath(layer), os.path.join(MACHINES, f"{name}.usdz"))
    return list(layers)


def write_fleet(path):
    eden_rail, arch_rail, polar = gantries()
    places = layout.fleet_places(robots.rail_anchor(eden_rail), robots.rail_anchor(arch_rail), robots.polar_anchor(polar))
    with open(path, "w") as f:
        f.write(robots.fleet(places, relative(MACHINES, WORLD_DIR)))
    return len(places)


def main(export=True):
    reset()
    looks = kit.Looks(TEXTURES)
    base, built = layout.build(looks)
    report = {"buildings": len(built), "kinds": len({b.kind for b in built}), "objects": len(bpy.data.objects)}
    if export:
        os.makedirs(MODULES, exist_ok=True)
        for old in os.listdir(MODULES):
            os.remove(os.path.join(MODULES, old))
        done, colliders = set(), 0
        for b in built:
            if b.kind not in done:
                colliders += export_module(b, os.path.join(MODULES, f"{b.kind}.usdc"))
                done.add(b.kind)
        report["colliders"] = colliders
        report["rooms"], report["trays"], report["lights"] = world_layer(os.path.join(WORLD_DIR, "mars_base.usda"), built)
        report["machines"] = write_machines()
        report["robots"] = write_fleet(os.path.join(WORLD_DIR, "mars_base_fleet.usda"))
    return report


if __name__ == "__main__":
    print(main("--no-export" not in sys.argv))
