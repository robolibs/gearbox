"""Mars wear on a machine package's own looks, in its .blend (see
machines/scripts/README.md). The dust and sky reach of `weathering.masks`,
over the model standing on its wheels, go into

    each colour texture its materials read, baked again in its UV layout,
    darkened where the sky cannot reach and dusted (`weathering.worn`);
    each texture they read roughness or metal from, rougher and duller under
    the dust in the channels they read (`weathering.dulled`);
    each constant colour, roughness and metal of their surfaces, as that
    wear's mean over the faces wearing it.

Textures keep their names, formats and sizes, material specs their shape;
see-through materials stay clean. The clean textures and specs stay in the
.blend and every run starts from them. Headless, then exported as ever:

    flatpak run --filesystem=$PWD org.blender.Blender -b --factory-startup \\
        machines/blender/perseverance_nasa.blend --python scripts/mars_base/weather_machine.py
    oslo make blender-export --blend=$PWD/machines/blender/perseverance_nasa.blend \\
        --package=$PWD/machines/usd/perseverance_nasa.usdz
"""
import os
import sys
from collections import namedtuple

import bpy
import numpy as np
from pxr import Gf, Sdf

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(ROOT, "machines", "scripts"))

import weathering  # noqa: E402
from gearbox_usd import spec as S  # noqa: E402

CLEAN = "weathering_clean"
PROBE = "weathering_probe"
DEFAULTS = {"diffuseColor": (0.18, 0.18, 0.18), "roughness": 0.5, "metallic": 0.0, "opacity": 1.0}
CHANNELS = {"r": "Red", "g": "Green", "b": "Blue"}
Texture = namedtuple("Texture", "file channel uv")


def _default(spec, name):
    attr = spec.attributes.get(name)
    return attr.default if attr is not None and attr.HasDefaultValue() else None


class Look:
    """The UsdPreviewSurface of a material's clean spec: each input of
    DEFAULTS as its constant, a Texture, or None when not authored."""

    def __init__(self, mat):
        self.layer = S.parse(mat.get(CLEAN, mat[S.SPEC]))
        paths = []
        self.layer.Traverse(Sdf.Path.absoluteRootPath, lambda p: paths.append(p) if p.IsPrimPath() else None)
        prims = [self.layer.GetPrimAtPath(p) for p in paths]
        self.surface = next((s for s in prims if s.typeName == "Shader" and _default(s, "info:id") == "UsdPreviewSurface"), None)
        self.inputs = {name: self.read(name) for name in DEFAULTS} if self.surface else {}

    def read(self, name):
        attr = self.surface.attributes.get("inputs:" + name)
        if attr is None:
            return None
        targets = list(attr.connectionPathList.explicitItems) or list(attr.connectionPathList.prependedItems)
        if not targets:
            value = attr.default if attr.HasDefaultValue() else None
            return tuple(float(c) for c in value) if hasattr(value, "__len__") else value
        texture = self.layer.GetPrimAtPath(targets[0].GetPrimPath())
        reader = self.layer.GetPrimAtPath(list(texture.attributes["inputs:st"].connectionPathList.explicitItems)[0].GetPrimPath())
        return Texture(texture.attributes["inputs:file"].default.path.lstrip("./"), targets[0].name.split(":")[-1], _default(reader, "inputs:varname"))

    @property
    def see_through(self):
        opacity = self.inputs.get("opacity")
        return isinstance(opacity, Texture) or (opacity is not None and opacity < 1.0)

    def value(self, name):
        found = self.inputs[name]
        return DEFAULTS[name] if found is None else found

    def constant(self, name):
        found = self.inputs.get(name)
        return found is not None and not isinstance(found, Texture)

    def spec(self, values):
        """The clean spec text with the surface's inputs set to `values`."""
        layer = S.parse(self.layer.ExportToString())
        surface = layer.GetPrimAtPath(self.surface.path)
        for name, value in values.items():
            surface.attributes["inputs:" + name].default = value
        return layer.ExportToString()


def clean_image(image, data):
    """The clean copy of a package texture kept in the .blend, made on the first run."""
    asset = image[S.ASSET]
    clean = next((i for i in bpy.data.images if i.get(CLEAN) == asset), None)
    if clean is None:
        clean = image.copy()
        del clean[S.ASSET], clean[S.ORDER]
        clean[CLEAN] = asset
        clean.name = f"{asset} clean"
        clean.use_fake_user = True
    clean.colorspace_settings.name = "Non-Color" if data else "sRGB"
    return clean


def canvas(clean, data):
    """A bake target holding `clean`'s texels."""
    w, h = clean.size
    image = bpy.data.images.new(f"{clean.name} worn", w, h, alpha=False)
    image.colorspace_settings.name = "Non-Color" if data else "sRGB"
    px = np.empty(w * h * 4, np.float32)
    clean.pixels.foreach_get(px)
    image.pixels.foreach_set(px)
    return image


