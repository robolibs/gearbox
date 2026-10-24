"""Mars wear baked into a model's own texture atlas with Blender's Cycles.

The model's material maps come from what each texel of its colour and
specular maps shows: painted panels, bare and copper metal, solar cells and
glass each take their roughness and metal. The bake then lays dust on what
faces the sky and on what rides low where wheels throw it, darkens what the
sky cannot reach, and turns the tyres to rubber; dust roughens and dulls
whatever it lies on.
"""
import math
import os

import bpy
import numpy as np

def linear(srgb):
    return tuple(c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in srgb)


# Mars dust, tyre rubber and the wheels' anodised metal, picked in sRGB;
# shader sockets take them linear.
DUST = linear((0.80, 0.62, 0.47))
RUBBER = linear((0.055, 0.05, 0.048))
GUNMETAL = linear((0.42, 0.43, 0.45))


def pixels(image, size):
    """An image's RGBA texels, scaled to `size`, top row last as Blender keeps them."""
    copy = image.copy()
    copy.scale(size, size)
    px = np.empty(size * size * 4, dtype=np.float32)
    copy.pixels.foreach_get(px)
    bpy.data.images.remove(copy)
    return px.reshape(size, size, 4)


def grey_image(name, values, path):
    """A Non-Color image of `values` in [0, 1], saved to `path`."""
    size = values.shape[0]
    image = bpy.data.images.new(name, size, size, alpha=False)
    image.colorspace_settings.name = "Non-Color"
    rgba = np.repeat(np.clip(values, 0.0, 1.0)[..., None], 4, axis=-1)
    rgba[..., 3] = 1.0
    image.pixels.foreach_set(rgba.astype(np.float32).ravel())
    image.filepath_raw = path
    image.file_format = "PNG"
    image.save()
    return image


def surface_maps(colour, spec, size):
    """Roughness and metal per texel, from the colour and specular maps."""
    c = pixels(colour, size)[..., :3]
    s = pixels(spec, size)[..., :3]
    lum = c @ np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
    shine = s @ np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
    sat = c.max(axis=-1) - c.min(axis=-1)
    rough = np.clip(0.66 - 0.22 * (shine - 0.5), 0.45, 0.8)
    metal = np.zeros_like(lum)
    bare = (sat < 0.07) & (lum > 0.12) & (lum < 0.78) & (shine < 0.92)
    rough[bare], metal[bare] = 0.45, 0.6
    copper = (c[..., 0] > c[..., 2] + 0.12) & (sat > 0.15)
    rough[copper], metal[copper] = 0.45, 0.55
    cells = (c[..., 2] > c[..., 0] + 0.12) & (c[..., 2] > 0.15)
    rough[cells], metal[cells] = 0.14, 0.15
    glass = lum < 0.06
    rough[glass], metal[glass] = 0.16, 0.0
    return rough, metal


class Graph:
    """Shader nodes of a bake, wired as they are made."""

    def __init__(self, mat):
        self.nodes, self.links = mat.node_tree.nodes, mat.node_tree.links

    def node(self, kind, **inputs):
        n = self.nodes.new(kind)
        for key, value in inputs.items():
            if isinstance(key, str) and key.startswith("_"):
                setattr(n, key[1:], value)
            else:
                self.put(n.inputs[key], value)
        return n

    def put(self, socket, value):
        if hasattr(value, "is_output") or hasattr(value, "links"):
            self.links.new(value, socket)
        elif isinstance(value, (tuple, list)):
            socket.default_value = (*value, 1.0)[: len(socket.default_value)]
        else:
            socket.default_value = value

    def math(self, op, a, b=0.0, clamp=False):
        n = self.node("ShaderNodeMath", _operation=op, _use_clamp=clamp)
        self.put(n.inputs[0], a)
        self.put(n.inputs[1], b)
        return n.outputs[0]

    def mix(self, a, b, fac, kind="RGBA"):
        n = self.nodes.new("ShaderNodeMix")
        n.data_type = kind
        sockets = {"RGBA": (6, 7, 2), "FLOAT": (2, 3, 0)}[kind]
        self.put(n.inputs[0], fac)
        self.put(n.inputs[sockets[0]], a)
        self.put(n.inputs[sockets[1]], b)
        return n.outputs[sockets[2]]

    def ramp(self, value, lo, hi):
        n = self.node("ShaderNodeMapRange", _interpolation_type="SMOOTHSTEP")
        self.put(n.inputs[0], value)
        n.inputs[1].default_value, n.inputs[2].default_value = lo, hi
        return n.outputs[0]


