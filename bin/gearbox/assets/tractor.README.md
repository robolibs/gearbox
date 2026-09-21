# Fendt 300 Vario S4

`tractor.usd` replaces the stock tractor with the refined Fendt Blender model.
Its geometry is in `tractor_geom.usdc`; textures are in `tractor_textures/`.
Keep these together when copying the asset.
Textures are PNG because the current Gearbox build does not enable DDS decoding.

The `/robot` root, chassis/wheel/steering body paths, six joint names, machine
roles, parallel front steering, 45-degree limits, and stock masses/inertias are
preserved. Wheelbase, track widths, wheel radii, collision shapes and joint
anchors match the Fendt. There are seven sibling rigid bodies and seven colliders.
Cab and implement mechanisms are exported in their closed static pose.

Source: `/home/bresilla/Fendt_300_Vario_S4_detailed.blend`.
Regenerate with `make export-usd render-usd install-usd` in
`/home/bresilla/fendt300_work`. `make validate-usd` checks geometry, dependencies,
materials, original control metadata, body hierarchy and coincident joint anchors.

The previous `tractor.usd` and `tractor.usdc` are backed up in
`/home/bresilla/fendt300_work/usd-backup/tractor-original/`.
The old `tractor.usdc` remains in the asset folder but is not used by this wrapper.
Oxbo is unchanged by this export.
