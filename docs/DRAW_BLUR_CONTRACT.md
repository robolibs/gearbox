# Shared drawing blur

`blur` is a per-entity frost strength from 0 (clear) to 1 (full), independent
of RGBA, opacity, glow and motion. Rust uses `.with_blur(value)`, Python uses
`blur=value`, and CLI creation/update uses `--blur VALUE`. It is carried in
the existing metadata section, preserving the `gearbox.draw.v1` wire layout.
Live metadata updates retain geometry and unrelated appearance settings.

Omission preserves existing appearance: world drawings have no added backdrop
filter, while panels retain their approved frost. Explicit values affect the
background transmitted through each drawing, not its text, boundaries or data.
Text without a backing and completely opaque pixels have no transmitted scene
to soften. Scientific colors, categories and geometry are not filtered.

The renderer uses a bounded nine-tap screen-space filter for world drawings,
with a five-physical-pixel offset. The parameter controls strength, not radius.
The scene copy follows the opaque and transmissive passes; ordinary transparent
layers are composited normally, not recursively blurred. Drawing materials keep
their original render phase and depth behavior. Sensor cameras do not receive
the drawing scene-copy pass.
Panel frost blends clear and filtered scene captures. The shared strength is
bounded; stroke/surface cache keys quantize it to 8 bits rather than growing an
unbounded material cache. Existing unconfigured drawings keep their current path.

## Verification

The approved TRON trail silhouette, energy treatment and 0.45 m demo height
are unchanged. Evidence is under `target/draw-vfx/blur/`:

- `test-draw`: 338 passing focused tests; the two GPU tests are run separately.
- `test-draw-frost`: both GPU backdrop/filter and resize/release tests pass.
- `test-draw-parity`: 115 Rust/Python entities match, including blur metadata
  through the ten world-helper fixtures.
- `test-draw-mission`: six route/controller tests pass.
- `test-draw-live`: native Python 0/1/0 and CLI 1/0 blur updates preserve other
  fields, alongside the existing retained-lifecycle checks.
- `test-draw-sensors`: optimized and full sensor paths remain unaffected with
  blur enabled on surface, stroke, heat and ghost drawings.
- The native `blur-review` window compares clear, half and full frost. Its
  overview includes panel text; the near view shows the transmitted terrain.
- The headless catalogue captured 103 bays and 12 sections with `blur=0.6`.
  These viewport-only captures verify world geometry, not host labels or panels.
- A separate native-window run captured 26 annotation/panel/interaction/control
  bays at `blur=0.6`, plus all three panel finishes. All six panel families were
  inspected at full resolution. The existing native interaction regression also
  passed (point/pose picks, handle drag, step, cancellation and menu actions).

This is visual/functional verification, not a performance benchmark or user
approval. Panel clear captures add two full-resolution textures. The pre-existing
Bevy slab-allocator startup diagnostic remains; successful capture logs contain
no shader-validation, invalid-view-query or out-of-memory failures.

## Usage

```python
gb.draw.shapes("demo", "glass", "sphere", [(0, 0, 1)], blur=0.6)
gb.draw.table("demo", "values", (2, 0, 1), [("Speed", "0.9 m/s")], blur=0.6)
gb.draw.update("demo", "glass", blur=0)
```

```rust
let entity = DrawEntity::lines("demo", "trail", &points).with_blur(0.6);
let update = DrawEntity::update("demo", "trail").with_blur(0.0).updating(&["props"]);
```

```sh
gearbox draw update demo trail --blur 0.6
oslo make preview-draw --args='blur-review overview'
GEARBOX_DRAW_BLUR=0.6 oslo make preview-draw --args='exhibition overview'
```

Values outside 0..1, NaN and infinity are rejected. The catalogue environment
override is a test/preview convenience; every resulting entity still carries
the same public `blur` metadata. `blur-review` compares 0, 0.5 and 1 with equal
colors and geometry, including the approved wall, curved surfaces, regions,
real robot ghosts and a white-frost table.
