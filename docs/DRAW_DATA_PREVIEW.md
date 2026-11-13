# Spatial data previews

## Current compact cloud preview

From `.work/draw-vfx`, use repository-native build/launch recipes:

```sh
nix develop --offline --command oslo make preview-draw-data
nix develop --offline --command oslo make preview-draw-cloud-clip
nix develop --offline --command oslo make preview-draw-cloud-palettes
nix develop --offline --command oslo make preview-draw-cloud-clocks
nix develop --offline --command oslo make preview-draw-cloud-precision
```

The first scene uploads four real typed clouds through the public writer: raw points,
intensity, classes and measured RGB. The clip review keeps these colors and annotations
while showing an unclipped reference, source-X half-space, intentionally fully clipped
cloud, and frosted height slice. The intentionally empty class bay is labeled
`FULLY CLIPPED / ZERO SELECTED`; its receipt is culled, not an absent upload.

The palette review instead shows four sets of raw-value bands: COSTMAP, MAP, RAW and
OKABE. Each has 3,072 points, including -1/0/99/100/255/256 boundaries; COSTMAP's zero
band is deliberately clear. These are indexed colors, not interpolated gradients.
Labels and energized frames use the existing drawing language.

The source-clock review shows VIRIDIS, TURBO, OKABE and GREY ages. Each bay contains
two aligned rows of 16 captures from unrelated sensor clocks: one near zero and one
near `u64::MAX`. Both rows span 0..15 seconds of age and must have identical colors
within each band. There are 32 snapshot scans and 3,072 points per bay. Simulation
retention permits both clocks; their source-age references remain independent.
This synthetic snapshot review is for the Earth preview: it uses the host's reported
home latitude/longitude and Concord's WGS84 datum, not live-frame placement.

The precision review exercises four age cases: a 953.674 ns color window at a
2^30-second origin, exact cyclic colors near `u64::MAX`, a subnormal window with
zero-age and one-nanosecond-future rows, and future half-second rounding ties.
Each bay has 3,072 points in 32 scans plus an explicit empty reference scan establishing
its source-clock time. The source offsets remain u64 nanoseconds, not f32 seconds.
These are deliberately synthetic numerical probes, not live LiDAR measurements.

These recipes build before launching, refresh only the owned preview when its binary
changes, and leave it open. Their JSON includes actual completed-view/frame receipts,
source counts and selected counts. Record near/grazing/overview views with
`oslo make capture-draw-data-views --args=OUTPUT_DIR` inside the same Nix environment.
Renderer diagnostics and exact generation activation are checked around each capture.

Clip boxes select sample centers in the declared source frame, preserving the stored
measurements and shared position/attribute buffers. This preview does not establish
LOD, picking, octree rendering, integrated million-point performance or release acceptance.
See [implementation status](DRAW_DATA_IMPLEMENTATION.md) for current proof and gaps.

## Earlier legacy Spatial Data Lab

This is a user-requested **interim visualization using the existing drawing renderer**.
It is not the new compact data renderer, retained streaming backend, picking system,
or scale-acceptance result. The scene itself carries that distinction.

## Open it

From `.work/draw-vfx`:

```sh
nix develop --offline --command oslo make preview-draw --args='data-lab overview'
```

Use `data-lab cloud` or `data-lab octree` for a close view. The recipe rebuilds the
simulator and gallery before launching. The launcher compares the running simulator's
executable hash with the build and restarts the dedicated preview if it changed.
Gallery-only changes republish the scene without a simulator restart. The launcher
leaves the window open and prints the actual instance name.

Refresh after each tested implementation batch, not only at the end. Use `data-lab keep`
to preserve live camera navigation when the simulator does not need a restart; use
an explicit view after a restart. Closing the window is not a request to stop future
previews. Unrelated simulator instances are not restarted.

## What is shown

- Three separated views of the same **5,852 synthetic samples**: surface-class RGB,
  intensity with Viridis, and actual sample height with Turbo. Positions and source
  order are identical apart from each bay's display translation.
- Two views of a validated **113-leaf mixed-depth octree**: occupancy state and
  hierarchy depth. Cells are 1.6, 0.8 and 0.4 metres, with true nonoverlapping keys.
  These are not randomly sized overlapping boxes; the legacy box renderer is only
  the interim visualization of the actual CPU octree fixture.
- Unknown is distinct from free, uncertain and occupied. Grey indicates unknown,
  green free, amber uncertain and coral occupied. Depth uses a separate palette.
- Consistent labels, structural frames and frosted cell surfaces use the existing
  approved design family. The user's first live review asked for more energized detail.
- The revised view adds 0.5-m cloud-frame rulers, 0.4-m depth rulers, partial moving
  selection arcs, low continuous glass rims and energy accents on two octree levels.
  These are annotations: they do not recolor, move or fabricate measurements.
- Glow remains localized and at most 0.45 in this scene. Meaningful measurement
  colors remain authoritative; no universal blue tint or global bloom increase.

The preview runs the scene clock so energy patterns can animate. Camera targets
include explicit bounds for overview, cloud and octree inspection. Labels were
repositioned after inspection found crowding in the initial capture.

## Verification and limits

```sh
nix develop --offline --command oslo make test-draw-data-preview
```

The gate includes all drawing-gallery tests, with three new checks for identical
cloud geometry/attributes, real nonoverlapping occupancy topology, localized
energy/style inventory, and valid focus targets. Native screenshots are inspected
alongside renderer logs; pixel existence alone does not establish success.

The complete typed-data backend plan remains active. This small preview does not
prove million-point performance, GPU budgets, transport throughput, live sensor
capture or typed dataset rendering. Other simulator/GPU workloads may be running;
the FPS label is not a controlled benchmark. User approval of the revised look
remains separate from these tests.

## Files

- `crates/gearbox-api/examples/draw_gallery/data_lab.rs` — fixtures, visualization and tests.
- `crates/gearbox-api/examples/draw_gallery.rs` — scene/manifest registration.
- `scripts/preview-draw.sh` — native window, clock and focus selection.
- `target/draw-vfx/live-preview/` — current preview screenshots/logs/instance identity.
- `target/draw-vfx/3d-data/preview/` — focused verification evidence.