def sample(g, image, uv, channel="rgb"):
    """`image` read at its texels through the UV map `uv`: its colour, or one channel."""
    coords = g.node("ShaderNodeUVMap", _uv_map="primvars:" + uv).outputs["UV"]
    colour = g.node("ShaderNodeTexImage", Vector=coords, _image=image, _interpolation="Closest").outputs["Color"]
    return colour if channel == "rgb" else g.node("ShaderNodeSeparateColor", Color=colour).outputs[CHANNELS[channel]]


def colours_into(worn, uv):
    """Picks a look's worn colour into the target of the colour texture it reads through `uv`."""
    def pick(look, g, colour, rough, metal, clean):
        read = look.inputs["diffuseColor"]
        return (colour, worn[read.file]) if isinstance(read, Texture) and read.uv == uv else None
    return pick


def maps_into(worn, uv):
    """Picks a look's worn roughness and metal into the channels it reads
    them from of a texture read through `uv`; its other channels stay."""
    def pick(look, g, colour, rough, metal, clean):
        read = [(look.inputs[name], value) for name, value in (("roughness", rough), ("metallic", metal))
                if isinstance(look.inputs[name], Texture) and look.inputs[name].uv == uv]
        if not read:
            return None
        file = read[0][0].file
        channels = {c: sample(g, clean[file], uv, c) for c in CHANNELS}
        channels.update({t.channel: value for t, value in read if t.file == file})
        return g.node("ShaderNodeCombineColor", Red=channels["r"], Green=channels["g"], Blue=channels["b"]).outputs[0], worn[file]
    return pick


def bake(objs, looks, pick, target, clean, ground_z, uv=""):
    """One EMIT bake over `objs`: where a face's material has a look for which
    `pick(look, g, colour, rough, metal, clean)` gives (value, image), value
    goes into image, or into the active colour attribute when image is None;
    the other faces bake into scrap."""
    scrap = bpy.data.images.new("weathering_scrap", 4, 4, alpha=False)
    works = {}
    for mat, look in looks.items():
        work = bpy.data.materials.new(f"{mat.name}_weathering")
        work.use_nodes = True
        g = weathering.Graph(work)
        for n in list(g.nodes):
            g.nodes.remove(n)
        dust, ao, _, _ = weathering.masks(g, ground_z)
        source = {}
        for name in ("diffuseColor", "roughness", "metallic"):
            value = look.value(name)
            source[name] = sample(g, clean[value.file], value.uv, value.channel) if isinstance(value, Texture) else value
        colour = weathering.worn(g, source["diffuseColor"], dust, ao)
        rough, metal = weathering.dulled(g, source["roughness"], source["metallic"], dust)
        picked = pick(look, g, colour, rough, metal, clean)
        if picked is None:
            bpy.data.materials.remove(work)
            continue
        emit = g.node("ShaderNodeEmission", Color=picked[0], Strength=1.0)
        g.links.new(emit.outputs[0], g.node("ShaderNodeOutputMaterial").inputs["Surface"])
        g.nodes.active = g.node("ShaderNodeTexImage", _image=picked[1] or scrap)
        works[mat] = work
    dummy = bpy.data.materials.new("weathering_scrap")
    dummy.use_nodes = True
    dummy.node_tree.nodes.active = dummy.node_tree.nodes.new("ShaderNodeTexImage")
    dummy.node_tree.nodes.active.image = scrap

    chosen = [o for o in objs if any(m in works for m in o.data.materials) and (not uv or "primvars:" + uv in o.data.uv_layers)]
    view = bpy.context.view_layer
    for o in view.objects:
        o.select_set(False)
    kept = {}
    for o in chosen:
        kept[o] = list(o.data.materials)
        for i, m in enumerate(kept[o]):
            o.data.materials[i] = works.get(m, dummy)
        o.select_set(True)
    if chosen:
        view.objects.active = chosen[0]
        bpy.ops.object.bake(type="EMIT", target=target, uv_layer="primvars:" + uv if uv else "", margin=4, margin_type="EXTEND", use_clear=False)
    for o, mats in kept.items():
        for i, m in enumerate(mats):
            o.data.materials[i] = m
        o.select_set(False)
    for work in [*works.values(), dummy]:
        bpy.data.materials.remove(work)
    bpy.data.images.remove(scrap)


