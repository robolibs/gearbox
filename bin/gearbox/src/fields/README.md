# Field surfaces

Field appearance and wheel response are independent of terrain geometry and scene-wide sky.

## Ownership

- `../terrain.rs`: height samples, colliders, surface meshes, mesh LOD, and distant ground.
- `grassland/`: green ground material, shared patch palette, grass/clover/rosette geometry,
  vegetation shader, textures, density/fade settings, and wheel-response policy.
- `harvested_wheat/`: brown post-harvest ground, irregular cut-hay rows, short cut stalks,
  shaders, textures, and wheel-response policy.
- `profile.rs`: extensible package registry and ground-material/vegetation contracts.
- `layout.rs`: validated, non-overlapping rectangular field definitions.
- `geometry.rs`: clips existing terrain triangles at field boundaries without height offsets.
- `runtime.rs`: field entities, material assignment, heightmap sharing, independent track maps,
  and camera-distance vegetation streaming.
- `render.rs`: shared instanced renderer with a shader and GPU bindings per field/layer.
- `canopy.rs`, `shaders/canopy.wgsl`: sparse, crossed-quad distant grass/straw clusters.
- `textures.rs`: linear-light mip chains and anisotropic sampling for field textures.
- `shaders/surface_detail.wgsl`: filtered fibers, surface normals and canopy lighting.
- `contacts.rs`, `shaders/interaction.wgsl`: wheel contacts, footprint stamping, and track sampling.
- `../environment/skybox/`: sky, clouds, sunlight, indirect illumination, fog, and postprocessing.

Each package embeds its own assets. Original packs in `assets/textures/terrain/` remain
available for USD-relative paths. Neighbouring fields share terrain collision and one sky.

## Select a layout

The default is all harvested wheat (straw stubble). From the repository root:

```sh
GEARBOX_FIELD_LAYOUT=bin/gearbox/src/fields/layouts/mixed.json make run
GEARBOX_FIELD_LAYOUT=bin/gearbox/src/fields/layouts/grassland.json make run
```

Use `BACKEND=wayland` and `WAYLAND_DISPLAY=wayland-0` where required by the desktop.

A layout chooses a `default` profile for uncovered terrain and optional named rectangular
`fields`. Coordinates are world XZ metres. Fields must have finite, positive bounds inside
the procedural terrain (currently -400 to +400 m on each axis), unique names, and no overlap.
Names starting with `__background/` are reserved. The distant visual ground uses the default
profile; only the sampled physical terrain receives bounded vegetation and wheel maps.
The mixed example deliberately crosses mesh/chunk boundaries.

Layout and `EnvironmentSettings` are startup configuration, not live-editable settings.

## Add a package

1. Create a sibling folder with `mod.rs`, `shaders/`, and `textures/`.
2. Its plugin registers embedded assets and its typed Bevy material plugin.
3. Register a `FieldProfile`: name, material factory, vegetation layers, wheel response.
4. Each vegetation layer supplies a mesh template, shader, density, and radial fade distances.
   Use the shared vegetation uniform layout and wheel sampler; clip roots to field bounds.
5. Add the plugin in `FieldsPlugin`, then reference its profile name in a layout.

No terrain-type switch, collision changes, or controller changes are required. The material
factory receives that instance's wheel texture, sampling parameters and shared surface-height
samples. Ground and vegetation
share its map; other field instances use different textures, even for the same profile.
Rectangular maps allocate in proportion to field area, rather than a full terrain map per field.

## Existing USD workflow

`examples/python/bale_run_multi.py` loads `assets/world/terrain.usd`. The harvested-wheat package
preserves its previous material override, authored tint, and cut-hay rows. Loading USD terrain
retires procedural field entities and their renderer resources with the procedural terrain.
The legacy USD adapter is material-only: mixed layouts, generated stalks, and field wheel maps
currently apply to the procedural heightfield, not arbitrary imported USD meshes.

