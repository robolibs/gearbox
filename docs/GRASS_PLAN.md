# Grass plan

The meadow looks right and costs far too much GPU on an RTX 4080. This plan
keeps the look and moves the work to where shipped engines put it. It comes
from a code read of `crates/gearbox-fields` and a web research pass over
shipped games, engine docs and papers (2026-09-25). **Nothing here has been
measured yet**; every payoff below is an expectation from the mechanism, and
Step 0 exists to replace it with numbers.

## The short version

Shipped games make dense grass cheap with four habits:

1. Decide on the GPU, in compute, **which blades exist before any vertex
   shader runs**, and do every per-blade calculation once per blade.
2. Keep blades **truly opaque** (no `discard`, no alpha-to-coverage) so the
   depth test rejects hidden pixels before they are shaded.
3. Light blades with **one cheap lobe**, never full PBR several times.
4. Stop drawing blades a few tens of metres out and let **terrain colour**
   take over.

Our blade is already the Ghost of Tsushima blade (clump pull, twin folds,
Bézier shape, edge-on thickening, widen-and-thin below 1.2 px in `got_blade`).
What we lack is its front end. Ghost of Tsushima considers about 1 M blades a
frame, draws about **83 000** (15 vertices near, 7 far, 64 B per-blade record
written by compute) and spends about **2.5 ms** on grass end to end. Our 8 m
ring around the camera alone issues roughly **16 M vertex invocations** before
anything is culled.

## Status and measurements (2026-09-26)

Measured on the RTX 4080 with a private probe instance (`gearbox run --name
probe --ephemeral`), GPU pass times from `GEARBOX_RENDER_DIAGNOSTICS=1
MARA_GPU_TIMESTAMPS=1`, readings taken after the weather bake settles. Poses:
*default* (the start view) and *cab* (`instance camera goto 52.369677
4.896051 2.5`, 2.5 m above the meadow).

| | Vegetation pass (`main_transparent_pass_3d`) | Frame | fps |
|---|---|---|---|
| Baseline, default pose | 71.1 ms | 82.7 ms | 12.1 |
| Baseline, cab pose | 80.7 ms | 92 ms | 10.9 |
| Now, default pose | ~10 ms | 24.5 ms | ~40 |
| Now, cab pose | 10.3–10.6 ms | 24.5 ms | ~40 |
| Vegetation switched off (floor) | 0.01 ms | 24–25 ms | ~40 |

The frame is now at the floor it has with no vegetation at all: the
remaining ~24 ms is outside the grass (the listed GPU passes add to ~5 ms, so
it is CPU or unlisted passes). Screenshot diff at the cab pose against the
baseline: mean 0.62/255 over the ground, 0.07% of pixels off by more than
24/255 — the look is unchanged.

Where the 71 ms went (triage, default pose, each group drawn alone):

| Layers | Before the work below | Note |
|---|---|---|
| Road grit + litter (`way_grit`, `way_litter`, 5200 + 2600/m², 54/20-vertex lumps) | **32.6 ms** | ~1.3 M candidates per 16 m chunk the track touches, nearly all culled in the vertex shader |
| Poly Haven clumps | 6.2 ms | |
| Tufts + stubble straw | 2.6 ms | |
| Canopy cards | 1.0 ms | |
| Flowers + stubble leaves | 0.7 ms | |
| Grass blades + stubble stalks (after Stage 3) | 0.6 ms | was the bulk before the compute path |
| Everything, fragment shading replaced by flat colour | 54.5 ms vs 49.7 lit | lighting was never the cost |

### What is done

