# Typed spatial data: a user guide

Gearbox draws robot data as typed datasets in world space: point clouds, occupancy
octrees, surfels, mesh blocks, sparse volumes and layered elevation maps. This guide
says how to send each kind, how to read it back, and what failures mean. The byte-level
contract is in [DRAW_DATA_PROTOCOL.md](DRAW_DATA_PROTOCOL.md); measured acceptance is in
[DRAW_DATA_ACCEPTANCE.md](DRAW_DATA_ACCEPTANCE.md).

The legacy `gearbox.draw.v1` helpers (`cloud()`, `voxels()`, `mesh()`, `grid()`) are
unchanged and still work. Typed datasets are a separate path. Use them when data is
large, updated in parts, needs exact identity, or has unknown or missing regions.

## The model

A **dataset** is `(namespace, id)`. It holds:
- a **descriptor**: its shape, frame, stream mode, lifetime and presentation;
- a sequence of immutable **generations**.

A writer opens a **lease**, begins a **transaction** (`Replace` or `Delta` against an
exact base revision), declares and sends **pages**, then commits. The commit is atomic:
viewers see the old generation or the new one, never a mix. A presentation update
replaces only the presentation; no data is sent again.

| Shape | One page holds | Unknown or missing |
|---|---|---|
| `Cloud` | columns of points in scans | invalid position bits |
| `Octree` | occupancy leaves | the unknown state, implicit space in a ROI |
| `Surfels` | cloud columns with normal and radius | a null normal: the disc faces the camera |
| `MeshBlocks` | one indexed triangle block | absent normals: shaded by the triangle's plane |
| `Volume` | one sparse block of `side³` samples | validity bits; a block never sent is unallocated |
| `Elevation` | one layer of one tile | missing elevation is a hole; a missing layer is a missing colour |

**Frames:** every shape but `Cloud` carries one `DataFrame`: `world`, a machine's `odom`,
or a link, with `live` or `snapshot` placement, an origin and a pose. Clouds carry a
frame per scan. A capture stamp (pose, datum, scene epoch) pins data where a sensor saw
it, not where the robot is now.

**Standing on the ground:** set `ground_radius_m` on a live `world` frame, and the
origin's z then counts from the highest terrain within that radius. A rigid map, block
or mesh placed on a slope never sinks into it.

## Sending data

**Rust:** `Client::draw_data_pipeline(config, deadline)?.writer(window, deadline)?` gives a
`DrawDataWriter`. `writer.upload(&DataUpload { descriptor, mode, base, scans, pages,
removed_pages, removed_scans }, deadline, cancel)` runs the whole transaction. For
presentation updates, find the current revision with a `STATUS` request on
`client.draw_data()?`. Encoders:
- `CloudPage::encode`, `encode_occupancy_page`, `encode_mesh_block`;
- `encode_volume_block` with `volume_page_id`;
- `encode_elevation_tile` with `elevation_page_id`.

**CLI** (`gearbox draw data …`):
- `list [NS]`, `status NS ID`, `delete NS ID`, `clear NS`;
- `slice NS ID --axis z --center X Y Z --size W H --cell M --range LO HI [--band LO HI]
  [--contour L]…` shows a volume slice;
- `layer NS ID NAME --range LO HI` colours an elevation map by one layer;
- `field NS ID --at X Y Z … [--gradient] [--nearest] [--weight]` reads a volume.

## Each representation

**Clouds:**
- A schema of typed columns (position, source id, colour, intensity, category,
  confidence, radius, …).
- Scans of pages, kept by `RetainedMap`, `Latest` or `Accumulate` (bounded by scans, age
  and bytes).
- LOD keeps at most the profile's visible points (1M by default). Selection runs off the
  render thread, so moving the camera never waits on it.
- Colour by constant, attribute, scalar palette, category, axis, range or age.
- Clip boxes can be turned and inverted.
- Picks answer the exact source sample.

**Occupancy octrees:**
- Leaves of keyed mixed-depth voxels with probability.
- Display chooses occupied, free, uncertain and unknown, with a depth limit, a ROI and a
  solid, surface or wire style.
- A 1% keyed delta uploads only its pages.

**Surfels and mesh blocks:**
- Surfels are cloud columns with exactly one normal and one metric radius; they draw as
  oriented discs.
