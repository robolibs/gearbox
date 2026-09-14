# Grassland

Owns the green ground material, shared patch palette, dense grass, clover/rosettes,
blade geometry, and wheel-response policy. `mod.rs` registers the `grassland` profile.
The shared field runtime and renderer allocate and draw independent instances.

Defaults: 6000 base blades/m², full density through 4 m, fading to zero at 32 m;
80 detail plants/m² fading gently from 8 to 24 m. `GEARBOX_GRASS_DENSITY` overrides base
density; zero disables vegetation. Tracks recover over 300 seconds.
The accepted palette, patch scale, and blade sizes are retained. Vegetation uses two-sided
PBR normals and 35% diffuse transmission for thin-leaf backlighting, with 0.2 mm optical
thickness. Blade normals follow the ribbon with an upward canopy bias; wheel-flattened
blades return toward the ground normal. No emissive brightness is added.
Primary grass falloff combines inverse-square thinning with a quadratic outer fade:
100% through 4 m, at most 18% at 8 m, 6% at 12 m, and under 1% at 20 m.
Individual blades fade over intervals up to 4 m. CPU budgets and per-blade fade radii
use the same curve; shader distances are full 3D camera-to-plant distances.
The secondary/tertiary grasses, clover and rosettes do not use inverse-square thinning.
Vegetation and ground have low dielectric reflectance (0.04) and no specular environment
contribution; diffuse sky fill, leaf transmission and atmospheric fog remain active.

## Embedded assets

- `textures/grass_albedo.jpg`: ambientCG Grass005, 1K JPG color map.
- `textures/soil_albedo.jpg`: ambientCG Ground001, 1K JPG color map.

Both are CC0 1.0 Universal, copied from the existing project texture packs.
Sources: <https://ambientcg.com/view?id=Grass005>, <https://ambientcg.com/view?id=Ground001>.