| # | Item | Status |
|---|---|---|
| 0 | Measure | Done: harness above; `GEARBOX_VEG_LAYERS=i,j,…` draws only those layer indices, `GEARBOX_VEG_FLAT=1` replaces vegetation lighting with flat colour |
| 1.1 | Cull before the expensive work | Done: chunk budget counted from the layer band's inner edge (`band_budget`); ground-distance early-out in every vegetation vertex path (blades, stalks, tufts, flowers, cards, clumps, bare lumps); flower/clump/stubble keep-tests before the heightmap; `way_only` layers issue nothing where no way reaches |
| 1.2 | Blade pipeline without `discard` / A2C | Done: `VegetationLayer::cutout`; only cards, clumps and bare tufts compile `discard` and use alpha-to-coverage |
| 1.3 | Front-to-back | Done: vegetation sorts ahead of every blended item, nearest chunk first |
| 1.4 | One shadow fetch for the lobes | Done: `surface_lighting` fetches the suns' shadows once and lights base + four lobes unshadowed less what the shadow takes (exact); cards no longer light twice |
| 1.5 | Tint per blade | Done for blades (`meadow_tint` baked into the record at the root); the other layers keep it per pixel — fragment cost is not what limits them |
| 2.1 | Grass lighting | Done as `plant_lighting`: Bevy's lighting with one shadow lookup shared by the light and the distance fog (Bevy's fog fetched a second one per pixel) |
| 2.2 | Cheap back-light | Done, opt-in: `GEARBOX_GRASS_BACKLIGHT=cheap` (−1.2 ms at the cab pose, 1.07/255 change) |
| 2.3 | Cheap shadows past cascade 0 | Done: one hardware comparison tap beyond the first cascade for vegetation |
| 2.4 | Ranges | Done as a knob: `GEARBOX_VEGETATION_RANGE` scales every layer's fade and band (default 1 = the authored look) |
| 2.5 | Depth prepass | Done, opt-in: `GEARBOX_GRASS_PREPASS=1` lays blade depth first, lit pass depth-equal (`@invariant` position). Measured no gain (10.9 vs 10.8 ms): fragments are not the cost |
| 3 | Compute cull + indirect | Done for the meadow's blades (`blades.rs`, `grassland/shaders/blade_cull.wgsl`, `blade_draw.wgsl`): one invocation per candidate root culls (distance, band, pixel width, frustum, way wear) and shapes the blade once into a 64-byte record, one indirect draw bends a 9-vertex strip per record, per view. Stubble stalks stay on the vertex path: with the early-outs they measure well under a millisecond |
| — | Road grit sieve (not in the original plan, found by the triage) | Done (`sieve.rs`, `bare/shaders/grit_sieve.wgsl`): a compute pass runs the grit's first tests once per candidate and the chunk draws only the survivors, indirectly, through its own vertex shader. 32.6 ms → part of the ~10 ms total |
| — | Hi-Z occlusion, instanced vs non-instanced blades | Not done: the blades now cost ~0.6 ms together with the stalks, so neither can move the frame |
| — | Clump sieve | Not done: clumps (~6 ms GPU) are the largest vegetation cost left, but the frame is at its no-vegetation floor; the sieve module takes another shader when it matters |

## Where the cost went (the original read)

| # | Problem | Where | Why it costs |
|---|---|---|---|
| A | Opaque, depth-writing blades are queued in `Transparent3d`, sorted back-to-front by chunk centre | `render.rs:260` | The nearest, densest chunks draw last and overdraw everything already shaded. One measurement: back-to-front raised pixel-shader invocations 47% |
| B | `alpha_to_coverage_enabled` whenever MSAA > 1, on every vegetation pipeline | `render.rs:719` | A2C forces late depth. Blades output alpha 1.0, so they gain nothing from it |
| C | A `discard` is compiled into every blade's fragment shader (only cards need it) | `grassland/shaders/vegetation.wgsl:730` | A compiled `discard` defers depth writes even if it never runs: +32% pixel-shader invocations in one measurement |
| D | Every vertex recomputes all per-blade data: R2 spot, 3×3 Voronoi `clump_of`, 4 heightmap loads, 4 wheel-map loads, 2 wind samples, species/mottle/dryness noise, way-wear polyline search | `got_blade`, `vegetation.wgsl:378-525` | A near instance has 18 vertices, so the same work runs 18 times |
| E | Culled blades die only after that work, as a degenerate vertex | `culled_vertex()`, reached at `vegetation.wgsl:408` | Band, fade, slope and pixel-width tests come after `clump_of` and `sample_field` |
| F | Chunk budget is taken at the chunk's *nearest* point | `instance_budget`, `runtime.rs:608-622`, called at `:717` | A mid-LOD 4 m chunk 3 m from the camera issues its full 72 000 instances only to cull them; a far 16 m chunk around the camera issues ~1.15 M |
| G | Flowers are culled by noise inside the vertex shader after 74 vertices of setup | `flower()`, `vegetation.wgsl:572-596` | Most flower instances produce nothing |
| H | Full `apply_pbr_lighting` per blade fragment, plus the diffuse-transmission defs | `vegetation.wgsl:755`, `render.rs:707-713` | Transmission adds a second directional BRDF, a second ambient term and a second env-map lookup (no second shadow fetch: that needs `TRANSMITTED_SHADOW_RECEIVER`, which is unset) |
| I | Canopy cards run PBR **6×** per pixel: one discarded call, then `surface_lighting`'s base + 4 lobes | `vegetation.wgsl:755-757`, `shaders/surface_detail.wgsl:43-64` | Each call does its own shadow PCF: 9 taps, twice in the 0.2 cascade blend band, so 54–108 taps per card pixel |
| J | The grassland ground under the grass runs PBR **5×** per pixel | `grassland/shaders/material.wgsl:460` | 45–90 PCF taps per ground pixel, fully shaded before the grass covers it |
| K | ~10 gradient-noise evaluations per blade pixel for a tint whose features are 14–60 m wide | `meadow_tint(meadow_pattern(...))`, `vegetation.wgsl:732` | Per-pixel work for a field-scale signal |
| L | Ranges far past what shipped games draw | `grassland/mod.rs:120-202`, `canopy.rs:9` | Blades fade to 128 m, tufts to 144 m, cards to 360 m. At 1440p a 3 mm blade is under a pixel wide past ~4 m and a 10 cm blade is ~4.6 px tall at 30 m |

