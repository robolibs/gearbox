"""Bake staged v2 overlays into machine packages.

    python3 promote_v2.py STAGING OUT ASSET_ROOT

STAGING is a `stage-v2-migration` run (packages/, layers/, v2/). For every
staged package that is byte-identical to ASSET_ROOT/<name>.usdz this writes
OUT/<name>.usdz: a new root layer `<name>_v2.usda` (the staged overlay, its
sublayers made package-relative), the interfaces layer, then every original
entry unchanged and in order. Entries are stored uncompressed with their
data 64-byte aligned, as USDZ requires. ASSET_ROOT is only read.
"""

import hashlib
import os
import struct
import sys
import zipfile

ALIGN = 64
PAD_ID = 0x1986


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def write_aligned(zout, info, data):
    """Store `data` so it starts on an ALIGN-byte boundary."""
    info.compress_type = zipfile.ZIP_STORED
    info.extra = b""
    start = zout.fp.tell() + 30 + len(info.filename.encode())
    pad = (-(start + 4)) % ALIGN
    info.extra = struct.pack("<HH", PAD_ID, pad) + b"\0" * pad
    zout.writestr(info, data)


def check_aligned(path):
    with open(path, "rb") as f, zipfile.ZipFile(path) as z:
        for info in z.infolist():
            f.seek(info.header_offset)
            header = f.read(30)
            name_len, extra_len = struct.unpack("<HH", header[26:30])
            data = info.header_offset + 30 + name_len + extra_len
            if info.compress_type != zipfile.ZIP_STORED or data % ALIGN:
                raise SystemExit(f"{path}: {info.filename} is not stored and {ALIGN}-byte aligned")


def promote(staging, out, asset_root, name):
    original = os.path.join(asset_root, name + ".usdz")
    staged = os.path.join(staging, "packages", name + ".usdz")
    if sha256(original) != sha256(staged):
        raise SystemExit(f"{name}: production package differs from the staged copy; stage again")
    overlay = open(os.path.join(staging, "v2", name + "_v2.usda")).read()
    interfaces = open(os.path.join(staging, "layers", name + "_interfaces.usda")).read()
    with zipfile.ZipFile(original) as zin:
        entries = zin.infolist()
        root = entries[0].filename
        layer, iface = f"{name}_v2.usda", f"{name}_interfaces.usda"
        if any(e.filename in (layer, iface) for e in entries):
            raise SystemExit(f"{name}: the package already holds {layer} or {iface}")
        old = f"subLayers = [@../layers/{name}_interfaces.usda@, @../packages/{name}.usdz@]"
        if old not in overlay:
            raise SystemExit(f"{name}: unexpected sublayers in the staged overlay")
        overlay = overlay.replace(old, f"subLayers = [@./{iface}@, @./{root}@]")
        target = os.path.join(out, name + ".usdz")
        with zipfile.ZipFile(target, "w") as zout:
            write_aligned(zout, zipfile.ZipInfo(layer, date_time=entries[0].date_time), overlay.encode())
            write_aligned(zout, zipfile.ZipInfo(iface, date_time=entries[0].date_time), interfaces.encode())
            for entry in entries:
                write_aligned(zout, zipfile.ZipInfo(entry.filename, date_time=entry.date_time), zin.read(entry))
    check_aligned(target)
    print(f"[promote] {name}: root {layer} over {root}, {len(entries)} original entries kept")


def main():
    staging, out, asset_root = sys.argv[1:4]
    os.makedirs(out, exist_ok=True)
    names = sorted(f[:-5] for f in os.listdir(os.path.join(staging, "packages")) if f.endswith(".usdz"))
    for name in names:
        promote(staging, out, asset_root, name)


if __name__ == "__main__":
    main()
