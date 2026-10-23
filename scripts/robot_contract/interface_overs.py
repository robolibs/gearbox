"""Author the command interfaces a v1 machine derived, as a USDA layer.

    python3 interface_overs.py V1_MANIFEST.json OUT.usda

Every DOF whose interfaces were `legacy_derived` and not empty gets
`uniform token[] gearbox:joint:<token>:commandInterfaces` on its joint with
the interfaces its authored drive can realize (position: stiffness > 0,
velocity: damping > 0, effort: any scalar joint). The rest are listed in
OUT.dropped.json.
"""

import json
import sys

SCALAR = {"revolute", "prismatic", "d6"}


def realizable(interface, dof, kind):
    drive = dof.get("drive") or {}
    gain = lambda k: (drive.get(k) or {}).get("value") or 0.0
    if kind not in SCALAR:
        return False
    return {"position": gain("stiffness") > 0, "velocity": gain("damping") > 0, "effort": True}.get(interface, False)


def main():
    machine = json.load(open(sys.argv[1]))["machines"][0]
    kinds = {j["path"]: j["kind"] for j in machine["joints"]}
    joints, dropped = {}, []
    for dof in machine["dofs"]:
        interfaces = dof["interfaces"]
        if interfaces["source"] != "legacy_derived" or not interfaces["value"]:
            continue
        kept = [i for i in interfaces["value"] if realizable(i, dof, kinds.get(dof["joint"]))]
        lost = [i for i in interfaces["value"] if i not in kept]
        if lost:
            dropped.append({"joint": dof["joint"], "dof": dof["name"], "dropped": lost, "reason": "no authored drive gain realizes it"})
        if kept:
            joints.setdefault(dof["joint"], []).append((dof["token"], kept))
    with open(sys.argv[2].replace(".usda", ".dropped.json"), "w") as f:
        json.dump(dropped, f, indent=1)
    tree = {}
    for path, values in joints.items():
        node = tree
        for part in path.strip("/").split("/"):
            node = node.setdefault(part, {})
        node[None] = values

    lines = ["#usda 1.0", ""]

    def emit(node, depth):
        pad = "    " * depth
        for token, interfaces in node.get(None, []):
            listed = ", ".join(f'"{i}"' for i in interfaces)
            lines.append(f"{pad}uniform token[] gearbox:joint:{token}:commandInterfaces = [{listed}]")
        for name, child in node.items():
            if name is None:
                continue
            lines.append(f'{pad}over "{name}"')
            lines.append(pad + "{")
            emit(child, depth + 1)
            lines.append(pad + "}")

    emit(tree, 0)
    with open(sys.argv[2], "w") as f:
        f.write("\n".join(lines) + "\n")
    print(f"[migration] {len(joints)} joints keep their v1 command interfaces, {len(dropped)} DOFs drop unrealizable ones")


if __name__ == "__main__":
    main()
