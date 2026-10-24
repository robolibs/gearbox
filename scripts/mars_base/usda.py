"""A small USDA text writer for the gearbox layer: Xforms, Cubes and
Cylinders with materials, and the greenhouse schemas on them."""


class Usda:
    def __init__(self, looks_path):
        self.lines = []
        self.looks = looks_path

    def emit(self, depth, text):
        self.lines.append("    " * depth + text)

    def text(self):
        return "\n".join(self.lines) + "\n"


def fmt(values):
    return "(" + ", ".join(f"{round(v, 4):g}" for v in values) + ")"


def schemas(names):
    return "[" + ", ".join(f'"{n}"' for n in names) + "]"


def xform(u, depth, name, at, schemas_=(), attrs=(), yaw_deg=0.0):
    """Opens an Xform at `at`, turned `yaw_deg` about Z; close it with `end`."""
    if schemas_:
        u.emit(depth, f'def Xform "{name}" (')
        u.emit(depth + 1, f"prepend apiSchemas = {schemas(schemas_)}")
        u.emit(depth, ")")
    else:
        u.emit(depth, f'def Xform "{name}"')
    u.emit(depth, "{")
    for attr in attrs:
        u.emit(depth + 1, attr)
    u.emit(depth + 1, f"double3 xformOp:translate = {fmt(at)}")
    if yaw_deg:
        u.emit(depth + 1, f"float3 xformOp:rotateXYZ = (0, 0, {yaw_deg:.4f})")
        u.emit(depth + 1, 'uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]')
    else:
        u.emit(depth + 1, 'uniform token[] xformOpOrder = ["xformOp:translate"]')


def end(u, depth):
    u.emit(depth, "}")


def cube(u, depth, name, centre, size, material, collide=True, rotate=None):
    """A Cube scaled to `size`, centred at `centre`."""
    u.emit(depth, f'def Cube "{name}" (')
    u.emit(depth + 1, f"prepend apiSchemas = {schemas(['MaterialBindingAPI'] + (['PhysicsCollisionAPI'] if collide else []))}")
    u.emit(depth, ")")
    u.emit(depth, "{")
    u.emit(depth + 1, "double size = 1")
    u.emit(depth + 1, f"double3 xformOp:translate = {fmt(centre)}")
    order = ['"xformOp:translate"']
    if rotate:
        u.emit(depth + 1, f"float3 xformOp:rotateXYZ = {fmt(rotate)}")
        order.append('"xformOp:rotateXYZ"')
    u.emit(depth + 1, f"float3 xformOp:scale = {fmt(size)}")
    order.append('"xformOp:scale"')
    u.emit(depth + 1, f"uniform token[] xformOpOrder = [{', '.join(order)}]")
    u.emit(depth + 1, f"rel material:binding = <{u.looks}/{material}>")
    u.emit(depth, "}")


def cylinder(u, depth, name, centre, radius, height, material, collide=True, axis="Z"):
    u.emit(depth, f'def Cylinder "{name}" (')
    u.emit(depth + 1, f"prepend apiSchemas = {schemas(['MaterialBindingAPI'] + (['PhysicsCollisionAPI'] if collide else []))}")
    u.emit(depth, ")")
    u.emit(depth, "{")
    u.emit(depth + 1, f'uniform token axis = "{axis}"')
    u.emit(depth + 1, f"double radius = {radius}")
    u.emit(depth + 1, f"double height = {height}")
    u.emit(depth + 1, f"double3 xformOp:translate = {fmt(centre)}")
    u.emit(depth + 1, 'uniform token[] xformOpOrder = ["xformOp:translate"]')
    u.emit(depth + 1, f"rel material:binding = <{u.looks}/{material}>")
    u.emit(depth, "}")