def means(objs, looks, pick, clean, ground_z):
    """Each material's area-weighted mean of the colour `pick(look, g,
    colour, rough, metal, clean)` gives, baked into a colour attribute of
    every face of `objs`."""
    for o in objs:
        o.data.color_attributes.active_color = o.data.color_attributes.new(PROBE, "FLOAT_COLOR", "CORNER")
    bake(objs, looks, lambda *a: (pick(*a), None), "VERTEX_COLORS", clean, ground_z)
    sums = {}
    for o in objs:
        me = o.data
        probe = me.color_attributes[PROBE]
        corners = np.empty(len(me.loops) * 4, np.float32)
        probe.data.foreach_get("color", corners)
        me.color_attributes.remove(probe)
        face = {key: np.empty(len(me.polygons), kind) for key, kind in (("loop_start", np.int32), ("loop_total", np.int32), ("area", np.float32), ("material_index", np.int32))}
        for key, values in face.items():
            me.polygons.foreach_get(key, values)
        colour = np.add.reduceat(corners.reshape(-1, 4)[:, :3], face["loop_start"]) / face["loop_total"][:, None]
        area = face["area"] * abs(np.linalg.det(np.array(o.matrix_world.to_3x3()))) ** (2 / 3)
        for i, mat in enumerate(me.materials):
            wearing = face["material_index"] == i
            if mat in looks and wearing.any():
                total = sums.setdefault(mat, [np.zeros(3), 0.0])
                total[0] += (colour[wearing] * area[wearing, None]).sum(axis=0)
                total[1] += area[wearing].sum()
    return {mat: weighted / area for mat, (weighted, area) in sums.items() if area > 0}


def ground(objs):
    """The lowest world height of `objs`' vertices."""
    lowest = []
    for o in objs:
        co = np.empty(len(o.data.vertices) * 3, np.float32)
        o.data.vertices.foreach_get("co", co)
        lowest.append((co.reshape(-1, 3) @ np.array(o.matrix_world)[:3, :3].T + np.array(o.matrix_world)[:3, 3])[:, 2].min())
    return float(min(lowest))


def repack(image, path):
    """`image`'s packed file becomes the file at `path`."""
    keep = image.filepath_raw
    image.unpack(method="REMOVE")
    image.filepath_raw = path
    image.reload()
    image.pack()
    image.filepath_raw = keep


def show(mat, values):
    """The material's Blender display takes the surface's new constants."""
    bsdf = next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None) if mat.use_nodes else None
    for name, socket in (("diffuseColor", "Base Color"), ("roughness", "Roughness"), ("metallic", "Metallic")):
        if bsdf is not None and name in values and not bsdf.inputs[socket].is_linked:
            value = values[name]
            bsdf.inputs[socket].default_value = (*value, 1.0) if name == "diffuseColor" else value


def main():
    scene = bpy.context.scene
    objs = [o for o in bpy.context.view_layer.objects if o.type == "MESH" and not o.hide_render and len(o.data.materials)]
    ground_z = ground(objs)
    looks = {mat: Look(mat) for mat in {m for o in objs for m in o.data.materials if m is not None and m.get(S.SPEC)}}
    looks = {mat: look for mat, look in looks.items() if look.surface is not None and not look.see_through}
    textures = [(name, look.inputs[name]) for look in looks.values() for name in ("diffuseColor", "roughness", "metallic") if isinstance(look.inputs[name], Texture)]
    data = {t.file for name, t in textures if name != "diffuseColor"}
    assets = {image[S.ASSET]: image for image in bpy.data.images if image.get(S.ASSET)}
    clean = {t.file: clean_image(assets[t.file], t.file in data) for _, t in textures}
    worn = {file: canvas(image, file in data) for file, image in clean.items()}

    settings = (scene.render.engine, scene.cycles.samples, scene.cycles.device)
    scene.render.engine, scene.cycles.samples, scene.cycles.device = "CYCLES", 16, "CPU"
    for uv in sorted({t.uv for _, t in textures}):
        bake(objs, looks, colours_into(worn, uv), "IMAGE_TEXTURES", clean, ground_z, uv)
        bake(objs, looks, maps_into(worn, uv), "IMAGE_TEXTURES", clean, ground_z, uv)
    colours = means(objs, looks, lambda look, g, colour, rough, metal, clean: colour, clean, ground_z)
    surfaces = means(objs, looks, lambda look, g, colour, rough, metal, clean: g.node("ShaderNodeCombineColor", Red=rough, Green=metal).outputs[0], clean, ground_z)
    scene.render.engine, scene.cycles.samples, scene.cycles.device = settings

    out = os.path.join(ROOT, ".work", "weathering", os.path.splitext(os.path.basename(bpy.data.filepath))[0])
    for file, image in worn.items():
        path = os.path.join(out, file)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        image.filepath_raw = path
        image.file_format = "PNG" if file.lower().endswith(".png") else "JPEG"
        image.save(quality=90)
        repack(assets[file], path)
        bpy.data.images.remove(image)
        print(f"weather_machine: {file} worn")
    for mat, look in looks.items():
        values = {}
        if look.constant("diffuseColor") and mat in colours:
            values["diffuseColor"] = Gf.Vec3f(*(round(float(c), 4) for c in colours[mat]))
        for k, name in enumerate(("roughness", "metallic")):
            if look.constant(name) and mat in surfaces:
                values[name] = round(float(surfaces[mat][k]), 4)
        if values or CLEAN in mat:
            mat[CLEAN] = mat.get(CLEAN, mat[S.SPEC])
            mat[S.SPEC] = look.spec(values)
            show(mat, values)
            print(f"weather_machine: {mat.name} {', '.join(f'{k} {v}' for k, v in values.items())}")
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_mainfile(compress=True)
    print(f"weather_machine: {bpy.data.filepath} weathered")


main()