Default density is 6000 (`lib.rs`), which makes 4500 instances/m² per blade
LOD layer, two blades per instance: ~9000 blades/m² near the camera. The
camera runs HDR, 4× MSAA, bloom, fog and four sun shadow cascades to 800 m
(`bevy_weather/src/sky.rs`). Adding the Poly Haven clumps once took the frame
from ~14 to ~10.7 fps (~22 ms). Their dense packs are light (bermuda 228
triangles, tuft 301, sorrel 420), so that cost is probably fragment-side:
alpha-tested full PBR with transmission, drawn back-to-front.

## What shipped games spend

| System | Drawn per frame | Geometry | GPU cost |
|---|---|---|---|
| Ghost of Tsushima (PS4) | ~83k of ~1M considered | 15 / 7 verts per blade | ~2.5 ms total |
| Jahrmann & Wimmer 2017 (GTX 780M) | 43k of 168k candidates | tessellated Bézier | 1.40 ms cull + 2.06 ms draw (3.87 ms draw without culling) |
| Horizon Zero Dawn (PS4) | ~100k placed objects | 20–36 tris per grass clump | 0.25 ms placement |
| Battlefield 3 (PS3) | all terrain decoration | clump meshes | 1–4 ms |
| Helio, wgpu/WGSL (stated budget) | 1M blades | 11 / 7 / 4 verts | < 3 ms at 1080p |

Where the blades stop: Ghost of Tsushima switches to an artist-painted
terrain texture after two blade LODs; Far Cry 5's last LOD is quads that copy
terrain height and colour; Black Ops 4 shrinks blades and sinks them into
matching terrain; Epic's open-world example culls field grass at 65 m;
Unity's detail distance defaults to about 80 m; Helio stops blades at 45 m.

## The rule that keeps the look

Moving a calculation between stages does not change pixels: the same
deterministic functions run on the same inputs, whether in compute, per
blade, per vertex or per pixel. Everything in Stage 1 and Stage 3 is of that
kind. **Only the range cuts in Stage 2 change what is seen**, and the colour
hand-off they need already exists: the ground (`material.wgsl:340`) and the
blades (`vegetation.wgsl:732`) read the same `meadow_pattern`.

Every step is judged by screenshot diff from fixed poses, and by counters.

## Step 0 — measure first (one hour)

The biggest unknown is whether the frame is vertex-bound or fragment-bound.
It decides which Stage 1 item comes first.

- Private instance, never the user's window: fixed pose through
  `gearbox instance camera -i probe goto LAT LON HEIGHT` once it answers (the
  `GEARBOX_CAMERA_*` variables no longer reach the camera: the view is driven
  by `View`, which overwrites the chase rig every frame), fixed sun through
  `GEARBOX_SUN_ELEVATION` / `GEARBOX_SUN_AZIMUTH`, clear sky through
  `GEARBOX_CLOUD_COVER=0`. Let chunk streaming and the weather bake settle
  (about three diagnostic rounds) before recording.