def masks(g, ground_z):
    """Dust, the sky's reach, and the model's tyre and wheel marks, each 0..1."""
    geo = g.node("ShaderNodeNewGeometry")
    sep_p = g.node("ShaderNodeSeparateXYZ", Vector=geo.outputs["Position"])
    sep_n = g.node("ShaderNodeSeparateXYZ", Vector=geo.outputs["Normal"])
    height = g.math("SUBTRACT", sep_p.outputs["Z"], ground_z)
    low = g.math("SUBTRACT", 1.0, g.ramp(height, 0.2, 2.0))
    up = g.ramp(sep_n.outputs["Z"], 0.3, 0.95)
    facing = g.ramp(sep_n.outputs["Z"], -0.7, 0.5)
    upright = g.math("SUBTRACT", 1.0, g.math("ABSOLUTE", sep_n.outputs["Z"]))
    broad = g.node("ShaderNodeTexNoise", Vector=geo.outputs["Position"], Scale=1.1, Detail=3.0, Roughness=0.45)
    fine = g.node("ShaderNodeTexNoise", Vector=geo.outputs["Position"], Scale=30.0, Detail=2.0)
    stretched = g.node("ShaderNodeVectorMath", _operation="MULTIPLY", Vector=geo.outputs["Position"])
    stretched.inputs[1].default_value = (7.0, 7.0, 0.6)
    streak = g.node("ShaderNodeTexNoise", Vector=stretched.outputs[0], Scale=1.0, Detail=2.0)
    patch = g.ramp(broad.outputs["Fac"], 0.3, 0.7)
    speck = g.ramp(fine.outputs["Fac"], 0.35, 0.65)
    runs = g.math("MULTIPLY", g.ramp(streak.outputs["Fac"], 0.5, 0.72), upright)
    settled = g.math("MULTIPLY", up, g.math("ADD", 0.45, g.math("MULTIPLY", patch, 0.55)))
    thrown = g.math("MULTIPLY", g.math("MULTIPLY", low, facing), g.math("ADD", 0.55, g.math("MULTIPLY", speck, 0.45)))
    dust = g.math("ADD", g.math("MULTIPLY", settled, 0.6), g.math("MULTIPLY", thrown, 0.7))
    dust = g.math("MINIMUM", g.math("ADD", dust, g.math("MULTIPLY", runs, 0.22)), 0.82)
    ao = g.node("ShaderNodeAmbientOcclusion", Distance=0.35, _samples=16)
    tread = g.node("ShaderNodeAttribute", _attribute_name="tread", _attribute_type="GEOMETRY")
    wheel = g.node("ShaderNodeAttribute", _attribute_name="wheel", _attribute_type="GEOMETRY")
    return dust, ao.outputs["AO"], tread.outputs["Fac"], wheel.outputs["Fac"]


