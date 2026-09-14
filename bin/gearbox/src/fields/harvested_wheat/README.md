# Harvested wheat

Registers the `harvested_wheat` field profile: brown post-harvest soil, layered ground
textures, subtle 1 m darker residue strips every 4 m with 0.4 m soft edge transitions,
and 5–15 cm cut stalks at 6000 stalks/m², matching the default base-grass density.
Stalks keep their full 4–6 mm width up to a flat-cut top, with minimal wind motion.
Each stalk leans 5–19° and kinks at a node halfway up (6–14°, or 23–57° for the third
snapped over by the header), so the stubble reads from above.
Two-sided PBR shading uses 15% diffuse transmission and 0.6 mm optical thickness;
dry stalks transmit less light than green leaves.
Stalks and soil use low reflectance (0.04), high roughness, and no specular environment
contribution. Diffuse lighting, transmission and fog are unchanged.
`GEARBOX_STUBBLE_DENSITY` overrides stalk density independently of grassland.
Stalks fade radially from 4 to 32 m. Wheel tracks bend stalks and darken the soil
using an independent field map, with 1800-second recovery.
Distance thinning uses the same inverse-square and outer-fade curve as grassland;
the full close-up density is unchanged.

The ground keeps a consistent straw-brown palette with subdued broad colour variation.
Residue strips slightly darken the existing colour instead of adding bright hay bands.
Regrowth patches (`shaders/patches.wgsl`, warped 10–30 m blobs) turn the ground olive green,
turn half the stalks into green volunteer shoots and tint the distant clumps. A leaf layer
(60/m², fading 24–96 m) scatters clover and rosettes the cutter passed over: thick in the
patches, one in ten elsewhere. Wheel tracks bend and darken them like the stalks.

The ground material comes from the existing multi-bale field:
`scripts/bale_run_multi.py` loads `assets/world/terrain.usd`.
The USD material adapter preserves authored tint and reduced hay-row strength for
explicitly tinted crop terrain. That legacy adapter remains material-only; generated
stalks and wheel maps belong to procedural field instances.

## Embedded assets

- `textures/soil_albedo.jpg`, `soil_height.jpg`: ambientCG Ground001 color/displacement.
- `textures/detail_albedo.jpg`, `detail_height.jpg`: ambientCG Ground003 color/displacement.

CC0 1.0 Universal, copied from the existing project packs.
Sources: <https://ambientcg.com/view?id=Ground001>, <https://ambientcg.com/view?id=Ground003>.