- Three poses: cab height looking along the meadow, a mid-meadow vista, and
  a low back-lit sun.
- Confirm the frame is GPU-bound: frame time against summed GPU pass time.
- Counters: wgpu `PIPELINE_STATISTICS_QUERY` (vertex invocations, clipper
  primitives, fragment invocations) and `TIMESTAMP_QUERY_INSIDE_PASSES`
  around the vegetation draws. Nsight Graphics or RenderDoc for per-draw cost.
- Median over hundreds of frames; check machine load, fps here is noisy.
- Triage toggles, one at a time:
  - flat-colour grass fragment → the fragment share;
  - `GEARBOX_GRASS_DENSITY` halved → vertex and raster scaling;
  - 50% render scale → the fill-rate share;
  - clump layers off → what the clumps really cost.

Record the baseline in this file before changing anything.

## Stage 1 — invisible fixes (hours each)

### 1.1 Cull before the expensive work (fixes E, F, G)

- **CPU.** At `runtime.rs:717`, issue zero instances for a chunk whose
  *farthest* point (`farthest_distance`, `runtime.rs:630`) is still inside the
  layer's inner band edge, and for a chunk whose nearest point is past the
  outer edge. The band test in `stream_vegetation` already exists; this makes
  the instance count follow it.
- **GPU.** At the top of `got_blade`, before `clump_of` and `sample_field`,
  run a conservative test on the unclumped `spot`: distance bounded with the
  chunk's height span (add low/high to `VegetationParams`), padded by the LOD
  jitter (`LOD_JITTER_M`) and the twin offset (≤ 3 cm; `CLUMP_PULL` is 0).
  Drop a blade only if it fails the band, fade or `rand(id, 19u) * widen`
  test even at its most generous distance. Survivors take the existing exact
  path, so they are unchanged.
- **Flowers and tufts.** Move the `opening` noise test to the top of
  `flower()` and the fade test to the top of `meadow_detail()`.
- **Check:** vertex invocations doing heavy work fall roughly with the
  instances culled early; screenshots are pixel-identical.

### 1.2 Split the blade pipeline from the alpha pipeline (fixes B, C)

- Add a key bit to `VegetationPipeline::specialize` (`render.rs:696-721`):
  blades specialise **without** alpha-to-coverage and without a
  `MAY_DISCARD`-style shader def that guards the `discard` at
  `vegetation.wgsl:730`. Canopy cards and clumps keep both.
- This is Bevy's own pattern: `MAY_DISCARD` exists only for masked
  materials, and opaque materials never compile a `discard`.
- Cards and blades share one shader and one mesh today (the canopy is a
  template of the grassland layer). The key comes from the layer, so the
  canopy layer gets the alpha variant and the blade layers the opaque one.
- `@early_depth_test(force)` is not a shortcut: with depth writes on,
  discarded fragments would still write depth.
- **Check:** fragment invocations fall; blades render bit-identically.

### 1.3 Draw front-to-back (fixes A)

