# Drawing color contract

The shared visual language does not impose a palette. Shape, line treatment,
type, framing and energy behavior stay consistent when the caller changes color.
The blue review scene is a comparison fixture, not a renderer restriction.

## Tintable elements

- Lines, points, arrows, shapes, polygons, meshes and robot ghosts use the
  supplied entity RGBA for their body, core, structural details and emission.
- Nonzero per-item colors override the entity fallback where the kind supports
  item data. Ghost poses have individual tints; their source materials are not
  copied. Missing/zero entries inherit the entity tint.
- Raw text retains its supplied text color. `label_accent` independently sets
  its frame, symbol and leader. Generated measurement identities can remain
  neutral while carrying the caller tint in their accent.
- Panels use entity color for their translucent glass body, frame and chart
  accent. Bars can supply individual RGBA colors; zero entries inherit the
  panel accent. Dark tinted glass separates the scene from light neutral
  text and chart cores. Caller RGB remains in the glass hue, frame, signal
  edges and bar bodies; scientific ramps are unchanged.
- `color2` remains the explicit secondary outline/background where supported.
  On shapes and polygons it controls the outline, not the annotation backing. Attached
  labels are transparent unless selected or explicitly given `label_background`
  (a named/hex color or packed RGBA integer in props). Raw TEXT still supports
  `color2` for its background, as do existing ruler/line annotations;
  `label_background` takes precedence when supplied.
- Glow changes light intensity, not the chosen hue. Surface lighting and
  tonemapping still affect perceived brightness and saturation.

Robot ghosts use the source machine's loaded, visible geometry. Late model
parts appear automatically; removing the source hides its copies, and reloading
the same machine restores them without a new drawing request. Ordinary source
motion does not move the requested ghost poses. A changed source assembly
refreshes the copied geometry at those poses, retaining their API colors.

Imported OBJ meshes retain authored corner normals and smoothing groups. Explicit
`vn` normals take precedence; `s off`/`s 0` and the flat default keep face creases,
while a shared nonzero group smooths adjacent faces. STL normals are generated
from its separate triangles. Mesh body colors still come from the drawing API,
not the asset's material library. Imported assets remain cached by path.

## Structural emphasis

HOLO, GHOST, SHIELD and FIELD boxes and rectangles share short heavier corner
arms. Each arm spans 18% of its edge. Wire and combined-fill outlines partition
each edge into a bold arm, a fine middle and a bold arm, rather than stacking a
second outline. Solid energy surfaces retain corner-only stroke accents.
Every cube vertex has three bold arms, one along each meeting edge; rectangle
vertices have two. The connecting middle of each edge remains thin.

Default fine edges are one pixel wide; accents are 1.75 pixels wide. Explicit
widths and per-item radii control the fine rule, with the same 1.75x ratio at
corners, retaining their metre/pixel units. Corners use the outline tint,
including `color2` for combined fill, and existing glow controls. PLAIN geometry
is unchanged. Batches above the existing 64-instance detail limit keep their
ordinary outlines without per-corner detail.

Clearance brackets and selected-waypoint accents use the same 1.75-pixel default.
Image-plane detection boxes and wire-only 3-D detections opt into the shared
partitioned frame with `props.corner_frame="true"`. They remain PLAIN wire
geometry without added glow or surfaces. Setting this property to `"false"`
retains an ordinary uniform outline. Caller widths and Python common options
override helper defaults; semantic class/score colors and selection colors remain.

The same opt-in is available for PLAIN shapes, polygon boundaries and closed
PLAIN line loops. Lines keep their original vertices, colors and radius units, with up to
64 loops and 64 authored corners per loop. Terrain subdivision preserves the
authored corner positions rather than adding accents at every terrain sample.
Mixed metre/pixel radii within one loop retain their original uniform treatment
instead of interpolating between incompatible units. Explicitly open or
degenerate line loops are not decorated.

Curved HOLO/GHOST/SHIELD shapes use short stronger arcs around four stations
on each structural ring, not a corner at every tessellation vertex. Cylinders,
cones and capsules carry the same hierarchy into their rims and supports.
SWEEP frustums and sectors use related border/end accents while preserving
sensor extents. Plain geometry keeps its ordinary treatment unless opted in.
When strokes provide the curved outline, surface shaders omit duplicate
meridians while retaining the glass rim, sectional detail and moving energy.

Styled polygon and projected DECAL boundaries include both outer and hole
rings. Their accents follow authored vertices through terrain subdivision.
Annular sectors keep their empty interior, including full circles without a
radial bridge. Sector shape builders default the inner radius to zero;
explicit instance or entity inner extents remain supported.

Imported energy meshes and real robot ghosts share feature-edge chains.
Smooth tessellation vertices do not become corners: true bends and junctions
receive short emphasis, and smooth closed rings receive four spaced arc
stations. At most 64 stations are accented per source mesh. Feature extraction
selects at most 16,384 genuine edges; a truncated graph stays uniform instead
of creating false endpoints. Source positions, triangulation, authored normals,
data colors and dense samples stay intact. Ghost accents use filtered screen
widths and fade fine detail when it becomes subpixel.

Glass panels and selected label frames use the same reduced accent weight on
their actual chamfered contours. Their film, blur, contents and caller colors
are independent of this line hierarchy.

## Annotation placement

