# Spatial data acceptance contract

Profile revision: 1, frozen 2026-11-10. Implementation scope and ordering:
`plans/PLAN.md`. Current implementation status: `docs/DRAW_DATA_IMPLEMENTATION.md`.
The contract below was frozen first. The measured results for Releases A and B are in
[Results](#results--releases-a-and-b-2026-11-13) at the end.

## Profiles and environment

Machine-readable limits and targets live in `tests/fixtures/draw_data/profiles.json`.
The reference device observed at baseline was an RTX 4080 with 16,376 MiB driver-
reported memory, NVIDIA 595.112.07, Linux/Vulkan. About 5,922 MiB was already in use.
An unrelated simulator was running; do not stop it without instruction and do not
claim controlled GPU timing while it contaminates the measurement.

Reference timing requires an actual 1920x1080 render target, four MSAA samples,
10 seconds warmup, 60 seconds measured, and no presentation-rate cap. Record the
actual target settings in the manifest. If those settings cannot be applied, fail
profile setup instead of silently measuring another configuration. Native-window
captures remain necessary for labels, panels, controls, and interaction, independently
of uncapped/offscreen timing. A compositor can override requested window dimensions.

Low-memory uses 1280x720, one sample, 128 MiB total new-data CPU ownership, 32 MiB
GPU ownership, and 100k visible points. It tests bounded degradation/rejection, not
the reference frame-time target. Limits include old/new generation overlap, staged
pages, temporary indexes, retained/pending buffers, readbacks, and deferred destruction.
Sub-budgets are included in the totals, not extra allocations.

## Deterministic inputs

`crates/gearbox-api/tests/support/draw_data_fixtures.rs` supplies seed `0x5eed2026`:

- Plane, step, curved wall, noisy organized-order samples, and semantic samples.
- Stable source IDs, RGBA, intensity, class, and validity coexist in every fixture.
- Invalid noisy-scan samples use a false validity flag and canonical finite zero
  coordinates, not NaNs uploaded to shaders.
- Samples use binary-exact lattice spacing; layers repeat at different heights for
  million-point scale. The generator is capped at five million samples.
- Occupancy uses two 16 m roots, including a negative tile, depth 0/1/2 leaves,
  occupied/free/uncertain/unknown values, and no ancestor/descendant overlaps.
- Manifests include units, frame, counts, seed, FNV-1a content fingerprint, and exact
  first/middle/last source-ID query values. The fingerprint is a reproducibility
  checksum, not a cryptographic integrity or authentication claim.

Commands, from the drawing worktree:

```sh
nix develop --offline --command oslo make test-draw-data-fixtures
nix develop --offline --command oslo make draw-data-fixtures --args='target/draw-vfx/3d-data/fixtures 1024'
```

The generator writes small manifests, not huge source arrays. Future native runners
must consume these same generators or prove exact parity, not replace them with a
different easier scene. Golden 1,024-sample manifests live under
`tests/fixtures/draw_data/reference_1024/` once generated and checked.

## Measurement output

Every benchmark/native run uses a unique directory with the following files:

- `manifest.json`: schema revision, run ID, source and binary hashes, instance/PID,
  device/driver/backend, measured target size/MSAA, cap/vsync state, active workloads,
  profile name/revision, seed, counts, exact commands, and started/finished times.
- `frames.jsonl`: one record per measured frame with elapsed time, CPU frame ms,
  optional GPU ms (`null` plus reason if unsupported), source/retained/resident/visible
  counts, geometry/color/uniform upload bytes, owned CPU/GPU/staging/pending/index
  bytes, retired GPU bytes, process RSS, and culling/eviction/miss counters.
- `updates.jsonl`: object key, transaction, epoch, stored generation, visible generation,
  arrival/retained/visible monotonic times, byte counts, and explicit drop/reject reason.
- `results.json`: one named case per gate, pass/fail/unsupported state, measured
  percentiles, thresholds, and evidence paths. Unsupported required checks fail release
  acceptance; they are not converted into passes.
- `viewer.log`, `runner.log`, current screenshots, and `ACCEPTANCE.md` summarizing
  proved results, failed/open gates, and exact reproduction commands.

CPU/GPU ownership counters must reflect actual allocated capacity, shared allocation
identity, and delayed release. RSS is supplemental because allocator caches and other
simulator systems contribute. Camera sensor paths must never include drawing effects.
Record frame distributions, not only average FPS or an overlay screenshot.

## Gate ownership

| Gate | Phase | Required success |
|---|---|---|
| Baseline | 0 | Existing focused/parity/GPU/live/camera checks; native cloud/voxel/grid/panel/trail references |
| Expiry | 1 | Old scheduler fails regression; fixed scheduler passes plugin and live lifecycle checks |
| Data schema | 2 | Bounded validation, compatible legacy bytes, cross-client fixtures, capability behavior |
| Streaming | 3 | Atomic generations, quotas, cancellation, restart/clear barriers, no silent delta loss |
| Cloud | 4–5 | Compact records, correct colors/depth, LOD/frontier/culling and bounded residency |
| Occupancy | 6 | Correct keys/topology/states, atomic split/merge, compact mixed-depth display |
| Adapters | 7 | Capture-time rigid pose, valid sample identity, malformed input rejection |
| Inspection | 8 | Exact ID/generation picking and clip/depth parity in native interactions |
| Release A scale | 9 | 100k/1M/paged 5M cloud and 100k/500k occupancy matrix, budget and latency/frame targets |
| Surfaces | 10 | Supplied normal/radius/mesh-block semantics, seams, bounded dirty updates |
| Fields | 11 | Metric TSDF/ESDF, validity-aware slices and block updates, analytical probes |
| Layers | 12 | Shared elevation geometry, holes, atomic layers, rolling-window equivalence |
| Final catalogue | 13 | A+B regression matrix, every representation visible and coherent; optional EDL separately gated |

Native evidence must match the run's loaded binary and current logs. OOM, shader
validation failures, invalid view queries, or stale screenshots fail the run. The
pre-existing startup slab-allocator diagnostic is recorded separately and is not
blanket permission to ignore new lifetime failures. Visual approval by the user is a
separate state; successful automation or silence does not imply approval.

## Results — Releases A and B, 2026-11-13

Every recipe was run as `nix develop --offline --command oslo make <recipe>` from the
worktree. The whole list was rerun on 2026-11-13 after the user-directed removals
(file readers, format converters, Python extension and client, peerbus fork) and the
per-request data requester that replaced the fork. Logs are in
`target/draw-vfx/removed/gates/`, one per recipe, with exit codes in `summary.txt`.

### Gates

| Recipe | Matrix rows it covers | Result |
|---|---|---|
| `test-draw` | legacy drawing catalogue, wire and geometry, plugin and host drawing tests | pass; 552 Rust, 49 Python |
| `test-draw-expiry` | Lifetime: upload expiry scheduling | pass; 8 Rust |
| `test-draw-data` | Wire/schema, Transactions, Modes: store, Rust and CLI contracts; native host probe | pass; 136 Rust |
| `test-draw-data-fixtures` | Wire/schema: deterministic fixtures and golden manifests | pass; 7 Rust |
| `test-draw-stream` | Transactions, Modes: generations, deltas, accumulation, quotas, cancellation | pass; 113 Rust |
| `test-draw-parity` | legacy Rust/Python drawing parity | pass; 115 Rust/Python entities match |
| `test-draw-frost` | Style: shared blur on the GPU | pass; 2 Rust |
| `test-draw-live` | Transactions, Lifetime: Rust probe, raw CLI, 100k/1M retained uploads and legacy lifecycle against a rendering viewer | pass; Rust probe (27 checks), capacity, raw CLI and legacy lifecycle |
| `test-draw-cloud` | Cloud: compact records, colours, LOD, residency, clipping | pass; 241 Rust |
| `test-draw-octree` | Occupancy: keys, topology, states, aggregates, depth split, hidden faces | pass; 19 Rust |
| `test-draw-adapters` | Sensors: capture poses, LiDAR assembly | pass; 22 Rust |
| `test-draw-surfaces` | Surfaces: surfels, mesh blocks, normals, dirty-block reuse | pass; 13 Rust |
| `test-draw-fields` | Volumes, Elevation: analytical probes, slices, field queries, layers, rolling windows (17 native checks) | pass; 26 Rust, 17 native |
| `test-draw-mission` | legacy mission drawings | pass; 6 Python |
| `test-draw-grid` | legacy GRID kept intact beside typed elevation | pass; native image oracle |
| `test-draw-strokes` | Style: legacy strokes | pass; native image oracle |
| `test-draw-drape` | Style: legacy drape | pass; native image oracle |
| `test-draw-normals` | Style: legacy normals | pass; native image oracle |
| `test-draw-ghosts` | Style: legacy ghosts | pass; native image oracle |
| `test-draw-data-native` | Picking: every kind, clip and depth parity, stale camera, namespaces | pass; 7 Rust, 36 native |
| `test-draw-sensors` | Sensors: every kind absent in optimized and full camera paths | pass; 4 camera checks |

### Scale (`bench-draw-data`)

Rerun 2026-11-13 on the tree after the removals, with no competing instance.

**Reference** (`target/draw-vfx/3d-data/scale-removal/ACCEPTANCE.md`). Device: NVIDIA GeForce RTX 4080, 595.112.07, 16376 MiB. Size 1920x1080, MSAA 4, uncapped headless, 10 s warmup, 60 s measured. Competing instances at start: []; GPU at start: 4 %, 1256 MiB. CPU frame time is wall time between main-world frames; GPU timestamp queries are not enabled.

| Scenario | CPU frame ms p50 / p95 / p99 | Frame-time gate |
|---|---|---|
| cloud-100k-live | 10.9 / 12.2 / 13.6 | pass |
| cloud-1m | 11.6 / 14.2 / 15.2 | pass |
| cloud-5m-paged | 12.0 / 22.8 / 23.3 | pass |
| mixed: window 1 | 12.3 / 16.9 / 18.8 | pass |
| mixed: window 2 | 12.9 / 17.6 / 19.1 | pass |
| octree-100k | 12.0 / 13.4 / 14.5 | pass |

44 pass, 2 report-only, 0 fail. Result: all gates passed.
- 100k live at 10 Hz, stored to visible: p50 227 ms, p95 229 ms against 250 ms. 50 of
  209 updates were not seen visible within three periods (report only; earlier runs
  gave 0 to 182).
- 1M retained to visible: 369 ms.
- 5M paged: at most 1M selected, GPU 265 MB, residency 76 of 77 pages, 0 misses.
- 20 load/clear cycles return to idle.

The earlier recorded miss (p95 265 ms while the user's `lab` shared the GPU) no longer
applies: this clean run passes the gate.

**Low-memory** (`target/draw-vfx/3d-data/low-memory-removal/ACCEPTANCE.md`). Device: NVIDIA GeForce RTX 4080, 595.112.07, 16376 MiB. Size 1280x720, MSAA 1, uncapped headless, 10 s warmup, 60 s measured. Competing instances at start: []. CPU frame time is wall time between main-world frames; GPU timestamp queries are not enabled.

12 pass, 1 report-only, 0 fail.

- Budgets held: 100k selected, 89 MB CPU and 30 MB GPU against 128 MB and 32 MB.
- An oversize upload is refused, and the previous generation is kept.
- 20 load/clear cycles return to idle.
- Frame time is reported only; low-memory is not a timing profile.

### Native evidence

- **Overview:** `capture-draw-overview` → `target/draw-vfx/3d-data/overview/`. It gives
  19 full-window and 19 viewport frames by day and night. The binary hash matches the
  running preview and the log holds 0 renderer errors.
- **Style:** `capture-draw` in window mode → `target/draw-vfx/3d-data/final-style/`.
  It covers blur 0, 0.5 and 1.0 (`blur-review`), the three panel finishes
  (`review-finishes`: colour, white, black), and panels, dense, maps and spatial. The
  window and viewport frames were inspected, with 0 error lines.
- **Picks:** `test-draw-data-native` → `target/draw-vfx/3d-data/data-native/`.
- **Fields and terrain:** `test-draw-fields` → `target/draw-vfx/3d-data/fields/`.
- **Camera exclusion:** `test-draw-sensors` → `target/draw-vfx/3d-data/sensors/`. Every
  kind is checked in the optimized and full camera paths.

### Not claimed

- **Eye-Dome Lighting:** deferred, not implemented.
- **Paged deltas:** a delta over paged storage briefly leases every base page and fails
  with `BUDGET` rather than exceed the cap.
- **STATUS telemetry:** LOD error and upload work are not in `STATUS`; the `draw-probes`
  recorder writes them per frame.
- **GPU timing:** unsupported on this path and labelled as such; CPU frame times are
  reported.
- **User visual approval:** separate and not yet given.