def bake(obj, mat, ground_z, out_dir, name, size=4096, maps_size=2048):
    """Bakes the weathered colour, roughness and metal of `obj`'s `mat` into
    `out_dir`; returns a material that shows them over the model's normals."""
    nodes = mat.node_tree.nodes
    textures = {link.to_socket.name: link.from_node for link in mat.node_tree.links
                if link.to_node.type in ("BSDF_PRINCIPLED", "NORMAL_MAP") and link.from_node.type == "TEX_IMAGE"}
    colour = textures["Base Color"].image
    normal = textures["Color"].image
    spec = next(n.image for n in nodes if n.type == "TEX_IMAGE" and n.image not in (colour, normal))
    os.makedirs(out_dir, exist_ok=True)
    rough0, metal0 = surface_maps(colour, spec, maps_size)
    rough0 = grey_image(f"{name}_rough0", rough0, os.path.join(out_dir, f"{name}_rough0.png"))
    metal0 = grey_image(f"{name}_metal0", metal0, os.path.join(out_dir, f"{name}_metal0.png"))

    work = bpy.data.materials.new(f"{name}_bake")
    work.use_nodes = True
    g = Graph(work)
    for n in list(g.nodes):
        g.nodes.remove(n)
    uv = g.node("ShaderNodeTexCoord").outputs["UV"]
    col = g.node("ShaderNodeTexImage", Vector=uv, _image=colour).outputs["Color"]
    r0 = g.node("ShaderNodeTexImage", Vector=uv, _image=rough0).outputs["Color"]
    m0 = g.node("ShaderNodeTexImage", Vector=uv, _image=metal0).outputs["Color"]
    dust, ao, tread, wheel = masks(g, ground_z)
    anodised = g.node("ShaderNodeMix", _data_type="RGBA", _blend_type="MULTIPLY")
    g.put(anodised.inputs[0], wheel)
    g.put(anodised.inputs[6], col)
    g.put(anodised.inputs[7], GUNMETAL)
    rubbered = g.mix(anodised.outputs[2], RUBBER, g.math("MULTIPLY", tread, 0.92))
    grime = g.math("ADD", 0.5, g.math("MULTIPLY", ao, 0.5))
    shaded = g.node("ShaderNodeMix", _data_type="RGBA", _blend_type="MULTIPLY", Factor=1.0)
    g.put(shaded.inputs[6], rubbered)
    g.put(shaded.inputs[7], g.node("ShaderNodeCombineColor", Red=grime, Green=grime, Blue=grime).outputs[0])
    albedo = g.mix(shaded.outputs[2], DUST, dust)
    rough = g.math("MAXIMUM", g.mix(g.mix(r0, 0.86, tread, "FLOAT"), 0.96, dust, "FLOAT"), 0.0)
    metal = g.math("MULTIPLY", g.math("MULTIPLY", m0, g.math("SUBTRACT", 1.0, dust)), g.math("SUBTRACT", 1.0, tread))
    emit = g.node("ShaderNodeEmission", Strength=1.0)
    g.links.new(emit.outputs[0], g.node("ShaderNodeOutputMaterial").inputs["Surface"])
    target = g.node("ShaderNodeTexImage")

    scene = bpy.context.scene
    engine = scene.render.engine
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 16
    scene.render.bake.margin = 8
    original = list(obj.data.materials)
    obj.data.materials.clear()
    obj.data.materials.append(work)
    view = bpy.context.view_layer
    for o in view.objects:
        o.select_set(False)
    obj.hide_render = obj.hide_viewport = False
    obj.select_set(True)
    view.objects.active = obj
    baked = {}
    for key, value, srgb, res in (("colour", albedo, True, size), ("rough", rough, False, maps_size), ("metal", metal, False, maps_size)):
        image = bpy.data.images.new(f"{name}_{key}", res, res, alpha=False)
        image.colorspace_settings.name = "sRGB" if srgb else "Non-Color"
        target.image = image
        g.nodes.active = target
        if hasattr(value, "type") and value.type == "RGBA":
            g.links.new(value, emit.inputs["Color"])
        else:
            g.links.new(g.node("ShaderNodeCombineColor", Red=value, Green=value, Blue=value).outputs[0], emit.inputs["Color"])
        bpy.ops.object.bake(type="EMIT")
        image.filepath_raw = os.path.join(out_dir, f"{name}_{key}.jpg")
        image.file_format = "JPEG"
        scene.render.image_settings.quality = 92
        image.save()
        baked[key] = image
    scene.render.engine = engine
    obj.data.materials.clear()
    for m in original:
        obj.data.materials.append(m)
    bpy.data.materials.remove(work)

    return baked_look(name, baked["colour"], baked["rough"], baked["metal"], normal)


