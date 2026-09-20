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

The default is the bundled `mixed.json` (grassland with a harvested-wheat stubble region).
From the repository root:

```sh
GEARBOX_FIELD_LAYOUT=crates/gearbox-fields/src/layouts/grassland.json make run
GEARBOX_FIELD_LAYOUT=crates/gearbox-fields/src/layouts/harvested_wheat.json make run
```

`make run` detects Wayland/NVIDIA on its own each launch; no `BACKEND`/`WAYLAND_DISPLAY`
setup is needed.

A layout chooses a `default` profile for uncovered terrain and optional named rectangular
`fields`. Coordinates are world XZ metres. Fields must have finite, positive bounds inside
the procedural terrain (currently -400 to +400 m on each axis), unique names, and no overlap.
Names starting with `__background/` are reserved. The distant visual ground uses the default
profile; only the sampled physical terrain receives bounded vegetation and wheel maps.
The mixed example deliberately crosses mesh/chunk boundaries.

A field may also carry `"wear"`, nought to one, saying how far the wheels have taken it
back to bare ground. It says how hard a way is used, while **Ways** below says where it runs
and how wide. Left out, the profile decides. One `track` profile therefore covers every kind
of way:

```json
{ "name": "green-lane", "profile": "track", "wear": 0.25, "min": [-60, -120], "max": [-54, 120] },
{ "name": "bare-road",  "profile": "track", "wear": 1.0,  "min": [ 44, -120], "max": [ 50, 120] }
```

### The stages a way passes through

Wear is one number, but a way does not change evenly along it. It passes through stages, and
`cover.wgsl` holds where each gives way to the next so the ground, the grit lying on it and
what still grows in it all turn together rather than each on its own schedule:

| wear | what it reads as |
| --- | --- |
| below `WAY_BRUISED` (0.25) | the sward is crushed and darkened, no earth showing — a tractor that went over twice |
| `WAY_BRUISED`..`WAY_METALLED` | earth through the ruts, grass holding down the middle — a soft farm track |
| above `WAY_METALLED` (0.55) | stone comes up through the fines and the middle goes too — a made road |

Nothing but `wear` chooses between them, so the same way authored twice at two numbers is
the same road at two ages. `layouts/stages.json` lays all five side by side to be looked at
at once.

Every way also carries a **damp verge**: a narrow darker line at its outer edge, where water
stands in the lip of the hollow and mud comes off the tyres. Without one a road meets the
field on a clean line however well the two are blended, which is what reads as a road laid on
top of a field rather than worn into it. It is deliberately narrow — widen it and it reaches
the crown between the ruts, and a half-worn track flattens into one dark band instead of two
ruts with grass up the middle.

The wear is read by the ground material and by everything standing in it alike, so it
decides the soil's colour as it wears down to the stone under it, whether grass holds,
how thickly stones lie, and how far the wheels have pressed it. Driving over a field wears
it the same way, whatever its layout says: the two are taken together, the harder winning.
That holds on every cover that can wear — bare ground, meadow and stubble alike — though a
wheel bares turf more gently than it bares soil, and the blades are not taken away for it:
they lie flattened over what shows through, which is what a fresh tyre mark on grass is. A
concrete yard is the exception and wears not at all.

One map, two clocks. A flattened sward springs back over the profile's own
`recovery_seconds` — five minutes for a meadow — but the bare scar a wheel presses in greens
over far more slowly, so `wheel_scar` reads the same marks on a much longer one. Without it
a layout's wear stayed for good while a driven mark evaporated, though both are meant to be
the same rule. Nothing outlasts the stamp clock itself, which spans an hour: a mark driven
into a field lasts a working session, not for ever. A record that truly never fades wants a
map of its own.

`tracks.json` keeps a `drive-me` lane at positive coordinates, barely worn, to drive along
and watch a wheel mark wear it bare:

```sh
gearbox spawn machine bin/gearbox/assets/tractor.usd --machine tug --at 83 0
gearbox scene play
gearbox machine move tug --forward 3 --for 20s
```

See `layouts/tracks.json` for the range of wear, and `layouts/between.json` for a lane
with a different field up each side: each of the four sides is washed towards its own
neighbour, so the same lane is dark where it meets ploughed earth and pale where it meets
sand.

## Ways: a road that is not the shape of a field

A field is always a rectangle, so on its own a `wear` can only run straight down one. A
**way** is the line the wheels actually follow, and the field becomes merely the corridor it
winds along inside. A field names its own with `way` (world XZ points, at most sixteen) and
`way_width` (metres across the worn part; left out, as wide as the field):

```json
{ "name": "winding-lane", "profile": "track", "wear": 0.7,
  "min": [96, -120], "max": [120, 120], "way_width": 6.0,
  "way": [[112, -120], [115, -86], [110, -52], [104, -17],
          [103, 17], [108, 52], [113, 86], [110, 120]] }
```

The ruts themselves run at a tractor's gauge, a little under two metres apart, whatever the
way is worn to: `wear` says how bare each rut and the ground between them is, not how far
apart they sit. A way too narrow to hold them — a footpath, a quad track — closes them
towards its middle instead, so it wears as one strip rather than as two ruts mostly outside
its own verge. `layouts/narrow.json` puts 1.2 m, 2.4 m and 6 m ways side by side.

A road that outlives one field goes in the layout's own `ways` instead, and wears every
field it crosses — whatever that field grows:

