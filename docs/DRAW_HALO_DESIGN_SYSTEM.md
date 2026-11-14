# Halo-derived world drawing system

Research checkpoint: 2026-11-09. **Implementation subsequently resumed at the
user's explicit request.** This document defines the target; it is not evidence
that the target is fully implemented or accepted. See
[`DRAW_HALO_IMPLEMENTATION.md`](DRAW_HALO_IMPLEMENTATION.md) for current code,
native preview evidence and outstanding gates. The initial research pass changed
documentation only; later implementation is tracked separately.

## Direction that supersedes the earlier experiments

- **Halo Infinite is primary; Halo 5: Guardians is secondary. Death Stranding
  is excluded.** Earlier DS2 research and the blue fading-label implementation
  are historical experiments, not the current design specification.
- **This is a robot simulator, not a game HUD.** Drawings attach to robots,
  routes, measurements and world objects. Do not add a visor, ammunition,
  health, radar, menu navigation or screen-edge objective indicators.
- **Labels come first**, but redesign them together with the drawings they
  describe. A label-only replacement cannot make the system coherent.
- More detail means meaningful structure, hierarchy and state, not greater
  blur, bloom, glare or a thicker luminous border.
- Energized styles retain their moving packets, rising fronts and pulses.
  Optional glow is separate from readable animated structure; removing glare
  must not turn the whole system static.
- Panels and annotation frames use caller-tinted translucent energy glass,
  like the Tron-style world elements: not opaque cards and not fully clear
  outlines. Panels offer color, soft white + color and soft black + color
  finishes over the same live blurred scene. White uses dark contents;
  black uses light contents. Keep the film translucent, retain scene contrast
  and do not clamp its luminance into flat gray cards. Color remains in the
  glass hue, edges and data, not a universal blue theme.
- Preserve the drawing API and scientific meaning. Theme defaults must never
  override an explicit client color, a colormap, an axis convention or a value.
- Show intermediate native previews rather than completing the catalogue
  unseen. Record current previews and outstanding checks in the implementation
  checkpoint; a successful launch is not visual approval.

## Reference evidence and its limits

These are observations of original artists' imagery, not instructions to copy
whole game screens. The Gearbox rules below are our proposed adaptation.

### Primary: the user's campaign-menu reference