World text, attached labels, TF names and panels share reserved viewport bounds.
Placement uses the current simulator shell and play/clear control bounds after
they are painted in the same UI pass, including responsive window resizing.
Explicit centered text keeps its supplied anchor. Selected labels are placed
before panels; ordinary attached labels use the remaining space. Leader lines
attach to a facing frame edge rather than crossing that frame's contents.
Their strokes and anchor marks are hidden behind neighboring annotation bounds.
Offscreen world-panel anchors do not become screen-edge indicators. Viewport
legends keep their bottom-right preference while avoiding reserved annotations.

Placement does not change caller text size, color, chart data or world anchors.
When the viewport cannot fit everything, placement minimizes overlap rather
than silently deleting annotations or shrinking their contents.

## Panel glass finishes

`props.panel_finish` selects the glass film without recoloring chart data:

| Value | Glass film | Contents |
| --- | --- | --- |
| `color` | Stronger caller tint | `panel_theme` selects dark or light ink |
| `white` | Soft white mixed with caller color | Dark ink |
| `black` | Soft black mixed with caller color | Light ink |

All three share real scene blur, frame geometry and colored edges. White
uses a stronger milky film at 50–58% coverage; color and black retain their
thin 18–26% film. All still transmit the blurred scene rather than becoming
opaque cards. Scientific ramps and per-series colors are unchanged.

Without a recognized finish, `panel_theme=light` selects white + color;
otherwise black + color is the default. Explicit white/black finishes choose
their own opposing ink, even when `panel_theme` requests the other polarity.
For `color`, `panel_theme=light` means dark ink; dark/unknown means light ink.

```rust
let panel = panel.with_prop("panel_finish", "white");
```

```python
draw.gauge("demo", "range", [0, 0, 1], 0.65, lo=0, hi=1,
           color="#cf72a8", props={"panel_finish": "white"})
```

```sh
oslo make cli --args="--instance NAME draw panel demo range --kind gauge --values 0.65 --min 0 --max 1 --panel-finish white --color '#cf72a8'"
```

Native panels sample a live GPU-blurred scene through a caller-colored film.
Only the unavailable-GPU fallback has a dense backing tested for 4.5:1
contrast over black and white backgrounds. Real translucent glass depends
on the scene behind it; choose the finish/ink polarity accordingly. Caller
alpha and distance fading also reduce opacity.

## Data-driven colors

- XYZ axes retain their coordinate convention.
- Scalar heatmaps, clouds and meshes with mapped values use the requested
  colormap and range. Choose a different colormap through the API; a generic
  tint must not silently replace the data mapping.
- Classification and state helpers retain their semantic default palettes.
  Explicit tint options apply to the documented tintable parts, not to
  unrelated status/class encodings.

## Examples

Rust entity construction:

```rust
use gearbox_api::{DrawEntity, draw_style};
use gearbox_api::wire::draw::{color, rgba};

let path = DrawEntity::lines("demo", "route", &[[0.0, 0.0, 0.1], [4.0, 1.0, 0.1]])
    .with_style(draw_style::RIBBON_FLOW)
    .with_color(rgba(255, 179, 71, 255));
let hidden = path.clone().with_color(color::CLEAR);
```

Python drawing client:

```python
draw.lines("demo", "route", [[0, 0, 0.1], [4, 1, 0.1]],
           style="ribbon_flow", color="#ffb347")
```

Change a retained entity through the CLI:

```sh
oslo make cli --args="--instance NAME draw update demo route --color '#ffb347'"
```

Packed wire `0` means automatic/default, not transparent black. Rust callers
can use `color::CLEAR` for fully transparent color. Explicit nonzero RGBA with
alpha zero must remain invisible through derived helper fades and emission.

## Selective live updates

Python and CLI updates preserve unspecified fields, including paired colors,
individual energy controls and colormap bounds:

```python
draw.update("demo", "route", glow=0.3)  # Retains pulse, flow, spacing and beam height.
draw.update("demo", "range", panel_finish="white")
draw.update("demo", "heat", vmin=0)
```

```sh
oslo make cli --args="--instance NAME draw update demo route --glow 0.3"
oslo make cli --args="--instance NAME draw update demo range --panel-finish white"
oslo make cli --args="--instance NAME draw update demo heat --vmin 0"
```

Rust/wire clients select scalar fields explicitly:

```rust
let update = DrawEntity::update("demo", "route").with_glow(0.3).updating(&["glow"]);
let finish = DrawEntity::update("demo", "range")
    .with_prop("panel_finish", "white").updating(&["props"]);
```

`tint` selects primary RGBA, `color2` secondary RGBA, `palette` the colormap
ID, and `vmin`/`vmax` individual bounds. Effect selectors use their wire field
names: `glow`, `pulse_hz`, `flow_speed`, `dash`, `beam_height`, `fade`, `urgency`, `opacity`.
Legacy `color`, `effects` and `colormap` masks still replace their whole groups.
An absent or empty wire mask still replaces all headers; `props` is metadata
only. Scalar masks require the updated host as well as the client.

Non-finite pose, scale, lifetime and per-item radius values are rejected without
changing the retained entity. Signed finite controls and default sentinels remain
valid; unknown NaN/Inf samples inside F32 grid-cell data remain supported.

All nine shapes honor explicit wire mode without filled surfaces or beams.
Both mode keeps the energized surface with its caller-selected secondary outline.
Default solid styles retain their technical detail and energy animation.

## Native review

```sh
oslo make preview-draw --args='review-colors overview'
oslo make preview-draw --args='review-colors-light overview'
oslo make preview-draw --args='review-finishes overview'
oslo make preview-draw --args='energy-colors overview 6'
```

These fixtures set colors through ordinary API fields. The scalar legend keeps
its chosen scientific ramp rather than becoming the panel's accent color.