```json
"ways": [
  { "name": "farm-track", "width": 5.5, "wear": 0.5,
    "points": [[-160, -20], [-110, -26], [-60, -18], [-20, -6],
               [20, 4], [70, 10], [120, 6], [170, -4]] }
]
```

The bundled `mixed.json` carries exactly this lane, so it is in the scene on a plain launch,
running out of the meadow and across the stubble field.

Each field is given the stretch of the road it can see, cut from the same points and never
resampled, so the two halves meet exactly on a boundary. The ruts take their wander from the
nearest point of the line itself, which is a place in the world and so the same from either
side, so nothing has to be told how far along the road it lies. A field that names no way of
its own is worn down its long axis exactly as before.

Where two roads cross the same field, both wear it and the harder of them wins, each keeping
its own width **and its own wear** — a faint field track may join a metalled lane without
either becoming the other, which is what `layouts/junction.json` crosses. The fields a way
passes over have nothing to say about how used that way is: only a field that bends no line
through itself may carry a `wear`, and that wears it all over.
**Sixteen points are shared between the two**, so a junction fits when the two
stretches a field can see come to sixteen points between them; beyond that the crossing road
is left out whole, with a warning saying so. Trimming either line instead would leave two
neighbouring fields describing the same road differently and it would kink between them. The
same sixteen are the limit for a single road, and a field needing more of them is warned and
cut short. Both are the one budget: a way's points live in two matrices in the material and
vegetation uniforms. `layouts/junction.json` crosses a lane and a drove over grass and
stubble, five points each.

A road crossing open ground keeps most of its points, because the background outside the
named fields is cut into a few large regions. So the budget is spent quickly by long roads:
prefer as few points as the shape needs, and split a field if a warning says a road was cut.

Two is also the most that can **wear** one field. A third road over the same ground is
refused with a warning, but the hollow knows nothing of that limit and still sinks along it,
so a triple crossing leaves a shallow grassy trough where the third road is not painted. It
reads as a disused track rather than as damage, but it is not what was asked for: keep to
two ways over any one field, or cut the field so each crossing has its own.

A way may also run straight **along** a boundary rather than through a field: each side wears
its own half, the hollow sinks across both, and nothing seams down the middle —
`layouts/boundary.json` lays one between a ploughed field and stubble. The two halves wear to
the same stone and reach it faster than the wear itself rises, so a made road matches across
its width; only a faint one keeps a little of each field's own earth, which is all a faint
mark is.

An authored way also **sinks the ground it runs over** — a little under a foot for a
half-worn farm track, tapering out over a metre and a half either side, and proportionately
less for anything narrower, so a footpath wears a groove rather than a shallow valley wider
than itself. That is the terrain grid,
which carries the collider as well as the mesh, so the hollow is felt by the wheels and not
only seen. A field carrying a `wear` but no way is not sunk: worn across its whole width
means shading, not a trench dug down the middle of it.

Every surface reads the wear the same way — meadow, stubble, ploughed and bare all wear down
through the same stages to the same hardcore, so a made road does not change where it leaves
one field for the next. Each keeps its own **subsoil**, though, which is what a soft track is
mostly showing: a lane reads a shade darker crossing a meadow than crossing stubble, the way
a real one carries the earth of the field it has just come off. `layouts/road.json` runs one
lane across all three.

Fields meet one another over the softer of their two `soft_border` widths: what grows in
each carries that far past its own edge and thins as it goes, and the ground washes towards
the neighbour's colour over the same distance. A yard's border is nought, so it ends on a
line.

Layout and `EnvironmentSettings` are startup configuration, not live-editable settings.

## Add a package

1. Create a sibling folder with `mod.rs`, `shaders/`, and `textures/`.
2. Its plugin registers embedded assets and its typed Bevy material plugin.
3. Register a `FieldProfile`: name, material factory, vegetation layers, wheel response,
   and the `surface_tint` and `soft_border` by which its neighbours meet it.
4. Each vegetation layer supplies a mesh template, shader, density, and radial fade distances.
   Use the shared vegetation uniform layout and wheel sampler; clip roots to field bounds.
5. Add the plugin in `FieldsPlugin`, then reference its profile name in a layout.

### What a new ground has to read

A cover that skips any of these is not wrong anywhere a compiler can see; it is simply the
one field in a layout that behaves unlike the rest, which is how the stubble went a long
while washing towards nothing while its neighbours washed towards it.

- **Everything in `Placed`.** The factory is handed where the field runs, what lies across
  each of its four sides and how far, how worn it is, and the way through it. `extent` and
  the four tints with `reach` feed `washed_into`; `Placed::tread` gives the tread, carrying
  a wear for each of the two lines; the way rides in as two matrices and a shape.
- **`worn()` and `washed_into()` from `bare/shaders/cover.wgsl`.** Never a second copy: the
  two grounds either side of a join must agree where it falls, and held apart they drift —
  one edge ragged, one ruled.
- **`bare::WAY_SOIL` and `bare::WAY_HARDCORE`** as what a way wears the cover down to, or a
  road changes colour where it crosses onto it.
- **The wheel press, folded in with `max`.** Driving and the layout are taken together and
  the harder wins; a cover that reads only its layout never wears under a wheel.

A cover that genuinely does not wear — a concrete yard — says so by leaving its tread at
nought, which `worn()` answers on its first line. That is a decision, not an omission.

Adding a field to `VegetationParams` means declaring it in **every** shader that names that
struct, or the binding size stops matching what is written. The same goes for a material's
own uniform: its Rust struct and its WGSL `struct` are one declaration kept in two files.

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
