# World-space drawing direction

## User direction

- **Superseded direction, 2026-11-09:** Death Stranding is completely excluded.
  Use **Halo Infinite primarily**, with **Halo 5: Guardians** as a secondary
  waypoint reference. The shared visual specification is
  [`DRAW_HALO_DESIGN_SYSTEM.md`](DRAW_HALO_DESIGN_SYSTEM.md).
- The user's leading reference is
  [Omer Younas' campaign-menu breakdown](https://www.behance.net/gallery/208543077/Official-HALO-INFINITE-Campaign-Menu-UI).
  Adapt its typography, frame construction, icon hierarchy and model detail
  to world drawings; do not copy its full-screen menu layout.
- **Labels remain the highest priority.** The fading blue-band label attempt
  was rejected, as were glow-first additions that lacked structural detail.
  Earlier approval of a quieter trail scene is not approval of the later
  extension or evidence that its glow settings are a coherent final system.
- Unify every drawing family, including labels, panels and TF, around shared
  components. More detail must not mean more blur, bloom or glare.
- Show intermediate native previews during implementation, so the user
  can guide the design before a whole-catalogue rollout.
- **No game HUD.** This is a robot simulator: no visor, minimap, ammo/health
  bars, screen-fixed targeting reticles or full-screen game interface.
- All additions belong to terrain, paths, robots, obstacles, destinations or
  actual measurements in the world. Existing drawing APIs and data semantics
  remain authoritative.

## Current status

Implementation resumed at the user's explicit request. After rejecting the
first mixed preview, the user approved the stronger white/color/black glass
and current visual direction, and requested continued full-system unification.
Energized elements remain, excessive glare is bounded, and the spaced native
exhibition covers 103 bays and 12 sections. See
[`DRAW_HALO_IMPLEMENTATION.md`](DRAW_HALO_IMPLEMENTATION.md) for current source
and validation evidence.

## Existing extension contracts and historical implementation notes

The sections below record the earlier extension's API and correctness work,
not acceptance of its appearance. Preserve useful geometry/lifecycle fixes
while replacing the rejected styling according to the Halo specification.

The extension adds five public helpers to the Rust, Python and CLI drawing
APIs, composed from existing retained entity kinds:

| Helper | World geometry | Caller owns |
| --- | --- | --- |
| `terrain_marks` | Batched hollow squares, crosses, triangles or diamonds | Positions, symbol class, size and color |
| `route_stations` | Thin route with regularly spaced crossbars and chevrons | Actual route points, station spacing and size |
| `destination` | Broken ground rings, upright open-diamond sign, leader and label | Pose, radius, label and color |
| `clearance_box` | Broken 3D corner brackets and optional label | Center pose and full XYZ extents |
| `field_volume` | Transparent rectangular grid boundary | Pose, full XYZ extents, grid spacing and color |

`FIELD` is an explicit style at value 20; all previous style values and the
`gearbox.draw.v1` wire layout remain unchanged. The surface uses a filtered
major/minor grid, sparse junctions and slow decorative activity. Its animation
does not change the volume's measured bounds or imply sensor acquisition.
Disabling pulse stops motion; disabling glow retains the static structure.

Every composite has stable child identities under `ns/id` and uses native
client chunking. Drawings retain normal world depth behavior, frame/lifetime
semantics and scoped clear/replacement. Status colors must come from real
input data or explicit fixture values; decoration must not invent a safe
route or a hazard. No game textures, logos or ripped icons are shipped.

Rigid annotations sample ground once at their pose anchor; the supplied height
above terrain is retained. Pitched/rolled helpers receive a world-vertical
ground translation, rather than moving along their tilted local Z. Terrain
sample marks, route segments and destination ground rings follow the ground
pointwise. These choices are explicit properties and do not change old draws.

## Using the extension

The Python helpers follow the existing `gb.draw` status/queue contract:

```python
gb.draw.terrain_marks("survey", "samples", positions,
                      glyph="square", size=0.25, color="cyan")
gb.draw.route_stations("planning", "route", route_points,
                       spacing=1.0, size=0.4, color="cyan")
gb.draw.destination("tasks", "dock", (5, 3, 0.1, 0.4),
                    radius=0.8, label="Dock A", color="green")
gb.draw.clearance_box("robot", "bounds", (0, 0, 1.0, 0),
                      size=(2, 1, 1.8), label="Required clearance")
gb.draw.field_volume("sensor", "coverage", (0, 0, 1.5, 0),
                     size=(4, 3, 2.5), spacing=0.75, pulse_hz=-1)
gb.draw.clear("sensor/coverage")
```

Rust exposes the same builders from `wire::draw_widgets`, returning entities
for `Client::draw`. CLI commands are `terrain-marks`, `route-stations`,
`destination`, `clearance-box` and `field-volume`; each provides `--help`.
`FIELD` supports BOX and RECT only. Other primitive kinds retain PLAIN behavior.

`size` for terrain marks is full symbol span; destination radius is the outer
ground ring; box sizes are full XYZ extents, not half sizes. Glyphs are XY
geometry and the upright destination sign lies in local XZ, not a billboard.
Route spacing is three-dimensional distance on the supplied polyline before
terrain draping. Nonfinite route samples break the route, rather than drawing
connections across missing data. Builders process at most 65,536 input samples
and generate at most 4,096 route stations. Invalid dimensions clear the affected
stable children. An empty label deletes its old label child. Explicit transparent
black is encoded as alpha-zero `1`, because wire color `0` means automatic color.

For intermediate native review:

```sh
nix develop --offline --command oslo make build
nix develop --offline --command oslo make build-draw-gallery
nix develop --offline --command oslo make preview-draw --args=survey
# Switch the same dedicated preview without rebuilding:
nix develop --offline --command oslo make preview-draw --args=spatial
```

The preview remains open. It restarts only its own instance when the built
viewer binary changes. It never clears namespaces outside `vfx-gallery`.

## Accepted first-pass checkpoint

- 23 API/store + 6 gallery + 32 renderer tests passed.
- Fourteen real native catalogue scenes, a 7.25-second hero recording and
  actual point/pose/handle/Step events were captured under `target/draw-vfx/final`.
- Day/night, near/far/grazing, disabled controls and 20,000-point/480-rollout
  views were captured under `target/draw-vfx/final-variants`.
- The gentler-glow adjustment passed the same 61 tests and a new native hero
  capture with live interaction checks under `target/draw-vfx/gentler-glow`.
- Flat-shaded and subset machine meshes now appear in the actual ghost trail.
- A baseline-reproduced startup slab-allocation diagnostic remains; it is not
  a drawing shader validation failure. Broader untested variants remain listed
  in the verification record.

The current Halo implementation and its native evidence supersede that
historical first-pass checkpoint. Coverage and remaining limitations are
recorded in `DRAW_HALO_IMPLEMENTATION.md`; no whole-system user approval or
controlled performance acceptance is inferred from the earlier images.
