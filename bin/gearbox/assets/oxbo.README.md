# Oxbo EPD540e

`oxbo.usd` is the Gearbox machine/physics layer. Keep `oxbo_geom.usdc` beside it.
The binary geometry layer contains the detailed six-wheel Blender model, materials,
labels, folded conveyor, and attached hydraulic hoses. It has no external textures.

The upgraded wheels have moulded flotation carcasses, 26 pairs of swept traction
lugs, embossed rubber lettering, recessed ventilated steel rims, and separate hub,
washer, stud, nut, and valve details. All 62 visual parts per wheel remain attached
to its original rolling body. Axle positions, rolling radii, steering, and collision
dimensions are unchanged. This is reconstructed visual detail, not manufacturer CAD.

Load it through the existing `examples/python/oxbo_flatland.py` workflow, or:

```sh
make run RUN_ARGS=bin/gearbox/assets/oxbo.usd
```

The existing `builtin:ackermann_cmd_vel` controller is preserved: two powered rear
wheel joints, four passive front/middle wheel joints, and four front/rear steering joints.
The middle axle stays straight. The EPD540e wheelbase/track/radii replace the old
procedural 2475 dimensions; steering multipliers and side differentials are unchanged.
All eleven rigid bodies are siblings, connected by ten revolute joints. Collisions
use three hidden eight-vertex box hulls and six hidden tyre cylinders, not the visual meshes.
Box extents are baked into mesh points because the current Gearbox collider adapter
does not apply USD Cube scale to the physics shape. Mass/inertia are approximate,
with the chassis center of mass placed near the middle of the six-wheel support area.

## Export scope

This replacement is drivable, with the harvesting assemblies in their stowed pose.
The Blender harvesting/unloading buttons, driver expressions, and flexible-hose rig
are not Gearbox controllers and are not transferred as executable control logic.
Their evaluated closed pose and named transform hierarchy are preserved in the USD.
The complete editable articulation and demo remain in
`/home/bresilla/OXBO_EPD540e_articulated.blend`.

Materials use USD Preview Surface. Blender-only procedural shader detail is not baked
to texture maps. Standard surface colors, UVs, normals, material slots, and geometric
labels remain present.

## Re-export and validation

The Blender exporter is `scripts/export_oxbo_blender.py`. Its local input is the saved
articulated blend; it writes staging files into `/home/bresilla/oxbo_epd540e_work/usd`.
It does not overwrite the live Blender file or publish directly into this directory.

```sh
make -C /home/bresilla/oxbo_epd540e_work export-usd
make -C /home/bresilla/oxbo_epd540e_work render-usd
make test
```

The local work directory also contains `usd-validation.json`, `usd-roundtrip.png`,
`gearbox-usd-preview.png`, and the Gearbox test/runtime logs.
Wheel-upgrade comparisons, native Gearbox previews, and the pre-upgrade backups are
in `/home/bresilla/oxbo_epd540e_work/wheel-upgrade`. `make render-wheels` in that work
directory produces the Blender detail and complete-machine views. Its
`upgrade_wheels.py` rebuilds only reconstructed wheel visuals on the existing rig.
The upgraded asset passed `make test`, 504 Blender rig checks, USD validation, and
native Gearbox visual inspection. The unchanged physics setup previously passed a
live acceleration/turn/brake smoke test (274 controller state samples); that earlier
runtime record is in `gearbox-drive-smoke.json` and was not rerun for this visual-only pass.

The original procedural `oxbo.usd` is backed up at
`/home/bresilla/oxbo_epd540e_work/usd-backup/oxbo.original.usd`.
Restore by copying that file over `bin/gearbox/assets/oxbo.usd`; its existing relative
`tractor.usdc` references resolve again from this assets directory.