[Omer Younas — Official HALO INFINITE - Campaign Menu UI](https://www.behance.net/gallery/208543077/Official-HALO-INFINITE-Campaign-Menu-UI)
is the leading reference. The page itself returned 403, but four original
Behance CDN boards were downloaded and visually inspected:

| Board | Observed evidence | Gearbox application |
| --- | --- | --- |
| [Target dossiers](https://mir-s3-cdn-cf.behance.net/project_modules/fs/a391a6208543077.66f25f254bfaf.png) | Fine frame fragments, aligned identity/status rows, compact symbols and mesh detail | Shared label/frame anatomy; topology-aware inspection drawings |
| [Target states](https://mir-s3-cdn-cf.behance.net/project_modules/fs/757676208543077.66f39b3e0fb73.png) | Distinct known/unknown and active/inactive layouts; small weapon imagery reduces dot noise to avoid moire | State changes alter structure and contrast; decorative detail has a screen-space budget |
| [FOB and marine holograms](https://mir-s3-cdn-cf.behance.net/project_modules/fs/f524fa208543077.66f25e02b94bb.png) | Category/identity hierarchy, separate availability row, framed selection, visible shaded model surfaces | Distinguish identity from metadata; preserve robot shape rather than showing only a bright rim |
| [TacMap sequence](https://mir-s3-cdn-cf.behance.net/project_modules/fs/682098208543077.66f25e02b3743.png) | Staged introduction and removal of map information; labelled concept frame and sequence beats | Use a coherent transition vocabulary, not constant activity everywhere |

The large presentation-page headings are not world-label samples. The TacMap
board contains concept material as well as sequence images; it is not proof
that every displayed design shipped. The menu's dramatic focus transitions
and model renders are not a mandate for depth-of-field or full-screen UI here.

### Broader Infinite system

[Eric Dies' design-system breakdown](https://www.mrdies.com/halo-infinite-presentation)
provides shared grids/components, icon tiers, contextual motion and a distinction
between cyan front-end messaging and green backend narrative graphics. Actual
design, inventory, state and VISR boards were inspected. A visible type specimen
names Industry Medium; it does **not** identify every campaign label font.

[343's UX/UI interview](https://www.halowaypoint.com/news/inside-infinite-october-2021)
describes restraint, readable information and muted front-end presentation.
Together these sources reject the premise that all Halo UI is one blue glowing
material. We choose one human technical language for Gearbox and adapt its
components to different information densities.

The label and spatial reports below add primary TacMap, icon, equipment and VFX
evidence. Alien holograms, campaign cinematics and fan redesigns are not silently
treated as interchangeable with the chosen human technical system.

### Secondary: the user's Halo 5 waypoint reference

[Jeff Christy's Halo 5: Guardians HUD](https://jeffchristy.artstation.com/projects/KR5RG)
remains the requested waypoint reference. Its page text was accessible, but the
linked motion could not be verified. His own
[Halo 5 Behance project](https://www.behance.net/gallery/29909697/Halo-5)
provided actual tracking/scan concept images: local station markers, a selected
frame and distant objective labels. That page explicitly identifies early
previsualization. Use these as secondary spatial cues, not evidence of final
shipping waypoint behavior and not permission to import the surrounding HUD.

## One visual language, several information densities

**Baseline: human technical inspection.** Clean pale text, cool structural
accents, open space, precise linework and selective shaded surfaces. The
simulator's world and measured information remain the subject.

Coherence means the same typography, line hierarchy, corner construction,
icon grid and state behavior. It does not mean every point, label, heatmap and
robot must have the same decoration or become cyan.

### Shared primitives

1. **Information core:** readable text, true contour, scalar fill or geometric
   centerline. It must exist without emission.
2. **Structural frame:** sparse corner segments, short datum ticks, aligned
   separators and an occasional junction square. Never every motif on every
   object; spacing and line weights are shared.
3. **Identity symbol:** one consistent glyph grid and stroke family. Original
   robot-relevant symbols distinguish a destination, sensor, measurement,
   obstacle and machine. Selection is not encoded by changing the class icon.
4. **State treatment:** quiet, emphasized, selected, stale/unknown and alert
   presentations, used only when a real caller or interaction supplies that
   state. Geometry and text must reinforce semantic color.
5. **Surface detail:** topology, a meaningful datum grid, or sparse hatching,
   chosen for the kind of object. Do not overlay every pattern at once.
6. **Optional light and motion:** small emphasis around an already readable
   core. Event cues and active effects do not obscure units, boundaries or data.

### Tokens to centralize before polishing individual helpers

| Token group | Shared decisions | Non-negotiable limit |
| --- | --- | --- |
| Type | Identity, category, value/unit and secondary text roles; consistent baselines | Raw strings retain casing, Unicode and content; no guessed game font or silent font replacement in the host shell |
| Stroke | Core, secondary and selected weights; joint/cap treatment | Keep negative pixel widths and positive world widths meaningful |
| Spacing | Icon cell, row gap, separator inset, corner length, leader offset | Scale with text and projected size; avoid hardcoded per-helper offsets |
| Color | Neutral ink, structural accent, background support, semantic state | Explicit client colors, XYZ axes, classes and scalar maps take precedence |
| Material | Coverage, shaded body, contour, internal detail, emission | Opacity affects every layer; emission cannot be the only visibility channel |
| State | Selected emphasis, unavailable treatment, alert redundancy | No invented confidence, safety, completion or sensor activity |
| Density | Compact, normal and inspected detail; screen-space thresholds | Drop decoration before identity or measured data |
| Motion | Entry, change, active cycle and settled state | No universal perpetual flicker; reduced-motion and static cases remain informative |

Exact palette values, font selection, line widths and timings are still preview
decisions. They should not be presented as sampled Halo values. Use a properly
licensed available face with the reference's compact squared-sans character;
the isolated world-font mechanism can be retained without keeping Ubuntu Light
as the intended final visual match.

## Labels: the first mixed-scene implementation

### Three levels, not one enormous callout

| Level | Anatomy | Intended use |
| --- | --- | --- |
| Compact | Small class/anchor glyph and readable identity; tight contrast support | Routine waypoints, graph/frame names, repeated annotations |
| Normal | Optional small category, identity, short separator and supplied value/unit or status; restrained leader when displaced | Robot, destination, detection and measurement labels |
| Inspected | Shared frame with explicit title/status separation and actual metadata rows | Selected object or an existing information panel |

The same type, corner and icon primitives must be used for `TEXT`, `LABEL`,
callouts, badges, destination captions, detections, graph labels, TF labels and
`PANEL` titles. An existing panel keeps its supported anchoring mode; unifying
its skin does not authorize new screen-fixed HUD widgets.

**Remove the rejected full-width fading blue bands.** Do not replace them with
the earlier rounded black pills. Compact labels normally need only a tight
glyph-edge contrast treatment; denser content can use a bounded quiet backing
with sharp frame fragments. Avoid mandatory plates or a diamond on every string.

Use tracked uppercase only for appropriate generated short categories or
identifiers; body text stays readable, and arbitrary client text is untouched.
Metadata comes from supplied values. Do not parse free text into invented
status fields or add decorative serial numbers as if they were telemetry.

Placement is part of the design: constrain leaders and labels to their world
anchor, clip safely to the viewport, handle long names and avoid concealing
adjacent labels. A world-anchored annotation must not become a screen-edge
objective arrow when its source leaves view. Occlusion needs explicit tests;
projection alone is not proof of terrain/robot depth occlusion.

## Apply it across the entire drawing catalogue

The migration must cover all **13 wire kinds and 20 explicit styles plus AUTO**, not only the five new
world helpers. Family-level priorities are:

| Family | Detail worth adding | Detail to remove or avoid |
| --- | --- | --- |
| Paths, trails, rollouts, swaths | Legible center/edge hierarchy, station ticks, terminal caps and consistent directional glyphs | Indiscriminate bloom, busy texture at distance, decorations that change route width or cost meaning |
| Points, clouds, particles, marks | Consistent small glyph construction, selected emphasis and density-aware decoration | A halo per dense sample, fake terrain classifications |
| Arrows, axes, forces, frames | Shared stem/head construction, clear origin, units and sparse scale ticks | Recoloring axes or altering vector magnitude for appearance |
| Shapes, bounds, covariance | Actual outline, selected corner brackets, quiet interior and appropriate datum lines | Universal honeycomb, moving measured bounds, suggesting uncertainty is a shield |
| Polygon, grids, heat and coverage | Boundary and grid hierarchy, subtle selection, legible legends | Cyan-washing data, filling unknown holes, animated values without new data |
| Ghosts and arbitrary meshes | Visible shaded body, deliberate contours and useful mesh detail at close range | Rim-only blobs, dense dot noise, topology lines that imply nonexistent measurement features |
| Sensor cones and sweeps | Clear origin, calibrated extent, restrained beam/core and explicit acquisition timing | A decorative scan interpreted as fresh sensor data |
| Labels, panels and TF | Common typography, icon cells, short rules, row hierarchy and anchor language | Three incompatible painters, DS2 bands, round glass capsules, redundant reticles |
| Interaction | Same selected/hover/drag grammar as ordinary annotations, persistent readable feedback | A game reticle or decoration that intercepts input |

`terrain_marks`, `route_stations`, `destination`, `clearance_box` and
`field_volume` become compositions of those shared elements, not their own
art style. Their geometry, stable child identities and clearing semantics stay
intact. Existing unsupported cases must remain explicitly unsupported rather
than being hidden by new styling.

## Rendering and API contract

The source audit found independent style decisions in helper builders,
`mesh::Fx`, stroke/surface/heat/ghost shaders, world labels, panels and TF labels.
Several luminous styles currently lose their visible body when emission is
disabled. A single global glow multiplier cannot solve that architecture.

The proposed rendering order is:

**caller data/overrides -> semantic role and actual state -> shared tokens ->
kind-specific geometry -> coverage/core + structural detail + optional emission.**

The arrows describe resolution order, not a new public API or one universal
shader. Surface, stroke, point, heat and UI paths still need different rendering
techniques. They should consume the same visual decisions instead of duplicating
hardcoded color, opacity, font and intensity choices.

Acceptance semantics for the redesign:

- Glow disabled: core geometry and information remain visible. This is a
  required change for current light-only branches, not a claim they already work.
- Opacity zero: core, frame, fill, label, leader and emission are all invisible.
- Motion disabled: no decorative cycle; static shape and values remain useful.
- Explicit colors and style controls remain authoritative. AUTO resolves
  contextually; it is not permission to convert scientific layers into effects.
- Preserve `gearbox.draw.v1`, numeric kind/style identities, retained `(ns,id)`
  operations, native client chunking, simulation-time lifetimes, transforms,
  draping, depth flags and interaction events.
- Keep correct holes, colormaps, bounds, covariance axes, vector lengths and
  supplied mesh geometry. Decorative subdivision is not additional sensor data.
- Do not wire-break Rust/Python/CLI to obtain visual consistency. New optional
  properties, if necessary, need documented defaults and cross-client parity.

## Keep correctness fixes; replace the rejected appearance

Keep the verified ghost completeness work for optional normals and USD material
subsets; anchor/world-vertical draping behavior; stable helper child cleanup;
native chunking; and isolated font-family support where useful. Preserve their
regression tests independently of visual design.

Replace the fading label bands, independent panel/TF chrome, uncontrolled
helper-specific glow defaults and effects that substitute emission for shape.
Do not discard good geometry/API fixes merely because the art direction was
rejected. Conversely, first-pass tests and earlier praise do not establish
acceptance of either the rejected extension or this proposed Halo system.

## Implementation order after resumption

1. **Shared foundation and mixed proof:** central tokens, label/frame/icon
   primitives, and one scene containing a robot label, selected destination,
   route, clearance bound, field grid and robot ghost. Keep a compact and an
   inspected label visible together. Preview with glow both enabled and off.
2. **All label consumers:** `TEXT`, `LABEL`, helper captions, panels, TF,
   detections and interactions. Review long text, Unicode, bright ground and
   overlapping anchors before adding more surface decoration.
3. **Linear and sampled data:** all strokes, arrows, points, paths, swaths,
   rollouts and terrain marks with shared line hierarchy and density policy.
4. **Volumes and data surfaces:** all shapes, polygons, grids, heat, meshes,
   ghosts and scans, preserving their semantic differences.
5. **Complete catalogue and contracts:** run the repository-native Make targets,
   verify Rust/Python/CLI parity, all looks/kinds and actual native interaction.

Each stage gets an actual native preview and user feedback. Do not implement
the entire design before showing the first mixed scene. Use the existing scoped
preview workflow; never clear another client's namespaces or restart an
unrelated viewer. Exact Make recipes and existing evidence are recorded in
[`DRAW_VFX_VERIFICATION.md`](DRAW_VFX_VERIFICATION.md).

## Visual acceptance, not just compilation

- With emission and motion off, the same objects are identifiable; boundaries,
  units, labels and selection remain understandable.
- In grayscale, role/state distinctions remain readable through hierarchy,
  geometry and text rather than color alone. Scientific colormaps retain their
  original values in normal rendering.
- Near, middle and far views remove fine decorative dots/ticks before they
  shimmer or overwhelm the actual data. Test camera movement, not just stills.
- Bright terrain, shadow and night preserve label contrast without huge plates
  or luminous haze. Test real occlusion, off-screen anchors and long labels.
- One mixed scene looks like a single system; an isolated attractive waypoint
  does not count as catalogue acceptance.
- Every kind/style, all nine shape primitives, fixed and world panel modes,
  scalar maps, holes and explicit controls are included in the review matrix.
- Dense points and rollouts need representative measured performance. Earlier
  capped-FPS captures do not establish a performance budget or improvement.
- Picking, pose placement, drag/handle feedback and Step events are exercised
  in the native viewer; screenshots cannot prove interaction correctness.

## Reference material

Reference downloads were inspection material only; they are not shipped game
assets. No game fonts, logos, textures or ripped icon assets were
added to the project.

**Still pending:** implementation resumption, first coherent mixed-scene
preview, user acceptance, the full API/render/interaction matrix and final
performance measurements. The known baseline startup allocation diagnostic
remains separate and unfixed. No new code/test/build result is claimed here.