def baked_look(name, colour, rough, metal, normal, uv=None):
    """A material showing baked colour, roughness and metal over a normal map,
    read through the UV map `uv` (the mesh's active one when None)."""
    look = bpy.data.materials.new(name)
    look.use_nodes = True
    lg = Graph(look)
    bsdf = next(n for n in lg.nodes if n.type == "BSDF_PRINCIPLED")
    coords = lg.node("ShaderNodeUVMap", _uv_map=uv).outputs["UV"] if uv else None
    texture = lambda image: lg.node("ShaderNodeTexImage", _image=image, **({"Vector": coords} if coords else {})).outputs["Color"]
    lg.put(bsdf.inputs["Base Color"], texture(colour))
    lg.put(bsdf.inputs["Roughness"], texture(rough))
    lg.put(bsdf.inputs["Metallic"], texture(metal))
    if normal is not None:
        lg.put(bsdf.inputs["Normal"], lg.node("ShaderNodeNormalMap", Color=texture(normal), **({"_uv_map": uv} if uv else {})).outputs["Normal"])
    return look


def keeps_own(mat):
    """Lit and see-through materials stay as they are, unbaked."""
    if mat is None or not mat.use_nodes:
        return True
    bsdf = next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
    if bsdf is None:
        return True
    lit = bsdf.inputs["Emission Strength"].default_value > 0.0 and any(c > 0.0 for c in bsdf.inputs["Emission Color"].default_value[:3])
    return lit or bsdf.inputs["Alpha"].default_value < 1.0 or mat.surface_render_method == "BLENDED"


def source(g, mat, socket, uv):
    """What a material's Principled `socket` reads, rebuilt in graph `g`: its
    image through the `st` UV map, or its constant."""
    bsdf = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    inp = bsdf.inputs[socket]
    if inp.links and inp.links[0].from_node.type == "TEX_IMAGE":
        return g.node("ShaderNodeTexImage", Vector=uv, _image=inp.links[0].from_node.image).outputs["Color"]
    value = inp.default_value
    return tuple(value[:3]) if hasattr(value, "__len__") else float(value)


def surface_area(objs):
    return sum(sum(p.area for p in o.data.polygons) * abs(o.matrix_world.to_3x3().determinant()) ** (2 / 3) for o in objs)