- Mesh blocks are indexed triangles, up to 65,536 vertices each, with optional normals,
  RGBA and confidence.
- `NormalArrows { every, length_m, rgba }` draws sparse arrows on supplied normals only.
- A dirty block uploads alone.

**Sparse volumes:**
- A `VolumeField` of kind `Tsdf { truncation_m }` (weights required), `Esdf { signed }` or
  `Scalar { units, range }`.
- Voxel size and an 8- or 16-sample block side; blocks are keyed by integer index.
- Show it through `Presentation.volume`: up to four slices, a band, contours, an unknown
  colour and a border.
- Colour by `value` or `weight` with any palette.
- Unknown and unallocated support draws as a two-tone checker, never as a value.
- Read values with `FIELD_QUERY`. Trilinear reads need eight valid corners; the gradient
  is a central difference one voxel apart. The answer reports `support`
  (`valid`/`unknown`/`unallocated`) and the spacing used.

**Layered elevation:**
- An `ElevationGrid` with cell size, tile side and named layers; layer 0 is elevation.
- Colour by any layer name. Switching layers re-uploads colours only.
- A layer-only delta keeps all geometry.
- Rolling windows: `RollingMap` shifts by whole cells, keeps the overlap, leaves new
  bands missing, and returns exactly the tiles and removals a fresh map would differ by.

## Visibility, budgets and failures

- **Visibility:** `wait_visible` (Rust) or `STATUS` with a view reports
  whether a generation was *drawn*, *culled*, *empty*, *pending*, *superseded* or *view
  gone*. Visible means a completed frame drew it, not merely that it was stored.
- **Picks:** answers are `data` with full identity (kind, namespace, id, incarnation,
  generation, and the sample, leaf, surfel, triangle, slice cell or terrain quad). Other
  answers are `no_data` and `stale` with a reason. Terrain and colliders occlude; they
  never answer.
- **Reply codes:**
  - `INVALID`: a contract was broken. The message names the rule, for example "unknown
    volume samples must hold canonical zeros" or "elevation tile key or layer differs
    from its page id".
  - `CONFLICT`: the base revision moved; read `STATUS` and retry against the new one.
  - `BUDGET`: a limit (`DataLimits`) would be exceeded. The previous generation is kept;
    send less or use the reference profile.
  - `EXPIRED`: the writer lease timed out after 10 s idle; open a new writer.
  - `STALE_EPOCH`: the host restarted; negotiate again.
  - `SUPERSEDED`: a field query named a generation that is no longer current.
- **Profiles:** `reference` (4 GiB CPU, 2 GiB GPU, 1M visible points) and `low-memory`
  (`GEARBOX_DRAW_PROFILE=low-memory`: 128 MiB, 32 MiB, 100k points). The low-memory
  profile refuses oversized uploads and keeps the old generation; it never runs out of
  memory. `GEARBOX_DRAW_GPU_BYTES` replaces the selected profile's GPU budget.
- **Sensors:** no dataset of any kind appears in robot camera images.

## Moving from legacy drawings

- Keep `cloud()` for small, fire-and-forget overlays. Use a typed `Cloud` when you need
  more than about 100k points, partial updates, capture-time placement, picking by
  source id, or scientific palettes with a missing colour.
- `voxels()` draws uniform boxes with no occupancy meaning. A typed `Octree` carries
  free, unknown and probability, and updates by key.
- `grid()` is a heat map draped on the terrain. A typed `Elevation` map has real holes,
  shared layers and rolling windows.

## Labs

Each lab is a separate `preview-draw-*` recipe in the owned preview:
- `preview-draw-surfaces`
- `preview-draw-fields`
- `preview-draw-terrain`
- `preview-draw-octree`
- `preview-draw-robot-scan`
- the cloud scenes

`preview-draw-overview` shows all of them at once, spaced apart (`--args=robot` also
drives the LiDAR robot). `capture-draw-overview` then saves full-window and viewport
frames of every bay by day and night, and refuses if the running binary differs from
the build or the log holds renderer errors.

## Known limits

- Eye-Dome Lighting is not implemented.
- A delta over a provider-backed (paged) dataset briefly leases every base page. It
  fails with `BUDGET` rather than exceed the cap.
- `STATUS` reports drawn and source counts, residency and bytes, but not the LOD error
  or per-frame upload work. The `draw-probes` recorder writes those to `frames.jsonl`.