## Validation and limits

### Foliage light response

Grass and straw retain separate diffuse reflection/transmission lobes in Bevy's PBR shader.
Near leaves transmit 45% and dry stalks 25%; distant aggregate clumps retain their ground-matched
35%/15% response. Foliage shading normals keep their sky-facing component on both sides, while
shadow receiver bias follows the terrain normal. Blade roots use shaded vegetation albedo
rather than soil-black gradients; wheel compression/darkening is unchanged.
This is a thin-sheet approximation, not volumetric subsurface scattering or emissive fill.
The two-sided treatment follows the distinction described in
[NVIDIA's SpeedTree leaf-lighting reference](https://developer.nvidia.com/gpugems/gpugems3/part-i-geometry/chapter-4-next-generation-speedtree-rendering).

Regression cases cover region coverage, clipped sloping geometry, and conservative radial LOD
budgets. They have been added but not run under the current no-tests instruction.
Live checks on the RTX 4080/Vulkan preview verified both materials and vegetation shaders,
the mixed boundary (including a low-angle view), independent wheel-map uploads on both
sides of a tractor straddling the boundary, and visible wheat tracks. Loading the multi-bale
USD terrain applied its legacy material and retired the procedural ground without render errors.
The scene-wide sky also rendered in the mixed view. These are manual checks, not automated tests.

The wheat material shares one sampler across its four ground textures to stay within the
renderer stage limit when combined with Bevy's PBR bindings.

## Near-to-far coverage

Primary grassland blades retain full density through 4 m and inverse-square thinning to 128 m.
Secondary grassland plants retain full density through 32 m and thin to zero at 144 m
with their own inverse-square tail and quadratic outer fade. Distant clusters use
80 instances/m² before LOD (scaled by the profile density override), inverse-square thinning
from 12 m to 360 m, and a gradual 8–20 m entrance. Each cluster has two crossed quads with
procedural grass tips or flat-cut straw, stable world-space orientation and wheel deformation.
Grass clusters lean outward and retain coverage in overhead views; straw clusters still
decrease toward overhead views, where the ground surface supplies coverage.
These distances are camera-to-root distances; CPU chunk bounds remain conservative.
The shared `canopy::FADE_END_M` controls both distant layers. Their per-clump fade spans
12% of the clump's cutoff distance (at least 4 m); individual near blades retain their 4 m fade.
Before the per-clump fade, the default distant density is approximately 2.38 clumps/m² at
60 m, 0.38 at 120 m and 0.024 at 240 m, reaching zero at 360 m.

Ground fibers fade to their average coverage when smaller than a pixel. Ground textures have
linear-light mip chains, 8× anisotropic filtering and explicit pre-wrap sampling gradients.
Surface normals sample the terrain normal grid per pixel rather than depending on mesh LOD.
The canopy-lighting approximation averages four inclined leaf normals with the soil response;
it uses the existing sunlight, diffuse transmission, shadows and fog, not baked sunlight.
Pressed areas reduce that canopy contribution. Imported USD terrain without surface samples
retains its mesh normals.

Distant clusters use terrain-oriented aggregate lighting and a shallow root-to-tip colour
gradient. Their lighting normal is independent of the camera-facing direction; the close
blades retain their individual normals and darker roots.

Technique references: [AMD grass LOD](https://gpuopen.com/learn/mesh_shaders/mesh_shaders-procedural_grass_rendering/),
[Far Cry 5 distant grass](https://media.gdcvault.com/gdc2018/presentations/TerrainRenderingFarCry5.pdf),
and [Croteam ground/vegetation matching](https://media.gdcvault.com/gdc2019/presentations/Ladavac_Alen_Four_Million_Acres.pdf).

Track timestamps retain the existing one-hour wrapped clock; expired stamps can recur after a
clock wrap. This is not persistent agronomic state. Track resolution is eight texels per metre.