def materials(u, depth, table):
    """A Looks scope of flat UsdPreviewSurface materials: name → (colour, metal, rough, emissive?)."""
    u.emit(depth, 'def Scope "Looks"')
    u.emit(depth, "{")
    for name, (colour, metal, rough, glow) in table.items():
        u.emit(depth + 1, f'def Material "{name}"')
        u.emit(depth + 1, "{")
        u.emit(depth + 2, f"token outputs:surface.connect = <{u.looks}/{name}/Surface.outputs:surface>")
        u.emit(depth + 2, 'def Shader "Surface"')
        u.emit(depth + 2, "{")
        u.emit(depth + 3, 'uniform token info:id = "UsdPreviewSurface"')
        u.emit(depth + 3, f"color3f inputs:diffuseColor = {fmt(colour)}")
        u.emit(depth + 3, f"float inputs:metallic = {metal}")
        u.emit(depth + 3, f"float inputs:roughness = {rough}")
        if glow:
            u.emit(depth + 3, f"color3f inputs:emissiveColor = {fmt(colour)}")
        u.emit(depth + 3, "token outputs:surface")
        u.emit(depth + 2, "}")
        u.emit(depth + 1, "}")
    u.emit(depth, "}")


def tray(u, depth, name, at, species, slots, zone, ppfd, hours, lamp_height, sown_days, size, medium="hydroponic", perchlorate=0.0,
         yaw_deg=0.0, lamp=True, basin="tray"):
    """A `GearboxTrayAPI` tray: its basin, and the lamp over it unless it grows by daylight."""
    attrs = [
        f'token gearbox:tray:species = "{species}"',
        f"int2 gearbox:tray:slots = ({slots[0]}, {slots[1]})",
        f'token gearbox:tray:medium = "{medium}"',
        f'token gearbox:tray:zone = "{zone}"',
        f"float2 gearbox:tray:size = ({size[0]}, {size[1]})",
        f"float gearbox:tray:ppfd = {ppfd}",
        f"float gearbox:tray:lampHours = {hours}",
        f"float gearbox:tray:lampHeight = {lamp_height:.3f}",
        f"float gearbox:tray:sownDays = {sown_days}",
    ]
    if medium == "regolith":
        attrs.append(f"float gearbox:tray:perchlorate = {perchlorate}")
    xform(u, depth, name, at, ["GearboxTrayAPI"], attrs, yaw_deg)
    # The table or terrace under a basin stands in the physics world for it.
    cube(u, depth + 1, "basin", (0, 0, -size[2] / 2), size, "regolith" if medium == "regolith" else basin, collide=False)
    if lamp:
        xform(u, depth + 1, "lamp", (0, 0, lamp_height))
        cube(u, depth + 2, "housing", (0, 0, 0.02), (size[0] + 0.02, size[1] + 0.02, 0.04), "housing", collide=False)
        cube(u, depth + 2, "face", (0, 0, -0.002), (size[0] - 0.04, size[1] - 0.04, 0.004), "led", collide=False)
        end(u, depth + 1)
    end(u, depth)


def zone(u, depth, name, room, at, yaw_deg, volume, centre, size, setpoints=(21, 0.75, 1000, 101.3), tank=400, ec=2.5, work=0, glazing=0.0):
    t, rh, co2, p = setpoints
    attrs = [
        f'token gearbox:zone:name = "{room}"',
        f"float gearbox:zone:volume = {volume:.2f}",
        f"float gearbox:zone:temperature = {t}",
        f"float gearbox:zone:humidity = {rh}",
        f"float gearbox:zone:co2 = {co2}",
        f"float gearbox:zone:pressure = {p}",
        f"float gearbox:zone:tank = {tank}",
        f"float gearbox:zone:ec = {ec}",
        f"float gearbox:zone:workLight = {work}",
        f"float3 gearbox:zone:centre = {fmt(centre)}",
        f"float3 gearbox:zone:size = {fmt(size)}",
    ]
    if glazing:
        attrs.append(f"float gearbox:zone:glazing = {glazing}")
    xform(u, depth, name, at, ["GearboxZoneAPI"], attrs, yaw_deg)


def service(u, depth, name, at, fault, room):
    xform(u, depth, name, at, ["GearboxServiceAPI"], [f'token gearbox:service:fault = "{fault}"', f'token gearbox:service:zone = "{room}"'])
    cube(u, depth + 1, "panel", (0, 0, 0), (0.3, 0.12, 0.3), "door", collide=False)
    end(u, depth)