- Reverse the chunk order so the nearest chunks draw first. Two routes:
  1. stay in `Transparent3d` and invert the sort key for the blade items
     (untested inference from Bevy's sort code) — the smallest change;
  2. queue the opaque blade variant as a custom item in the binned
     `Opaque3d` phase, which accepts non-mesh items in Bevy 0.19. Binned
     phases order by pipeline and bin, not distance, so this only pays once
     1.2 has restored early-Z and a measurement shows order no longer
     matters much. Stage 3 lands here anyway.
- **Check:** fragment invocations fall; screenshots identical.

### 1.4 One shadow fetch for the lighting lobes (fixes I, J)

- `surface_lighting` (`surface_detail.wgsl:43-64`) calls
  `apply_pbr_lighting` five times, and the lobes differ only in normal.
  Evaluate the base once with its shadow, then add each flank lobe as
  `max(dot(N_i, L), 0) * light * shadow`. Skip the lobes entirely when the
  `lobes` weight is zero (low sun).
- In the grass fragment (`vegetation.wgsl:755-757`), do not run the base
  `apply_pbr_lighting` for cards before throwing it away.
- Same change serves the ground (`grassland/shaders/material.wgsl:460`) and
  the stubble (`harvested_wheat/shaders/material.wgsl:595`,
  `harvested_wheat/shaders/vegetation.wgsl:549`).
- **Check:** near-identical screenshots (only the shadow's normal-offset
  bias differs per lobe); card and ground pixel cost drop the most.

### 1.5 Tint per vertex (fixes K)

- Compute `meadow_tint(meadow_pattern(...))` at the blade root in the vertex
  shader and pass it in `VertexOutput`. Its features are metres wide, a blade
  is millimetres wide.
- Later: bake `meadow_pattern` into a low-resolution world texture read by
  both the blades and the ground material.
- **Check:** near-identical screenshots.

## Stage 2 — shading and ranges (days)

### 2.1 A grass lighting function (fixes H)

Replace `apply_pbr_lighting` in the blade, card and clump fragments with a
grass-specific function, as shipped grass shaders do (Crysis shaded grass per
vertex for fill rate; Unity URP grass is Blinn-Phong with specular 0 and
per-vertex SH ambient and fog; HDRP shares one ambient probe for all grass;
Horizon fixes reflectance and uses one hacked normal):

- one directional (sun) evaluation, one shadow fetch;
- hemisphere or SH ambient and the fog factor from the vertex;
- no clustered point/spot lights, no rect lights, no env-map specular (if
  headlights on grass matter, evaluate them per vertex);
- keep `foliage_normal` and the distance settling, which already stop glitter.

Risk: moderate. Tune against the three poses, especially the low back-lit sun.

### 2.2 A cheap back-light instead of Bevy transmission (fixes H)

- Drop `STANDARD_MATERIAL_DIFFUSE_TRANSMISSION` and
  `STANDARD_MATERIAL_DIFFUSE_OR_SPECULAR_TRANSMISSION` (`render.rs:707-713`)
  and add a DICE-style thin translucency term (about 12 ALU instructions,
  0.03 ms full screen on DX11 PC in DICE's measurement) or Crysis's
  wrap-plus-view term.
- Decision for the user: today's transmitted lobe is unshadowed, so blades
  in a machine's shadow still glow from behind. Keep that (same look) or
  multiply by the front shadow (more correct, changes shaded areas).

### 2.3 Cheaper shadow receiving past the first cascade

- Past the first cascade (40 m in `sky.rs`) one hardware PCF tap per grass
  fragment is enough; blade noise hides filter quality. Horizon's main
  foliage cascade ends at 80 m; ours receives across four cascades to 800 m.

### 2.4 Pull the ranges in (fixes L) — the step that changes the look

Every distance becomes a knob, tuned by A/B screenshot at the user's
favourite viewpoints. Suggested starting points:

| Layer (`grassland/mod.rs`) | Density | Range now | Start at |
|---|---|---|---|
| Blade LODs near / mid / far | 4500 inst/m² each | fade 8 → 128 m | fade end 45–60 m |
| Detail tufts | 80/m², 54 verts | 32 → 144 m | end 60–80 m |
| Flowers | 36/m², 74 verts | 6 → 45 m | keep; far colour into the ground |
| Canopy cards (`canopy.rs:9`) | 80/m² | 12 → 360 m | end 45–60 m, or drop for ground colour |
| Bermuda / tufts / sorrel clumps | 120 / 50 / 30 per m² | to 32–40 m | end 20–30 m |
| Dandelion / nettle / celandine | 2.5 / 0.8 / 1.0 per m² | to 34–40 m | end 20–30 m |

Hide the hand-off the way shipped titles do: blades already fade by stable
rank and widen as they thin; past the blade range the ground's
`meadow_pattern` colour carries the field. Shrink or sink rather than fade
alpha. Watch a known trap: from eye level a short cutoff on a dense
small-scale layer shows as a hard band across the field (a 60 m cut on the
tuft layer was rejected for exactly that), so every layer needs a long fade,
not a short one.

### 2.5 Depth prepass — only after 1.1 (and ideally Stage 3)

- Blades: depth-only prepass with the cheapest possible vertex shader, then
  the lit pass with an EQUAL depth test. Cards and clumps: alpha-tested
  depth-only prepass, then a lit pass with no `discard` (Horizon Zero Dawn's
  recipe: "Very Cheap Depth Only Shader", then "Zero percent Overdraw!").
- It also lets the ground's lighting be rejected wherever grass covers it.
- Horizon Forbidden West later dropped this because it "transforms all the
  geometry twice". With today's vertex shader a prepass would run our
  heaviest stage twice, so it waits until the vertex cost is gone.
- Needs `@invariant` clip position shared by both passes.

## Stage 3 — the Ghost of Tsushima front end (1–3 weeks)

### Shape

1. **Compute, one lane per candidate blade** (per chunk, per LOD): R2
   placement → distance-rank test and the existing pixel-width thinning →
   frustum test on three curve points → Jahrmann's edge-on test → for
   survivors only: Voronoi clump, heightmap, wheel press, wind, species,
   tint, way wear.
2. **Write a packed record** of 16–32 B per surviving blade (root position,
   facing, height/width, clump tone, tint, press/roll, wind lean).
   Compaction: a workgroup-local count, then one global atomic per
   workgroup (Horizon's pre-reduction).
3. **One single-thread pass writes the indirect arguments.**
4. **Draw** with `multi_draw_indexed_indirect_count` (wgpu 29 backs it
   natively on Vulkan 1.2+). The vertex shader only reads the record by
   instance id and evaluates the Bézier, the thickening and the wind bob.
5. **Optional cache:** the static half of a record (placement, clump, ground,
   species, tint) can be kept per chunk and rebuilt only when the chunk
   enters or the wheel map changes. Ghost of Tsushima instead regenerates
   every frame from an 8-tile ring buffer; both are valid.

### Bevy 0.19 pieces

- Compute pass: a system in the `RenderGraph` schedule ordered
  `.before(camera_driver)`, as in Bevy's `compute_mesh` example.
- Template: **`bevy_eidolon` 0.5.0** (2026-08-19, Bevy ^0.19). It already
  does a per-instance frustum and distance cull in compute, `atomicAdd`
  compaction into `DrawIndexedIndirectArgs`, `multi_draw_indexed_indirect`,
  and binned `Opaque3d` + `Opaque3dPrepass` queueing. It has no Hi-Z and no
  shadows. Read it before writing ours.
- Clump variants are encoded in `first_instance` today
  (`render.rs:832-837`); indirect draws need `INDIRECT_FIRST_INSTANCE`,
  which the 4080 has.
- Everything `worn()`, `cover.wgsl` and `shader_contract.rs` guard stays the
  same function; it just runs in compute. Keep the contract tests passing.

### To measure, not assume

- Outerra measured an NVIDIA slowdown below ~80 triangles per instance
  (OpenGL, 2016). Our instances are 14 triangles. Benchmark instanced draws
  against a non-instanced `draw_indirect` that derives the blade from
  `vertex_index / V`.
- Hi-Z occlusion through Bevy's `ViewDepthPyramid` (module marked
  experimental, needs `DepthPrepass`). In open fields it is a small win:
  Jahrmann's occlusion test removed 6 025 blades against 79 533 for frustum,
  and Ghost of Tsushima called it "small". Add it last.

### Target

Grass inside a 2–3 ms envelope at 1440p. A target, not a measurement.

## Not now

- **Mesh shaders.** Not in Bevy 0.19. Mesh-shader pipelines landed on Bevy
  main (wgpu 30) on 2026-09-06, without material support; the author writes
  that they "are not a free performance win". No source measures a grass
  speedup over compute + indirect. Revisit after Stage 3, on Bevy 0.20.
- **TAA / upscalers.** Grass writes no motion vectors (neither does Ghost of
  Tsushima's), so TAA would smear the meadow. MSAA stays.
- **A depth prepass before 1.1.** It doubles the heaviest stage.
- **Visibility buffer / work graphs / Nanite foliage.** Visibility buffer
  saved Horizon Forbidden West ~1.4 ms at 4K on PS5, but it is a renderer
  rewrite; work graphs are not reachable from wgpu; Nanite Foliage is
  experimental and aimed at trees.
- **Variable rate shading.** Little help for thin geometry (disabled with
  depth export, per NVIDIA and AMD); whether wgpu exposes it is unverified.

## Order of work

| # | Item | Effort | Expected payoff | Look risk |
|---|---|---|---|---|
| 0 | Measure: vertex vs fragment, per-layer cost | 1 h | decides the order below | none |
| 1.1 | Cull before work (CPU band skip, VS early-out, flowers) | hours | high if vertex-bound | none |
| 1.2 | Blade pipeline without `discard` / A2C | hours | high if fragment-bound | none |
| 1.3 | Front-to-back | hours | medium–high on fragments | none |
| 1.4 | One shadow fetch for lobes; no discarded card eval | hours | high on card and ground pixels | very low |
| 1.5 | Tint per vertex | hours | medium | very low |
| 2.1 | Grass lighting function | 2–4 days | medium–high | moderate |
| 2.2 | Cheap back-light | ~1 day | medium | low |
| 2.3 | Cheap shadows past cascade 0 | hours | medium | low |
| 2.4 | Pull ranges in, thin clumps | 3–5 days | very high | moderate–high |
| 2.5 | Depth prepass (after 1.1) | 2–4 days | high on fragments | none |
| 3 | Compute cull + indirect | 1–3 weeks | very high | low |

## References

- Ghost of Tsushima, "Procedural Grass in Ghost of Tsushima", Eric
  Wohllaib, GDC 2021 — https://www.youtube.com/watch?v=Ibe1JBF5i5Y
- Ghost of Yōtei tech deep dive (grass output doubled on the same design) —
  https://blog.playstation.com/2025/10/23/ghost-of-yotei-tech-deep-dive/
- Horizon Zero Dawn vegetation, GDC 2018 —
  https://media.gdcvault.com/gdc2018/presentations/gilbert_sanders_between_tech_and.pdf
- Horizon Zero Dawn GPU placement, GDC 2017 —
  https://www.youtube.com/watch?v=_ooDLiU-o6c
- Horizon Forbidden West, "Adventures with Deferred", GDC 2022 —
  https://media.gdcvault.com/GDC+2022/Speaker+Slides/AdventuresWithDeferred_McLaren_James.pdf
- Far Cry 5 terrain, GDC 2018 —
  https://media.gdcvault.com/gdc2018/presentations/TerrainRenderingFarCry5.pdf
- Battlefield 3 terrain, GDC 2012 —
  https://media.contentapi.ea.com/content/dam/eacom/frostbite/files/gdc12-terrain-in-battlefield3.pdf
- Jahrmann & Wimmer, "Responsive Real-Time Grass Rendering for General 3D
  Scenes", 2017 —
  https://www.cg.tuwien.ac.at/research/publications/2017/JAHRMANN-2017-RRTG/JAHRMANN-2017-RRTG-draft.pdf
- GPU Gems 3 ch. 16, Crysis vegetation —
  https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-16-vegetation-procedural-animation-and-shading-crysis
- DICE, approximating translucency, GDC 2011 —
  https://www.slideshare.net/slideshow/colin-barrebrisebois-gdc-2011-approximating-translucency-for-a-fast-cheap-and-convincing-subsurfacescattering-look-7170855/7170855
- Unity URP grass shader source —
  https://raw.githubusercontent.com/Unity-Technologies/Graphics/master/Packages/com.unity.render-pipelines.universal/Shaders/Terrain/WavingGrassPasses.hlsl
- Matt Pettineo, early-Z measurements —
  https://therealmjp.github.io/posts/to-earlyz-or-not-to-earlyz/
- NVIDIA, advanced API performance: shaders —
  https://developer.nvidia.com/blog/advanced-api-performance-shaders/
- AMD RDNA performance guide — https://gpuopen.com/learn/rdna-performance-guide/
- AMD mesh-shader procedural grass —
  https://gpuopen.com/learn/mesh_shaders/mesh_shaders-procedural_grass_rendering/
- Fatahalian et al. 2010, quad-fragment merging —
  https://graphics.stanford.edu/papers/fragmerging/shade_sig10.pdf
- Outerra procedural grass — https://outerra.blogspot.com/2012/05/procedural-grass-rendering.html
- Helio foliage system (wgpu/WGSL) —
  https://pulsarnative.com/blog/2026-08-02-helio-foliage-system
- bevy_eidolon 0.5.0 cull shader —
  https://docs.rs/crate/bevy_eidolon/0.5.0/source/src/cull/compute.wgsl
- Bevy mesh-shader pipelines PR — https://github.com/bevyengine/bevy/pull/25627