def bake_module(objs, out_dir, name, ground_z=0.0, density=0.02, sizes=(512, 4096), samples=8):
    """Bakes a building's looks with Mars wear into one atlas: the parts that
    are not lit or see-through take a `bake` UV packed together, their colour
    (dust and grime over each material's own texture), roughness, metal and
    normals are baked into it at about `density` metres a texel, and they are
    joined into one object showing the bake. Returns that object, or None."""
    parts = [o for o in objs if o.type == "MESH" and not keeps_own(o.active_material)]
    if not parts:
        return None
    side = surface_area(parts) ** 0.5 / density
    size = int(min(max(2 ** math.ceil(math.log2(max(side, 1.0))), sizes[0]), sizes[1]))
    view = bpy.context.view_layer
    for o in view.objects:
        o.select_set(False)
    for o in parts:
        o.hide_viewport = o.hide_render = False
        o.data.uv_layers.new(name="bake")
        o.data.uv_layers.active = o.data.uv_layers["bake"]
        o.select_set(True)
    view.objects.active = parts[0]
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(60), island_margin=0.004, scale_to_bounds=False)
    bpy.ops.object.mode_set(mode="OBJECT")

    images = {}
    for key, srgb in (("colour", True), ("rough", False), ("metal", False), ("normal", False)):
        image = bpy.data.images.new(f"{name}_{key}", size if key in ("colour", "normal") else size // 2, size if key in ("colour", "normal") else size // 2, alpha=False)
        image.colorspace_settings.name = "sRGB" if srgb else "Non-Color"
        images[key] = image
    graphs = {}
    for o in parts:
        mat = o.active_material
        if mat.name in graphs:
            continue
        work = mat.copy()
        work.name = f"{mat.name}_bake"
        g = Graph(work)
        st = g.node("ShaderNodeUVMap", _uv_map="st").outputs["UV"]
        dust, ao, tread, _ = masks(g, ground_z)
        col = source(g, mat, "Base Color", st)
        r0 = source(g, mat, "Roughness", st)
        m0 = source(g, mat, "Metallic", st)
        shaded = g.node("ShaderNodeMix", _data_type="RGBA", _blend_type="MULTIPLY", Factor=1.0)
        g.put(shaded.inputs[6], col)
        grime = g.math("ADD", 0.5, g.math("MULTIPLY", ao, 0.5))
        g.put(shaded.inputs[7], g.node("ShaderNodeCombineColor", Red=grime, Green=grime, Blue=grime).outputs[0])
        albedo = g.mix(shaded.outputs[2], DUST, dust)
        rough = g.mix(r0, 0.95, dust, "FLOAT")
        metal = g.math("MULTIPLY", m0, g.math("SUBTRACT", 1.0, dust))
        emit = g.node("ShaderNodeEmission", Strength=1.0)
        target = g.node("ShaderNodeTexImage")
        bsdf = next(n for n in g.nodes if n.type == "BSDF_PRINCIPLED")
        output = next(n for n in g.nodes if n.type == "OUTPUT_MATERIAL")
        graphs[mat.name] = (work, g, {"colour": albedo, "rough": rough, "metal": metal}, emit, target, bsdf, output)
    originals = {o.name: o.active_material for o in parts}
    for o in parts:
        o.active_material = graphs[originals[o.name].name][0]

    scene = bpy.context.scene
    engine = scene.render.engine
    scene.render.engine = "CYCLES"
    scene.cycles.samples = samples
    scene.render.bake.margin = 6
    for key, image in images.items():
        for work, g, outputs, emit, target, bsdf, output in graphs.values():
            target.image = image
            g.nodes.active = target
            if key == "normal":
                g.links.new(bsdf.outputs[0], output.inputs["Surface"])
                continue
            value = outputs[key]
            if value.type != "RGBA":
                value = g.node("ShaderNodeCombineColor", Red=value, Green=value, Blue=value).outputs[0]
            g.links.new(value, emit.inputs["Color"])
            g.links.new(emit.outputs[0], output.inputs["Surface"])
        if key == "normal":
            bpy.ops.object.bake(type="NORMAL", normal_space="TANGENT")
        else:
            bpy.ops.object.bake(type="EMIT")
        image.filepath_raw = os.path.join(out_dir, f"{name}_{key}.{'png' if key == 'normal' else 'jpg'}")
        image.file_format = "PNG" if key == "normal" else "JPEG"
        scene.render.image_settings.quality = 88
        image.save()
    scene.render.engine = engine

    look = baked_look(f"{name}_look", images["colour"], images["rough"], images["metal"], images["normal"])
    for o in parts:
        o.active_material = look
        st = o.data.uv_layers.get("st")
        if st is not None:
            o.data.uv_layers.remove(st)
    for work, *_ in graphs.values():
        bpy.data.materials.remove(work)
    for o in view.objects:
        o.select_set(False)
    for o in parts:
        o.select_set(True)
    view.objects.active = parts[0]
    bpy.ops.object.join()
    joined = view.objects.active
    joined.name = f"{name}_look"
    joined.data.materials.clear()
    joined.data.materials.append(look)
    return joined
