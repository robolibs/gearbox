# Halo drawing implementation checkpoint

## Goal

Implement the whole Halo-derived drawing system in `.work/draw-vfx`, not just
a label sample. The user explicitly resumed implementation on 2026-11-09.
The broader goal was paused and then explicitly resumed by the user on
2026-11-10 after approving the stronger white finish. Implementation and native
verification continue beyond the first catalogue pass. The user explicitly
rejected stopping there. Whole-system visual approval and controlled performance
acceptance remain open; earlier captures are checkpoint evidence, not completion.

## Active continuation — current acceptance

- User activated a new full recheck goal after the structural correction:
  continue checking every drawing and API family. The former pause no longer
  applies. Current native coverage is not yet a final full-system acceptance.
- Targeted user refinement: emphasis is now shape-appropriate rather than
  cube-only. Shared gain is 1.75 instead of 2.5. Curved shape rings have sparse
  stronger arcs; camera/sector extents, polygon holes, small imported meshes
  and actual panel contours share fine middles and stronger short sections.
  Native close-up inspection exposed duplicate surface meridians masking the
  stroke hierarchy; those are suppressed when strokes supply the outline,
  while glass, rim, sectional detail and moving energy remain. Caller colors,
  width units, dense-data limits and plain opt-outs are retained.
  All 313 focused tests, two GPU frost tests and 10 world/24 composite parity
  cases pass. `structure-language-final` records viewer `8ead7521...` and
  gallery `8270957a...`, six inspected native windows and nine passing native
  interactions. The initial grazing camera aligned with the capsule; the
  corrected diagonal grazing view is recorded separately. Initial portrait
  QA exposed cropped review scenes; bounded review cameras now fit the actual
  aspect ratio and have a regression test. The new preview has no cubes.
  `structure-language-before-wide` preserves the old-viewer comparison on the
  same new gallery, not an old client snapshot. The 103-bay catalogue below
  predates this refinement; it was not replayed here. The broader goal remains
  paused; this is the requested local correction, not a resumed full audit.
  Engram tools remain unavailable; the scoped SESSION_SUMMARY is the fallback.
- Full-catalogue continuation: `unified-final-catalogue` now captures all 103
  bays and 12 sections on viewer `2d265947...`, gallery `6216d0e3...`. All seven
  contact sheets and all 12 section windows were inspected, plus individual
  full-resolution B013, B038 and all six panel bays B079–B084. All nine native
  interactions pass after B103; requested and live hashes stayed stable. The
  isolated instance `draw-vfx-2195974`, PID 2195988, stopped normally. Section
  overviews retain fixture distance fading; they are not proof that every
  small caption/panel is readable at once. Individual bays provide detail views.
  Known slab startup diagnostics remain; no fatal renderer error occurred.
  At that checkpoint, preview `draw-world-preview-2221353`, PID 2221369,
  was inspected on `corner-review overview` with a matching executable; it
  has since closed. The prior preview 2152189 stopped normally.
  Shared language covers all families without replacing scientific colors or
  simplifying away data. Controlled dense-scene performance and whole-system
  user approval remain open; this replay does not claim either.
- Shell avoidance continuation: a failing-before test renders the real Mara
  rail and reproduces a table under the play/clear buttons. `draw_annotations.rs`
  now queues owned projected label/panel snapshots and lays them out at the end
  of the same egui pass, after Mara's enforced shell renderer. Native shell
  exclusion rectangles come from Mara's published host geometry; play/clear
  use their actual registered widget bounds. No fixed gutter or camera change
  is introduced. The first response-only implementation exposed a stale button
  when resizing to phone-class: `read_response` can fall back to the previous
  pass. Published shell regions now handle hidden/reappearing window controls.
  Tests also cover first-pass/resize behavior, clipped/transformed action bounds,
  latest-only snapshots and clearing between passes. All 306 focused tests,
  both actual-GPU frost tests and 10 world/24 composite parity cases pass.
  `unified-chrome-before` preserves ten old-viewer resize captures, including
  the clear button over a table title at 480×700. All ten final `unified-chrome`
  windows were inspected across four sizes and desktop restoration, in both
  mirrored layouts. `unified-chrome-system` passes all nine native interactions;
  its six annotation, panel-content, label-contrast, corner-helper, Halo and
  robot windows were inspected. Viewer `2d265947...` and gallery `6216d0e3...`
  match their live process images throughout capture. Owned QA instances stopped
  normally. Frame keys are computed outside egui data locks. This is scoped
  native acceptance; the full-catalogue replay is recorded above.
- Narrow-window layout continuation: two new regressions fail on the old
  fixed-step placement search even though side slots are available. The first
  native 641×400 window also reproduces overlapping tables and covered headers.
  That capture attempt remains NOT_ACCEPTED because its image-size check
  compared against the requested 640 pixels instead of the compositor's
  reported 641; the probe now checks the actual measured dimensions. A bounded
  nearest-first boundary search has been implemented ahead of the existing
  lattice fallback. It keeps content sizes and caller anchors unchanged and
  shares the same placement path for panels, selected labels and TF labels.
  All 302 focused tests, both actual-GPU frost tests and 10 world/24 composite
  parity checks pass. `unified-layout-slots-before2` preserves six inspected
  failing old-viewer windows; `unified-layout-slots` has the corresponding six
  inspected final windows with separated tables and fully readable headers.
  `unified-annotation-slots` passes all nine native interactions; its five
  full-resolution annotation, six-panel, corner-helper, mixed Halo and robot
  windows were inspected. Viewer `d2f2af83...` and gallery `6216d0e3...` hashes
  remained stable. The finite search checks at most 169 boundary candidates,
  then the existing 169 lattice candidates if necessary; it is not an arbitrary
  density packing guarantee. First new size assertions needed a 0.001-point
  floating-translation tolerance, without changing contents or geometry.
  The follow-up finding addressed by the shell-avoidance pass above: in the narrow
  640×500 left-side case, the shell's
  play/clear buttons cover a few pixels of the panel's left border. Their actual
  bounds are not among the reserved rectangles; only the instance label is.
  This is separate from the corrected inter-panel overlap/status-label case.
  Native evidence is preserved; the fix reserves actual shell bounds rather than
  hardcoded screen margins or removing world drawings.
- Previous live preview was `draw-world-preview-1992956`, PID 1992971, on
  `annotation-layout near` with seven world entities. Its 2468×1348 window was
  inspected and its executable matched `d2f2af83...`. It stopped normally before
  the shell-avoidance captures. All three isolated layout
  probes and the shared annotation QA instance stopped normally. The primary
  checkout was clean at this checkpoint; other simulator processes were not
  targeted. Known slab startup diagnostics remain, without fatal shader,
  pipeline, OOM or panic errors in the final layout/annotation captures.
- Shared unbacked text now selects a dark or pale-neutral glyph edge by the
  supplied foreground's linear luminance, without changing the foreground RGB,
  text, font, placement or existing 0.5/0.75-point edge offsets. Glass-backed
  labels and panels retain their single-pass text. Edge alpha comes from each
  text role's own ink alpha; transparent categories no longer leave a visible
  black outline. Two failing-before painter regressions reproduce these defects.
  The first native after-run improves dark captions and passes nine interactions,
  but a further regression catches polarity flips during opacity fading from
  premultiplied Color32 rounding (gray 111, alpha 66). `unified-label-contrast`
  remains NOT_ACCEPTED. The final helper chooses polarity from original supplied
  RGB, independently of faded ink coverage. All 298 focused tests, both actual-GPU
  frost tests and 10 world/24 composite parity cases now pass.
  `unified-label-contrast-before` records the inspected old-viewer palette and
  real spatial captions. Final `unified-label-contrast2` passes all nine native
  interaction checks with stable viewer `cc5d55b3...` and gallery `6216d0e3...`
  hashes. All six full-resolution scene windows were inspected: label palette,
  spatial, Halo, panel content, corner helpers and robot. The palette was also
  inspected in the live preview at night and after restoring daylight. These
  are scoped checks, not another 103-bay replay or a universal contrast guarantee
  for arbitrary colors, opacity and backgrounds. Partial alpha remains partial.
  No world material, corner geometry or public color contract changed. The
  previous preview `draw-world-preview-1886667`, PID 1886681, used that viewer
  on the mixed `halo overview` scene with 31 entities and the loaded Husky.
  Its final window was inspected and its executable hash rechecked; it stopped
  normally before the narrow-window layout verification.
  The isolated QA instance stopped normally. The known slab startup diagnostic
  remains, with no fatal shader/pipeline/OOM/panic errors in these captures.
- Mixed-scene cleanup removes redundant clearance geometry laid directly over
  FIELD's restored corner arms in `halo` and both `spatial` field examples.
  FIELD itself is unchanged; its dimensions, color, major grid, depth and energy
  remain intact. The old bounds captions keep their entity IDs, supplied text,
  tint, font size, normalized pose and terrain anchoring. Both standalone APIs
  remain in separate catalogue bays. The new regression fails on the baseline
  duplicate entities and passes after the cleanup; all 294 focused tests and
  10 world/24 composite parity comparisons pass. Native before/after verification
  is recorded in `unified-field-framing-before` and `unified-field-framing`.
  The before-run's two scene windows and the final Halo, spatial, corner-helper
  and navigation-section windows were inspected. Both native runs pass all nine
  interaction checks with stable requested/live hashes. Final viewer remains
  `5a7eee81...`; gallery is
  `ff6fa655cf5f1bf681cd66aaf9ff311a1b1bba7e93644c59511132cc1c4e33e2`.
  Halo now sends 31 rather than 32 entities; spatial sends 11 rather than 13.
  Standalone clearance and FIELD remain separately visible. This is scoped
  verification, not another full-catalogue replay. No renderer/material changes
  were made. Previous preview `draw-world-preview-1770571`, PID 1770586, was on
  `spatial near`; its 2468×1348 window was inspected. It was stopped before the
  text-contrast pass. Both isolated QA instances
  stopped normally; other simulators were not targeted.
- Follow-up visual finding addressed by the text-contrast pass above: the
  supplied dark-blue extent captions were hard to read because every unbacked
  foreground previously received a black edge. The adaptive treatment retains
  explicit text tint/alpha and approved glass. Duplicate-field removal alone
  did not fix this; raw and world-helper labels are not forced to neutral white.
- Continuing the same measured-layout contract through all six panel families.
  `unified-panel-content-before/panel-content.window.png` reproduces clipped
  titles, status text, units and bar labels, plus overlapping legend values on
  viewer `48048899...`. `panel-content-before/regression.log` preserves the
  failing title-bounds test. A shared prepared content layout now measures
  headers, captions, value/unit columns and legend labels before reserving
  annotation space; painting reuses those exact galleys. Panel width, fonts,
  caller colors, three glass finishes, chart values and corner treatment are
  retained. Headers/rows grow only as needed; bars retain their supplied slots,
  sparklines retain sample order and gaps, and legends retain their ramp.
  The first native after-run captured all 103 bays/12 sections; seven contact
  sheets and all six individual panel bays were inspected. It remains
  NOT_ACCEPTED because the subsequent handle drag did not move at the final
  B103 camera. The previous viewer failed a point/pose comparison at the same
  view. Source inspection found camera ground-height easing (0.4 s time
  constant); a four-second pre-input wait makes the previous viewer pass all
  nine unchanged interaction checks, without changing the camera or thresholds.
  The final `unified-panel-content2` run passes 293 focused tests, both actual-GPU
  frost tests, 10 world/24 composite parity cases and all nine native interactions
  at the final B103 camera. All 103 bays and 12 sections were captured; all seven
  contact sheets and seven full-resolution scene windows were inspected. The
  individual section images were not all inspected again. Viewer
  `5a7eee81194eddfbc2a8104ce9c3994809859abb9ec43878fd98b925b5408dcd`
  and gallery `5944bc429fba93f96f975be6ba8f83214a16102de3642d351367c8a90c2a88f3`
  hashes remained stable through capture. DPI fixtures supply native viewport
  pixel density rather than applying a zoom change to an old logical viewport;
  this is not cross-display native acceptance. The failed first run and old
  viewer countercheck remain explicitly NOT_ACCEPTED.
- Previous live preview was `draw-world-preview-1693353`, PID 1693369, on
  `panel-content`: all six panel types with long labels, units and status text,
  across the three approved glass finishes. Its 2468×1348 window was inspected
  and the process executable matched viewer `5a7eee81...` above. It stopped
  normally before the field-framing pass. The isolated
  final QA instance `draw-vfx-1670429`, PID 1670443, stopped normally. The known
  slab startup diagnostic remains; no fatal shader/pipeline/OOM/panic errors
  were found in the final capture or preview. Other simulator processes and
  the primary checkout's unrelated `scripts/control/ondrive_lab.py` edit remain
  untouched. Whole-system user approval and controlled performance remain open.
- Full current-image catalogue capture completed on viewer `273eac47f5259da5ca9da94864ad3fb3beba717fd7eb115fc953a3c835375fc7`:
  103 bays, 12 sections, all seven contact sheets, six full-resolution panel
  types, the mixed Halo scene, spatial/dense view variants and nine native
  interactions. All requested/live binary hashes remained stable. Inspection
  found a real table defect: the fixed column split clips the beginning of
  longer right-aligned values, including the `0` in `0–100 / unknown 255`.
  `unified-complete-catalogue/halo.window.png` and the failing regression in
  `table-layout-before/regression.log` preserve that evidence. Table layout now
  measures column widths, wraps complete strings and reserves measured row
  height before annotation placement. Existing width, fonts, finishes, caller
  colors and glow are retained. The first after-run fixed clipped values and
  captured all 103 bays again, but exposed a taller table behind the existing
  instance-status strip. `unified-table-layout/NOT_ACCEPTED.txt` records that
  failure. The host now reserves the actual painted strip bounds before laying
  out movable world annotations; raw centered text keeps its explicit position.
  Final scoped `unified-table-layout2` passes 290 focused tests, both actual-GPU
  frost tests, 10 world/24 composite parity cases and nine native interactions.
  Its table, mixed Halo, clustered annotation, six-panel and corner-helper
  windows were inspected at 2468×1348: complete table strings remain inside
  their cells and the taller table avoids the existing status strip. The final
  viewer is `480488993d1a28bf3c10c7ae1699792a588856930486440d9b002bebeeee0eb3`;
  executable hashes remained stable. This final status-reservation change has
  scoped native coverage, not another complete catalogue inspection.
  The fixture also exposed the legacy
  newline-separated `key=value` row encoding: embedded newlines in wire row
  values remain unsupported. Renderer-only multiline tests do not establish
  wire support; no API encoding change has been made in this pass.
- Continued the restored corner language through actual helpers: clearance and
  selected-waypoint brackets previously used thin unrelated widths; 2-D image
  boxes and explicit wire-only 3-D detections previously had uniform edges.
  Consolidated the canonical rule/corner tokens in the API and shared the renderer's partitioned
  frame primitive. Helpers may opt PLAIN geometry into `corner_frame=true`;
  ordinary PLAIN drawings remain unchanged. Preserve supplied extents, camera
  projection, scores, tracks, palettes, width units and client overrides. A new
  six-bay `corner-helpers` scene exercises real helpers instead of more raw
  cube samples. The baseline renderer fails the explicit-frame regression and
  its native helper image was inspected. Shared line/shape framing now retains
  full-strip distances, vertex tints and same-unit radius interpolation. During
  terrain preparation, bounded internal source-index metadata records authored
  corners; the frame builder decorates those spans, not new terrain samples.
  A curved-ground regression checks four corners after 80 terrain subdivisions,
  exact existing sample heights and the unchanged 36% total bold-edge share.
  Mixed metre/pixel loops retain their ordinary geometry rather than blending
  incompatible units. Rust/Python helper override and projection tests pass;
  the expanded parity fixture includes wire-only detections. Final scoped
  `unified-corner-helpers` acceptance passes 286 focused tests, both actual-GPU
  frost tests, 10 world/24 composite Rust/Python comparisons and all nine native
  interaction checks. Its 1226×1348 helper/corner-review windows were inspected;
  the new helper scene includes real clearance, FIELD, glass/wire detections,
  image-plane detections and selected route stations in separate bays.
- Previous live preview was `draw-world-preview-1524680`, PID 1524699, on the
  six-bay `corner-helpers` scene, viewer
  `480488993d1a28bf3c10c7ae1699792a588856930486440d9b002bebeeee0eb3`.
  Its 2468×1348 window was inspected; the matching isolated QA instance stopped.
  The known slab startup diagnostic remains, with no fatal renderer errors.
  Its process hash matched the verified viewer. It was stopped for isolated
  panel-content validation. No cross-DPI or controlled performance completion
  is claimed. Earlier preview instances have stopped.
  At the final check the primary checkout had an unrelated modification in
  `scripts/control/ondrive_lab.py`, which this task did not edit. The earlier
  separate debug PID 530570 had exited; no task command targeted that process.
- User-requested corner hierarchy is restored on HOLO/GHOST/SHIELD/FIELD BOX/RECT
  geometry. The cleanup had restricted accents to solid-only shapes while BOTH
  used a uniform outline; their remaining default width was only one pixel.
  Corners now have short 2.5-pixel arms and fine one-pixel connecting edges.
  Explicit widths/radii preserve their units with the same 2.5x corner ratio.
  Combined/wire outlines partition each edge instead of stacking another stroke;
  all eight cube vertices have exactly three arms. Original ring arc length and
  actual neighboring segments are retained for joins, dashes and fade. PLAIN,
  caller RGBA/color2, glass and glow settings are unchanged. The 64-instance
  detail budget remains. `unified-corners2` records 281 focused tests, both
  actual-GPU frost tests and 10 world/19 composite parity cases passing, with
  native before/after corner comparisons and the nine-shape geometry image
  inspected. The first implementation is superseded because its per-edge arc
  length reset was caught before final acceptance. This is scoped visual/test
  evidence, not a full-catalogue, interaction or performance rerun.
- Previous preview was `draw-world-preview-1244151`, PID 1244654, on `corner-review`,
  viewer `d9daef1727aacbc867aa78af5b897353a9c5000c8be36ca64c4541d4debc6298`.
  The 2468×1348 native window was inspected; that preview has stopped. The isolated
  capture instance stopped; the user's debug simulator PID 530570 is untouched.
  The separately known slab startup diagnostic remains; no fatal renderer
  errors occurred and requested/live executable hashes stayed stable.
- Annotation continuation reproduced two layout defects: a 256×158 panel in a
  500×200 viewport can fall entirely offscreen despite available room, and a
  panel moved below its anchor routes its leader through the content
  (`/tmp/gearbox-annotation-before.log`, two failing regressions). Labels, TF
  names and panels now use one placement/leader geometry module. A prepared
  annotation pass reserves explicit centered text and selected labels before
  laying out panels, then places ordinary attached labels around all reserved
  bounds. The actual glass, fonts, caller tints and glow parameters are unchanged.
  The first native pass separates the annotation bodies and passes all nine
  interaction checks, but its clustered screenshot shows leaders crossing
  neighboring text. `unified-annotation-layout` is therefore not accepted as the
  completed fix. A follow-up layout phase resolves every label/panel bound before
  painting; leaders and anchor marks are occluded behind neighboring annotations.
  This retains continuous paths where unobstructed, rather than adding glow or
  changing the glass. Offscreen panel anchors are culled instead of becoming
  unwanted screen-edge indicators. Follow-up `unified-annotation-layout2` passes
  279 focused tests, both actual-GPU frost tests and 10 world/19 composite parity
  cases. Its native clustered, six-panel and real-Husky/TF window screenshots
  were inspected; all nine interaction checks pass and executable hashes stayed
  stable. The first crossing-leader capture remains explicitly not accepted.
  No full-catalogue, moving-camera jitter or controlled performance claim is made.
- Before the corner continuation, the live preview was
  `draw-world-preview-1159906`, PID 1160459, on the clustered annotation scene,
  viewer `2393553add60d7a2e4a6c1a199f30d040b09bf50434bba5c9cf856ea1af5c7a4`.
  The user's separate debug simulator remains untouched.
- Previous geometry preview was `draw-world-preview-1046824`, PID 1047319, on viewer
  `bf2412d96473d4e964b85664179a6569aa8d7a4953cb3f28392ea79a2dd61695`.
  The 26-entity mixed geometry scene includes the corrected imported mesh with
  explicit HOLO treatment beside all nine procedural shapes. Its 1840×1000
  native window screenshot and the isolated 1600×1000 headless geometry capture
  were inspected; caller colors and existing energy materials are retained.
  The live preview stays open (scene clock paused). A 388-frame/6.47 s clip is
  retained, without claiming controlled performance or motion validation from
  frame count alone. `unified-mesh-geometry` records the scoped native evidence;
  the isolated capture instance stopped and executable hashes stayed stable.
  Final focused tests pass 272 cases, both real-GPU frost tests pass, and parity
  remains 10 world/19 composite (`/tmp/gearbox-normals-final-*.log`). The user's
  separate debug simulator is untouched. The known slab startup error remains;
  this is not a new full-catalogue or performance acceptance claim.
- Imported-mesh audit reproduced lost authored `vn` normals and ignored OBJ
  smoothing groups: sharp corners were always averaged together. Three added
  regressions fail on the previous loader, including an inline comment whose
  numeric text was accidentally parsed as an extra face corner
  (`/tmp/gearbox-mesh-normals-before.log`). The loader now splits vertices at
  authored-normal and smoothing-group boundaries, keeps shared smooth normals
  within a group, and treats flat faces independently. Invalid position/normal
  indices and non-finite OBJ vectors reject the mesh instead of silently changing
  topology. The normal fan-out follows the original
  [Wavefront specification](https://www.martinreddy.net/gfx/3d/OBJ.spec), with
  authored normals taking precedence over smoothing groups. An old slash-index
  test fixture was corrected to declare the normals it referenced. Native
  `unified-mesh-normals2` passes against an independent STL reference: flat and
  authored cases differ by just 0.006/0.012 RGB levels; deliberate smoothing
  remains distinct at 5.54. All three OBJ images were inspected. The previous
  viewer fails both hard-normal cases at 5.54 (`unified-mesh-normals-before`);
  its flat image was inspected. Both isolated instances stopped. The first
  fixed capture passed image checks but is not accepted because a test-only
  gallery rebuild changed its hash during capture; the rerun verifies all
  executable hashes again after the final image. The expanded suite passes
  272 cases, both separate GPU frost tests pass, and Rust/Python parity remains
  10 world/19 composite. The first gallery test compile used a nonexistent
  `.pack()` method; it was corrected to the existing pack/wire helper. The
  approved drawing materials and caller color controls are unchanged.
  The mixed geometry showcase now explicitly selects HOLO for its imported
  mesh, sharing the procedural shapes' body/feature-edge treatment. The actual
  MESH AUTO/PLAIN API behavior and the separate plain catalogue bay are unchanged.
- Continued API/renderer audit reproduced a terrain-placement mismatch for
  implicit single AXES, SHAPES and MESH drawings: the mesh builder supplies an
  origin pose, but the preceding drape pass saw an empty instance list and did
  nothing. A failing regression records missing 7 m elevation versus an explicit
  origin instance (`/tmp/gearbox-implicit-drape-before.log`). The drape pass now
  materializes the same implicit pose before sampling. Empty ghost trajectories
  stay empty and explicit sizes/rotations remain unchanged. The initial expanded
  focused suite passes 263 cases. The first native comparison was contaminated
  by tiny colored terrain flowers and moving background visible through glass;
  it is not accepted. The placement oracle now uses opaque geometry and removes
  isolated color components smaller than 32 pixels, with dedicated mask tests.
  Its original 0.98 IoU and 1 px drift limits remain unchanged. Native rerun
  `unified-implicit-drape2` passes: 0.993–0.999 mask IoU and under 0.04 px
  centroid difference for all three kinds, on viewer `d04516d0…`. The isolated
  instance stopped; the previous-binary counterfactual fails all three kinds
  by 100–129 px in `unified-implicit-drape-before`, confirming the missing terrain
  offset. All six passing images and two pre-fix images were inspected. The expanded
  focused suite passes 265 cases and parity remains 10 world/19 composite.
  No production material or energy behavior was changed. That checkpoint's live
  preview `draw-world-preview-937340` (PID 938032) uses that verified viewer and
  shows the full 253-entity exhibition, focused on imported-mesh bay B033. Its
  native window screenshot was inspected, with no fatal renderer errors; the
  known slab startup diagnostic remains. The user's debug simulator is untouched.
- Previous ghost/frost checkpoint gates passed: 260 focused tests (`/tmp/gearbox-current-260.log`)
  and both actual-GPU frost tests (`/tmp/gearbox-current-frost2.log`). Rust/Python
  parity remains 10 world and 19 composite matches. `git diff --check` passes;
  the primary checkout remains clean. No commits or pushes were made. Current
  source, logs and evidence are archived in `unified-frost-window3`; ghost
  native and pre-fix counterexample reports remain separately reproducible.
- Native glass resize coverage now passes through a separate, PID-scoped
  Hyprland QA window in `unified-frost-window3`, on viewer `4493fec4…`.
  All three finishes remain readable with matching live scene blur at actual
  1201×800, 801×1000, 1601×900, then 1201×800 again. All four native PNGs were
  inspected; each exactly matches the compositor-reported dimensions. The
  retained request is unchanged and the isolated window stopped. The first
  harness attempt failed before capture because
  this compositor uses Lua dispatchers rather than legacy command syntax;
  `hyprctl` also returned success status with textual errors. The probe now
  detects Lua dispatch support and requires an explicit `ok` reply, then checks
  actual window dimensions. The second attempt reached 1201×800 for a requested
  1200×800 and correctly rejected the old exact-size assumption. The probe now
  allows one compositor rounding pixel but requires screenshot dimensions to
  match the actual reported size exactly. Both first attempts are not accepted.
  No renderer or approved material changes were needed. The existing slab
  startup diagnostic remains; this is not a controlled performance result.
- Previous live preview was `draw-world-preview-758062`, PID 758653, on viewer
  `4493fec40d259749f266941a31a6f9d99c49b9e3e5084df1baadf7bf731de903`.
  Robot ghosts, attached badge, frame labels and nearby world elements are
  visible in the inspected `live-preview/robot.window.png` capture. The preview
  was replaced by the current preview above; the user's separate debug simulator was untouched.
  Older preview identities below describe historical checkpoints.
- Ghost lifecycle audit reproduced incomplete streamed models: after its first
  source part, a ghost permanently skipped late-loaded parts. The production
  lifecycle regression fails 2 copies versus the required 4 on the prior code
  (`/tmp/gearbox-ghost-late-before.log`). Ghosts now compare loaded, visible source
  entity/mesh topology, sharing discovery per machine each frame. Topology and
  visibility changes refresh copies; ordinary source motion preserves requested
  frozen poses. Unavailable machines clear copies and retry when available.
  The expanded focused suite passes 260 cases (`/tmp/gearbox-ghost-tests3.log`),
  including mesh data arriving after its entity and externally removed copies.
  Native `unified-ghost-lifecycle` passes on viewer `4493fec4…`: the actual Husky
  ghosts appear when their source loads, disappear on removal, and return on
  reload without resending the drawing. Loaded/removed/reloaded masks contain
  1576/0/1570 magenta pixels; all three screenshots were inspected. The isolated
  QA instance stopped. The old-binary native counterfactual fails as expected
  in `unified-ghost-before`: 1569 ghost pixels survive source removal. Its
  removed-source image was inspected; that isolated instance also stopped.
  Rust/Python parity remains 10 world and 19 composite matches.
- Frost resource verification now uses the real egui-wgpu registry and production
  `DrawFrost::update` rather than a simulated release callback. A test-only
  egui-wgpu dependency matches the existing transitive 0.34.3 package; the make
  GPU gate runs both blur and resize/release checks serially. Both pass in
  `/tmp/gearbox-frost-resize3.log`. Actual texture registrations are reused on
  stable updates and freed on source replacement, size/DPI changes, missing
  source/GPU, inactive camera, zero viewport and disabled overlays. This proves
  the production GPU resource path; the subsequent native window capture above
  separately covers visible resizing at the current display scale.
- Latest full renderer catalogue completed in `unified-current-catalogue`:
  103 bay and 12 section captures, all seven contact sheets inspected, all six
  panel families inspected at full resolution, spatial and dense glyph
  near/grazing/night views inspected, and all nine native interaction checks
  passed. A 375-frame, 60 fps spatial clip is retained. Renderer/live hashes
  match `d0242517…`; the single existing slab startup error remains.
- That run exposed a presentation issue: the compositor tiled the capture
  window to 912×1000, but section camera distances assumed a wide viewport.
  The styles-section screenshot clipped an outer example. Exhibition focus
  now fits all eight declared bound corners to the actual window aspect and
  viewer FOV with 10% margin. Existing closer review/ghost framing is unchanged.
  Wide, square and two portrait layouts pass geometric tests. First native
  portrait capture fixed the clipped beacon but exposed the label fixture's
  35 m cutoff at the fitted 36 m distance. That capture is not accepted for full
  visibility. Standalone style samples now use 80 m; other gallery label ranges
  are unchanged. Follow-up `unified-focus-portrait2` passes at native 912×1000:
  both the beacon and its neighboring label are fully visible. The screenshot
  was inspected; nine native interaction checks also pass. Viewer `22139330…`
  and gallery `cb9ceb2b…` hashes are recorded with the source archive.
  Focused suite: 258 pass (`/tmp/gearbox-focus-tests3.log`), one GPU-only test
  separately verified in `/tmp/gearbox-final-frost258.log`. Rust/Python parity
  also passes 10 world and 19 composite cases (`/tmp/gearbox-final-parity258.log`).
  `git diff --check` passes; the primary checkout remains clean. All edits remain
  uncommitted in this worktree. Preview PID 615900 is left usable; no owned QA
  instances remain. The initial new
  fixture test had a missing kind import; corrected before this passing rerun.
- Asset-lifecycle regression checks now exercise the actual `sync_draws` system
  and Bevy asset tracking through 64 retained updates. Mesh IDs stay stable,
  replaced grid textures are reclaimed, cleared/expired/retyped entities release
  their parts, and externally removed roots recover. Shared material caches are
  intentionally retained; releasing the cache also releases those materials.
  This is CPU/ECS asset-lifetime evidence, not GPU allocator or terrain proof.
  The focused suite passes 255 tests in `/tmp/gearbox-resource-tests1.log`.
- The dedicated live preview is now `draw-world-preview-615289`, showing
  uncertainty/forces (14 entities) on viewer SHA-256
  `221393304f489951f7403871c4a8e003387c5977668d3844cd288a527d5e85a7`.
  Its native window screenshot was inspected. The renderer source differs from
  the full catalogue build only by test-module registration; no shader/material
  behavior changed after that full run. Gallery framing was updated separately.
- Current projection comparison passes in `unified-stroke-projection2`: coarse
  versus 256-segment rendering stays above the original 0.95 mask-IoU threshold
  in near/far views and both directions (0.9615–0.9665). New viewer SHA-256:
  `d02425173f6dd92d419baf55d582872a9c5f33750fe356f9914a04405cac02a6`.
  Pre-fix counterfactual fails all four cases at 0.6732–0.6934 IoU
  (`unified-stroke-before`). Seven map/surface family captures completed and
  were inspected in `unified-projection-families`, with matching live/requested
  hashes and no fatal renderer error. The existing slab startup diagnostic
  remains. At that projection checkpoint the focused suite passed 253
  tests (`/tmp/gearbox-projection-tests1.log`), with 10 world/19 composite Python
  parity cases passing in `/tmp/gearbox-projection-parity1.log`.
- Stroke audit found screen-linear interpolation of world arc length and tint,
  plus near-plane clipping that moved endpoints without clipping their attributes.
  The shader now carries clipped arc/color/radii and reconstructs world progress
  from both endpoint clip depths. Flat ribbons retain world-plane interpolation.
  Native coarse-versus-dense and pre-fix comparisons are accepted as described
  above; this is bounded image agreement, not pixel-perfect equivalence.
- First projection comparison exposed a second shared cause: the grass depth
  offset changed clip-w, warping world patterns on flat strokes, heatmaps and
  ground surfaces. All three now use one depth-only projection helper preserving
  xy and w. The first partial fix failed its native comparison (~0.90 mask IoU),
  so that capture remains explicitly not accepted. The complete fix passes the
  second native comparison and the seven shared material-family captures.
- Native raw-wire and real Python-binding invalid-value checks now pass in
  `target/draw-vfx/unified-finite-api/`. Invalid updates leave retained headers
  unchanged, and chunk/delete/extend/expiry behavior still passes.
- API validation now rejects non-finite pose, scale, lifetime and per-item radii
  before changing the retained store. Finite signed/pixel/default sentinels and
  unknown F32 grid samples remain valid. Lifetime builders no longer silently
  turn NaN into a permanent drawing. Added atomic-rejection tests and a raw-wire
  native check. The initial focused suite passes 252 tests
  (`/tmp/gearbox-draw-finite-tests1.log`); native raw-wire and real Python-binding
  checks subsequently passed. `make test-draw-live --args=new`
  now isolates those checks from the interactive preview.
- User approves the current visual direction and asks to continue autonomously.
  Preserve Halo-derived world drawings, caller colors, energized elements and
  approved white/color/black glass. No HUD and no Death Stranding direction.
- Previous 250-test and GPU-glass checkpoint logs remain in
  `/tmp/gearbox-verified-current-tests.log` and
  `/tmp/gearbox-verified-current-frost.log`. The subsequent 253-test suite and GPU
  glass test passed after the final atomic APPEND rejection case:
  `/tmp/gearbox-projection-tests2.log`, `/tmp/gearbox-projection-frost2.log`.
  Native evidence remains checkpoint-specific, not whole-project completion.
- Corrected independent native grid oracle passes in
  `target/draw-vfx/unified-grid-cache-atomic/`: frame update differs from fresh
  geometry by 0.020 pixels (mask IoU 0.99967); anchor/pose-height updates match
  exactly. The fresh branch constructs all properties in one UPSERT. Frame and
  height PNGs were inspected. The pre-fix binary correctly fails the same probe:
  frame drift 3.80065 px / IoU 0.88625; pose-height drift 39.12259 px / IoU 0.22759.
  Counterfactual evidence: `target/draw-vfx/unified-grid-atomic-before/`.
- Previous live preview `draw-world-preview-311860` used viewer SHA `6c0fe4fe…`,
  showing the perception family (15 entities). Its screenshot was
  inspected. Previous probe machines disappeared with the old dedicated
  instance; the user's separate debug simulator was not touched.

- Dual-mode native sensor acceptance now passes after the scheduling fix:
  `target/draw-vfx/unified-sensor-both-fixed/`. Optimized/full mean channel deltas
  are 0.03723/255 and 0.00082/255, both zero magenta pixels. Inspected the viewer
  positive control and full-camera images. The missing-cascade crash did not
  recur through sampled activation and pause/resume. New built viewer SHA-256:
  `6c0fe4feff1de5a27c93aeee296dcf980d52a5bfe4c4d498c88233807b7c0312`.
  Rust/Python parity passes again (`/tmp/gearbox-camera-order-parity1.log`).

### Chronological progression notes

Pending statements below describe the state at that checkpoint; current
acceptance above and per-capture `ACCEPTANCE.md` / `NOT_ACCEPTED.txt` take priority.

- Focused suite now passes 250 tests, including the sensor scheduling regression
  (`/tmp/gearbox-camera-order-tests2.log`). One GPU test is explicitly excluded;
  a fresh native dual-camera run is rebuilding the viewer. Native grid update
  versus delete/recreate comparison is implemented but has not run yet.
- Lifecycle absence checks now propagate transport failures instead of treating
  every failed lookup as successful deletion or expiry. Sensor capture cleanup
  checks fatal renderer logs even when the camera client fails first.
- Full-camera probe uncovered a production scheduling race: sensor activation
  could happen after directional shadow cascades were built, then Bevy's render
  light preparation panicked on the missing active-camera entry. Sensor drive/
  arm now run before camera and directional-cascade preparation. A scheduling
  regression test and another native dual-mode probe are required before claiming
  the crash fixed. Failing evidence: `unified-sensor-both/viewer.log`.
- Terrain-grid cache keys now also include anchored sampling and effective pose
  height. Changing those metadata properties must rebuild the sampled surface;
  scientific colors/cells still do not change the geometry key.
- Optimized sensor-camera isolation passed with a native viewer positive control
  in `target/draw-vfx/unified-sensor-isolation2/`. Surface, stroke, scalar grid
  and real-mesh ghost were visible in the viewer but absent from the actual
  160×120 camera feed: mean channel change 0.2403/255, zero magenta pixels.
  Both sensor PNGs and the viewer PNG were inspected. Full-quality camera
  isolation is the next extension; this is not an all-camera acceptance claim.
  The first attempt failed connection/fixture setup and is not accepted.
- Current GPU backdrop-blur test and Rust/Python parity also pass:
  `/tmp/gearbox-frame-frost1.log`, `/tmp/gearbox-frame-parity1.log`.
- Latest focused suite: 246 pass, one GPU-only test excluded by the recipe
  (`/tmp/gearbox-frame-cache-tests1.log`). Release build passes. Current built
  viewer SHA-256: `765bb0686ab7050e46529d8ec7ad72b8b34bbc645c282c0bf3cb11629dea6461`.
  This is newer than the interactive preview; do not interrupt the user's
  exploration merely to run acceptance probes.
- Controlled native snapshot acceptance passed in
  `target/draw-vfx/unified-frame-isolated/`: 3.3554 m actual robot travel,
  zero snapshot-center movement through robot motion and glow/metadata updates,
  local-pose movement in the frozen frame, and replacement recapturing the frame.
  Image metrics and inspected PNGs back the result. The isolated instance stopped
  itself; the interactive preview was left untouched.
- Terrain-grid geometry keys now include the resolved sampling transform, not
  just local entity pose. A new test covers translation, rotation, scale, heights
  and reuse when only colors/cells change. Native frame-switching-grid comparison
  remains to be added alongside sensor-camera isolation and orthographic checks.
- User feedback on the live direction is now positive: "taking shape very nicely"
  and "just continue". Preserve this material/color/label direction while fixing
  remaining behavior; do not restart the design or remove energy effects.
- Added atomic scalar update masks while retaining the existing grouped/empty
  mask behavior. Python and CLI no longer erase pulse/flow/beam/fade when only
  glow changes, or erase geometry/colors when only panel metadata changes.
  Explicit zero values remain updates, and primary/secondary tint and colormap
  bounds can change independently. No read-modify-write client race was added.
- CLI update now carries all effect controls, secondary tint, scale, flags,
  palette bounds, panel finishes and metadata. Empty CLI changes are rejected;
  Python's no-argument update uses the header-safe `props` mask.
- Energized wire shapes now emit only outlines, including sector/frustum/decal
  paths. Both mode respects secondary RGBA and owns the boundary once, without
  adding the default broken-corner decoration on top of a full outline.
  Default solid energy, beams, caller palettes and approved panel glass remain.
- Added native Rust/CLI retained lifecycle verification and a real-binding Python
  update verifier. The new `fill-contract` scene separates all nine shapes into
  solid/wire/both comparisons; native verification of this checkpoint is pending.
- Initial focused update run passes 238 tests; the first shape-mode run passes
  240 tests. The later live verifier and gallery fixture still need the final
  integrated rerun. Logs: `/tmp/gearbox-update-tests3.log`,
  `/tmp/gearbox-fill-tests1.log`.
- First native comparison exposed a separate color-role bug: shape `color2`
  was also used as an annotation backing, producing unreadable pale rectangles.
  Shape/polygon outlines no longer color label backings. `label_background`
  provides an independent explicit override; TEXT and existing ruler/line
  annotations retain their color2 behavior.
- Native Rust/CLI lifecycle and Python selective updates pass in
  `/tmp/gearbox-draw-live2.log`. This includes atomic six-chunk/6,000-point
  replacement, delete canceling pending assembly, bounded extend, growing patch,
  paused/running simulated expiry, namespace boundary clear and fixture cleanup.
  Label-role changes are newer than this native image and still require rebuild.
- Frame audit found that every update released a SNAPSHOT drawing's frozen frame,
  so changing glow could move it onto the robot's current pose. The next fix
  separates retained-entity generation from revision and caches the frame anchor,
  not the whole posed drawing. Header/patch/extend changes must keep that anchor;
  replacement, frame change or toggling snapshot starts a new capture. Pose
  updates remain relative to the captured frame. No wire layout change is needed.
- The snapshot fix now passes focused tests (245 total) and is awaiting native
  frame-transition captures. Separately, heat geometry caching includes entity
  pose but not the resolved frame transform: switching a draped grid between
  frames at the same local pose can reuse terrain geometry from the old place.
  Include the actual sampling transform in that cache key before accepting
  frame-changing terrain grids.
- The first live frame capture was taken while the camera/scene was being
  interacted with; differing viewpoints prevent a controlled position comparison.
  Keep those images as interactive review, not snapshot acceptance. Repeat the
  frame probe in a separate headless instance so the user's preview stays usable.

## Earlier full-catalogue checkpoint — 2026-11-10

- The compact full catalogue completed successfully: 103 individual bays,
  12 section views, overview, spatial near/far/grazing/night views, dense
  samples, controls and native interaction feedback. Evidence:
  `target/draw-vfx/unified-catalogue-compact/`; command log:
  `/tmp/gearbox-unified-catalogue3.log`. Requested and running binaries match
  SHA-256 `2c047d307d8e2e6687ebc9a610e2f318538f925206d62a525825dbc02dcea79f`.
- All seven catalogue contact sheets were inspected, with full-resolution
  checks of the six panel families, square points and near robot TF names.
  The approved glass finishes retain their caller tints, blurred transmission
  and opposite foreground brightness. Square marks are square; TF labels are
  separated. No API kind, style, shape, helper or panel family was removed.
- Spatial near/grazing/night, 20,000 scalar splats with 480 cost rollouts,
  2,304 terrain glyphs and motion/glow/zero-alpha controls were inspected.
  The spatial movie contains 390 frames at 60 fps (6.5 seconds). These are
  visual checks, not statistical frame-time or sustained-resource acceptance.
- Native input acceptance passed on this same image: point/pose picks,
  changed handle drag, no hover-only mouse events, Step, both cancellations,
  menu selection and Escape dismissal. `interactions.jsonl` retains payloads.
- Latest focused suite passes 232 tests, with the explicit GPU-only test
  ignored by that recipe (`/tmp/gearbox-compact-gallery-tests1.log`). That
  GPU blur/transmitted-color test passed separately in
  `/tmp/gearbox-final-frost2.log`; Rust/Python parity passes 10 world and
  19 composite comparisons in `/tmp/gearbox-final-parity2.log`.
- Only the baseline slab-allocator startup error is present in the successful
  native log. The two earlier OutOfMemory capture directories remain marked
  `NOT_ACCEPTED.txt`; compacting the gallery is not a terrain allocator fix.
- The separate `target/draw-vfx/unified-motion-terrain/` run completed on the
  same image: energized ribbons, walls and beacon motion, terrain fields,
  actual Husky ghost bodies/wheels and the full interaction verifier. Its
  motion contact sheet was inspected; the recording has 374 frames at 60 fps.
  Sequential baseline/candidate dense timing checks also completed.
- Final continuity probes isolate vegetation occlusion from broken geometry.
  The identical 81-point, 5-cm white curve is interrupted by grass at 4 cm
  ground clearance; explicitly on-top, raised to 50 cm, or rendered flat, it
  is continuous. Captures and the reproducible command script are in
  `target/draw-vfx/unified-continuity-probe/`. No forced on-top policy was added
  to conceal normal scene occlusion.
- The final focused rerun passes all 232 tests, with the GPU-only test separately
  passed as recorded above (`/tmp/gearbox-final-tests3.log`). `git diff --check`
  passes. Changes are uncommitted and confined to the drawing worktree.
- `geometry overview` is open in `draw-world-preview-1723482`, PID 1723490,
  verified as the active, mapped, non-hidden native window. Its live executable
  matches the renderer hash above. `live-preview/geometry.overview.window.png`
  was inspected. The user's separate debug simulator was not stopped.

### Sequential timing observations

Both `unified-baseline-timing` and `unified-candidate-timing` completed five
35-second measurement windows at 1600×1000, using the same gallery client.
Baseline SHA-256 is `8711a3fb2c22777b02052eb83c0095efbd3631240abe9892826ff49873e48f70`;
candidate is the image above. The harness now checks allocation/validation
failures before and after every screenshot and verifies the running image.

| Scene | Baseline transparent CPU ms | Candidate transparent CPU ms | Baseline transparent GPU ms | Candidate transparent GPU ms |
| --- | ---: | ---: | ---: | ---: |
| Empty | 0.1999 | 0.1934 | 10.0106 | 8.5669 |
| Energized paths | 0.2130 | 0.2116 | 9.6028 | 9.6613 |
| 20,000 splats / 480 rollouts | 0.2780 | 0.2718 | 9.8737 | 9.6603 |
| Terrain fields | 0.2859 | 0.3538 | 10.6485 | 9.6063 |
| 2,304 terrain glyphs | 0.2706 | 0.3221 | 11.6182 | 9.4623 |

These are means of 3–4 logged moving averages, not independent-frame samples.
The headless loop is capped at 30 fps, so its roughly 33.38 ms frame times do
not establish uncapped performance. Unrelated native simulator/GPU work stayed
running; the empty-scene GPU difference already shows environmental variation.
No speedup, memory optimization or statistical performance acceptance is claimed.
Candidate RSS ranged from 1,843,444 KiB empty to 2,058,252 KiB after the last
scene; this short sequence does not establish a sustained leak/resource bound.
No allocation/validation failure occurred; both logs retain the known baseline
slab-allocator error. Raw logs, measurement windows, RSS and timing summaries
are stored in those two directories.

## Earlier unification checkpoints — 2026-11-10

- **Current priority:** user reports blocky/aliased, disconnected drawings,
  especially over terrain, across all families. Fix shared rendering before
  further styling. The user is going to sleep and explicitly authorized
  continuing without asking for permission/input. Keep the goal active.
- The user rejected repeated energy/review-system previews. Move through
  new families; fields, measurements, frames, sensors and geometry have now
  been inspected in that order. Do not return to the
  same eight effects as the default progress demonstration.
- Found a concrete terrain bug: the flat-stroke vertex branch projected
  `wb-wa` onto the entity XY plane and built the whole quad from `wa`.
  It therefore discarded the end point's elevation, making every draped
  segment a horizontal terrace. First fix interpolates the true 3-D endpoints
  for the ribbon centre while retaining planar width/pattern coordinates.
  Join antialiasing, cross-width terrain following and triangle conformity
  remain under investigation. The first patch has built, rendered in `fields`
  and passed 216 focused tests (one additional GPU-only test ignored), logs
  `/tmp/gearbox-terrain-joins-{build,preview,tests}1.log`. Internal joins now
  have one owner instead of blending two half-alpha edges; draped line lists
  retain a continuous strip per original pair instead of restarting caps and
  patterns every half metre. The API entity is not mutated by this conversion.
  Current follow-up fixes the contour's own pixel footprint at grazing angles
  and height-grid tessellation that incorrectly used coarse cell sizes on terrain.
- Wide world-width flat strokes now have an optional terrain lattice across
  their full footprint, not only their centreline. The shader retains the same
  material, caller RGBA, cumulative distance and animated patterns. Its pixel
  halo margin follows the sampled terrain slope. Ordinary screen-space strokes
  and dense splats keep the original compact vertex layout; the extra attribute
  exists only on meshes containing terrain patches. Subdivision has a per-mesh
  vertex budget. Close and grazing native fields views were inspected; swaths
  remain continuous across the visible undulations. Immutable captures are in
  `target/draw-vfx/terrain-continuity/`. The grazing image is from native PID
  1304483, running/release SHA-256
  `53635cbca83553795f1e29d99023c26f8e8829f745e23434ee060aba43ba57ad`.
  `/tmp/gearbox-terrain-patch-tests2.log` passes 221 focused tests (94 renderer,
  23 host; one explicit GPU test ignored). Release/preview2 also passed.
  Only the previously recorded slab-allocator startup error remains; no new
  missing-attribute, WGSL or pipeline validation error was logged. This is not
  proof for every view, huge geometry budget or final user acceptance.
- Draped raw triangles now subdivide their interiors, and bent ground surfaces
  recompute normals from the actual tessellated terrain rather than retaining
  their pre-drape lighting plane. Coarse grid-height cells no longer override
  terrain sampling resolution. Rigid instanced shapes are not bent.
- The next preview is `measurements`: separated ruler, angle, compass,
  scale bar, callout and coordinate frame. Rulers now use the common rule
  weight, one set of end caps (no stacked balls), per-point RGBA on ticks,
  and the same label metadata/background/glow controls as other annotations.
  The added ruler contract test passes. `/tmp/gearbox-measurements-tests2.log`
  passes 223 focused tests, including polygon boundary ownership. Native
  measurements rendered successfully in `/tmp/gearbox-measurements-preview3.log`.
  Explicit GPU backdrop blur/transmitted-color verification also passed in
  `/tmp/gearbox-measurements-frost2.log`. Colored numerical text remains caller
  colored; no theme rule overrides that contract. The earlier chained run
  ended with signal 143 after passing tests, so build/preview/frost were rerun
  separately rather than treating the interrupted chain as successful.
- Coordinate axes now share the 1.5-pixel default structural weight with live
  TF frames and rulers, keep their canonical RGB, retain explicit caller width
  and shading, and omit a redundant glowing origin ball. The frames example
  removes the crossing path/duplicate scale bar already covered elsewhere so
  pose arrays, covariance and odometry are separated. Native `frames` was
  inspected at overview using build SHA-256
  `a4c36103ba7b96615522ab1ff370313565365be7102acf20c4cb4f9689ff7e4c`;
  release build and 223 focused tests passed in `/tmp/gearbox-frames-{build,tests}1.log`.
  The 10 world/19 composite parity comparisons also pass. Live TF machine
  overlay inspection remains separate from these standalone coordinate axes.
- Moved the native preview forward to `sensors` and inspected the point cloud,
  scan, range cone, camera frustum and 2-D/3-D detection labels. Semantic scalar
  palettes and separate measurement shapes remain intact. Small secondary
  label text was too dark on the exposed soil: the current follow-up uses a
  lighter neutral secondary ink and a thinner keyline below 14 px, without
  recoloring caller-controlled identity text or changing panel films. Its new
  host contract test, build and native sensors recheck passed. The secondary
  score/track line is visible over the soil, and caller-colored icons remain.
  Native geometry shows all nine shapes, triangles, polygon holes and mesh
  samples; renderer hash is `ff8ff16c558ea5b2190436d73dc4442674d2efcfe0ba61a58487585d16fbdc02`.
  `/tmp/gearbox-all-draw-tests1.log` passes 226 focused tests, including the
  newly added existing TF-routing test; one explicit GPU test is ignored.
- Polygon fills with explicit outlines no longer paint a second shader border;
  solid-only fills retain their own edge. Separate outline colors, holes and
  geometry remain intact. Light-wall base strokes and alert strokes now use
  the same coverage/emission function as their filled counterparts. Glow is
  independent of body coverage and still honors explicit off/transparent colors.
- Follow-up terrain AA work reserves enough raster margin at shallow angles
  and measures halo reach in pixels for flat and camera-facing strokes alike.
  Native shallow-angle validation passed on SHA-256
  `2070b39e94fc46000d5c9eaeebec50a32a9075aa1320dfefc4e5b01b06b51e16`;
  `target/draw-vfx/terrain-continuity/fields.grazing.pixel-margin.png` shows the
  bounded halo and continuous ground-following swaths. It does not establish
  performance on huge terrain paths or visual acceptance of every family.
- The user set a new **active** goal to continue unifying **every element**.
  The focused six-example preview and test count are not a stopping point.
  Continue through the remaining material families, all helpers and existing
  interaction controls, then verify the whole final catalogue.
- Current extension in progress: Step, pick prompts and handle menus now use
  the same glass frame, finish parsing, sharp text and caller tint as panels.
  Removed the separate square/solid button skin and redundant pick crosshairs;
  pose picks use the shared anchor and an open chevron. Standalone controls
  request the GPU backdrop even without a panel. Native point/pose picking,
  handle drag mouse-down/update/up and Step full=0 payload checks passed in
  `/tmp/gearbox-unify-controls-native1.log`. The extended native pass now also
  proves point and in-progress pose cancellation. Menu selection exposed an
  existing event bug: transform-gizmo `is_focused()` includes hover, but the
  host treated its transition as mouse-down/up. The current patch tracks a
  real pressed/held/released drag and suppresses gizmo interaction behind an
  open menu. `/tmp/gearbox-menu-tests1.log` passes 227 focused tests; release
  build passes. `/tmp/gearbox-all-interactions-native3.log` proves no events
  on handle hover, a changed drag with normalized quaternion, point and pose
  picks, Step full=0, both cancellations, menu selection with the original pose,
  and Escape dismissal. The earlier two native attempts failed and exposed the
  hover bug; they are not passing evidence. Native hash for this check:
  `2b7f8773796747feee941767163480c5fba4aa591d048026c8acf89bf1c48fde`.
  Follow-up puts handle geometry below foreground menus so its axes cannot
  paint across menu text; the approved glass films are unchanged.
- Direct axes, live TF frames and transform handles now share the canonical
  axis palette. TF links use neutral structural ink and reference rule weight.
  Handle hit geometry is unchanged: transform-gizmo ties `stroke_width` to
  focus distance and arcball radius, so changing it is not a paint-only edit.
- Destination helpers keep one ring and one diamond instead of nested duplicate
  copies. Stable child IDs, radius, marker placement, draping and clearing remain.
  API/Python helpers and renderer corners share the same 0.18 bracket fraction.
  Rust/Python 10 world and 19 composite parity checks passed in
  `/tmp/gearbox-unify-controls-parity3.log`. Live TF inspection remains pending.
- Shared `energy_glass` now covers hologram/shape ghost, real robot ghost,
  field volume, shield, sensor image plane and frustum faces, decal and zone.
  Separate effect patterns remain; coverage does not depend on glow.
- Removed duplicate full outlines, shield floor-ring decoration, micro-grid
  layers and backed-label glyph outlines. Frustums use one-pixel structural
  rules, no arrow-glow style or forced additive blending, and no second
  surface-painted far border. Coincident near/far planes paint one border.
- The main exhibition now has **103 bays instead of 134**: removed 12 repeated
  panel variants and 18 generic style samples already represented by helpers;
  ruler/scale-bar aliases now share one measurement bay.
  All 13 kinds, 20 styles, 9 shapes, 45 helper roles and 6 panel families remain
  covered by tests. Three panel finishes remain represented without multiplying
  every family. No public API IDs or functions were deleted.
- The approved white/black/color panel films, GPU backdrop blur, caller RGBA,
  scientific palettes and animated Tron/ribbon/tube/beacon effects are retained.
- Release build passed in 1m01s (`/tmp/gearbox-unify-build4.log`). Native
  `review-system.window.png` was inspected at 01:46:35 +0100; the six examples
  are separated and the frustum no longer stacks its far border. The scene is
  open in `draw-world-preview-805142`, PID 805180, verified focused/mapped/not
  hidden. Running and release SHA-256 both equal
  `dfb3574fb985d0d07b9282be9f0bfc73d0bac1c6878622d74083c5e0ac0954ab`.
- The first restart failed with Wayland `NoCompositor`; a retry with
  `WAYLAND_DISPLAY=wayland-1` opened successfully. Root cause was not established.
  The known baseline slab-allocator startup error remains; no new shader
  validation error was reported. `/tmp/gearbox-unify-parity4.log` passed the
  10 world/19 composite Rust/Python comparisons. The full focused drawing rerun
  passed 212 tests in `/tmp/gearbox-unify-tests5.log` (29 API/store, 10 wire,
  13 CLI, 35 Python, 17 gallery, 88 renderer, 20 host). One GPU-only frost test
  is ignored by this recipe and was not rerun in this material-consolidation pass.
  `git diff --check` passed.
- This pass has not yet received user visual acceptance. It is not completion
  of the broader interaction, all-family visual and performance gates.
- The uncertainty scene exposed example-layout overlap: the candidate fan
  extended through the covariance volume, and the largest particle glyphs
  touched adjacent samples. The fixture now keeps the fan in its own region,
  retains every candidate/weight, and uses smaller particle glyphs. No public
  helper defaults, covariance extents, palette or scientific values changed.
- Native maps and plain-reference scenes were inspected: scalar ramps, grid
  patches, missing-cell holes, categorical colors, plain fills and point shapes
  remain distinct rather than being recolored into one blue style. Geometry
  fixtures now explicitly vary caller tint across the same nine shapes.
- The native robot scene exercised the real Husky link tree and all body/wheel
  ghost surfaces. Its TF names revealed the five-position label avoidance limit:
  crowded link names exhausted the candidates and overlapped. The current
  follow-up adds bounded three-column label placement, preserving literal
  centered text positions, and tests clustered names at the viewport edge.
- Chevron glyph wings still used two independent round caps at their shared
  tip. They now use the same joint ownership as other strokes while retaining
  mirrored arc coordinates, so both wings brighten/pulse toward the tip with
  their original color/width. This covers particles, pose arrays and route
  chevrons, not only freehand lines. Native robot/uncertainty checks passed on
  `e029e49811e21f1d6867eda6a7bfa68c3afe364396376f04594c21b21a9c2e4d`:
  TF names are separated and the chevrons retain their energized appearance.
  `/tmp/gearbox-unified-tests-final.log` passes 229 focused tests (97 renderer,
  26 host, one TF-routing test); the separately run GPU frost test also passes,
  as do the 10 world/19 composite Rust/Python comparisons and `git diff --check`.
- A separate all-catalogue capture alongside the live preview failed at startup
  with GPU OutOfMemory allocating the terrain `blade records` buffer, followed
  by invalid blade bind groups. This occurred before catalogue rendering, not
  in a draw shader. Failed evidence is retained in `target/draw-vfx/unified-catalogue/`.
  Retrying serially in `unified-catalogue-serial` after stopping only this task's
  live preview; the user's other simulator/processes are untouched. The preview
  must be reopened after capture. Do not claim a passing full-catalogue capture yet.
- The serial capture also failed: crossing terrain regions during the old
  460-metre-wide exhibition triggered allocation failure during field texture
  replacement. Some later window captures existed despite the renderer's fatal
  error, so **neither capture run is accepted**. The harness now checks fatal
  rendering errors before/after every screenshot and resets the camera between
  scenes. The exhibition keeps the same 103 bays/18-metre spacing, but packs
  them into a spiral around the real machine, within 90 metres on each axis.
  IDs, helper coverage and per-bay bounds are preserved; redundant floating
  section titles are omitted. This limits the preview's terrain travel rather
  than modifying the terrain allocator or touching other applications.
- The full overview exposed TF names ignoring the distance fade used by other
  annotations. They now share the existing 60–90 metre fade; near names and
  explicit drawing distance overrides are unchanged. The shared-fade regression
  passed with 230 focused tests before the subsequent square-point fix.
- Plain/default square point marks were rendered as spheres; only SPLAT honored
  the square subtype. The renderer now retains square shape, caller RGBA and
  width for plain/glowing point marks too. A regression covers all three styles.
  Compact catalogue and square-point tests/build/native checks are pending.

### Earlier remaining-work list (superseded by latest checkpoint)

- Finish native clustered-TF-label and continuous-chevron checks; the uncertainty,
  map, plain and robot scenes have been inspected, without revisiting the same
  energy/review-system demo as the default scene.
- Menu foreground ordering passed in the native screenshot; all interaction
  payload and hover/cancel/menu gates passed in `/tmp/gearbox-final-interactions1.log`.
- Rebuild the gallery after the ruler/scale-bar alias consolidation and retain
  complete API/style/shape/helper/panel coverage with non-overlapping bays.
- Recheck time-varying effects and dense scenes on the final image. The captures
  above are visual/functional evidence, not a controlled performance comparison.

## Instructions

- Latest visual preference: retain stronger corner accents on geometry, especially
  the three short edge segments meeting at each cube vertex. The user explicitly
  likes that hierarchy; keep long outlines finer rather than thickening every edge
  or compensating with glow. Preserve the restored treatment during unification.
- Latest correction: **make drawings more similar, then remove duplicates**.
  The user rejected drifting into exhaustive catalogue checking instead of
  making the visual system coherent. Prioritize actual shared materials,
  typography and reduction of redundant decorations; retain energized
  effects and distinct data/interaction meanings. Do not alter the approved
  panel white/black/color materials while unifying the remaining drawings.
- Latest feedback on the live three-finish comparison: **"WAYYYYYY coloer"**.
  Keep this material direction as the baseline; it does not accept the still
  unverified full drawing exhibition or interaction/performance checks.
- Latest feedback on the stronger white finish: **"looks good to me now. continue"**.
  Preserve this accepted material and continue the broader drawing validation.
- Earlier white feedback rejected the subtle brightness nudge: **more white**.
  White now uses a visibly milky 50–58% film instead of 18–26%; it still
  transmits the GPU-blurred world. Color/black coverage, tint, glow, data and
  frame geometry are unchanged. Native build passed in 1m12s. Preview
  `draw-world-preview-501202` was verified focused, mapped and not hidden;
  `review-finishes.milky-white.window.png` shows visibly lighter white panels
  with blurred tracks still visible. Its running image matches release hash
  `33c6856386aff3678cfe1dbf19434fb19abf1d2bafb723757e3b21f100a2bf24`.
- Earlier follow-up requested only a slightly whiter white finish. Raised its neutral
  film offset from 172 to 188; tint coefficient, 18–26% film alpha, GPU blur,
  transmission and the color/black variants are unchanged. Rebuilt and opened
  for review; this small adjustment was subsequently rejected as insufficient.
- Previous feedback: the actual blurred-scene glass is **"wayyyyyyy netter"**,
  but too saturated. Preserve that glass and offer API-selectable **color**,
  **white + color** and **black + color** finishes. White/black are soft neutral
  mixtures, not opaque cards; keep caller tint, alpha, blur and bounded glow.
- Earlier explicit request: **more transparent, frosty and blurred** panels.
  Implement real scene-backdrop blur, not another alpha-only approximation;
  retain sharp text, charts and restrained caller-colored light at the edges.
- The user called the dark-glass/light-content replacement **"way nicer"**
  and asked for its opposite too. Preserve that dark version as the default;
  add light tinted glass with dark contents as a separate API-selectable
  variant, rather than overwriting the approved direction.
- The first inverse variant was rejected as **too bright**. Its backing must
  be muted smoky color, not near-white. Reduce backing RGB, not its opacity,
  so the scene cannot wash out its dark contents again.
- The user rejected both low-coverage panel film and the subsequent dark-ink
  reversal. **Background and contents must have opposite brightness**: either
  clearly dark backing with light contents, or clearly light backing with
  dark contents. Text outlines do not substitute for this separation.
  Research actual game treatments before changing it again.
- Latest panel clarification: **colored translucent glass like the Tron
  elements**, neither opaque dark/colored cards nor completely clear outlines.
  Keep a visible caller-tinted glass body, common luminous frame and readable
  data. The fully transparent experiment was also rejected.
- The user approved the restored direction ("niiiice", "you are in right
  direction") and emphasized that **color must remain API-settable**.
  Uniformity means shared construction, not mandatory blue. Axis conventions
  and scientific palettes retain their data meaning; tintable elements must
  honor explicit entity and per-item colors, including alpha.
- Latest reset: the user rejected the flattened visual pass and the panels.
  Restore the recognizable sci-fi vocabulary, including Tron light walls,
  flowing luminous ribbons, beams, ripples, shields and holographic scans.
  Glow must be **visible but restrained**, not effectively zero. Unify all
  drawings and all six panel families, not only the first review scene.
- Latest correction: **retain energized elements and visible motion**. Only
  excessive glow/glare was unwanted. The first mixed scene was rejected as
  the overall direction; it is evidence of operation, not visual approval.
- Provide a comprehensive scattered/spaced exhibition of every element with
  no overlap, consistent proportions and shared visual treatment.
- Halo Infinite primary, Halo 5 secondary; Death Stranding excluded.
- World-space robotics drawings, no new game HUD. Labels are the first priority.
- Shared structural detail, restrained light, explicit data and color preserved.
- Show intermediate native previews and continue through every drawing family.
- Use the Oslo Make recipes, patch edits, no Python file editing or commits.

## Discoveries

- Terrain-grid geometry depends on resolved frame placement, anchored sampling
  and effective pose height, not just dimensions and local pose. Native probes
  now compare updates against atomic fresh creation and reject the pre-fix image.
- Sensor activation must precede camera and shadow-cascade preparation. The
  native full-quality camera probe reproduced the old missing-cascade crash;
  the ordering fix passes focused and actual optimized/full-camera checks.
- Engram tools were unavailable in this continuation. Checkpoints and evidence
  are persisted here and in capture directories; no Engram save is claimed.
- Mara's viewport exposes its rendered scene through the active camera's
  `RenderTarget::TextureView` and `ManualTextureViews`; the host shares the
  same wgpu device/queue. This permits a host-owned blur without modifying
  the Mara checkout, reading pixels back to the CPU or blurring UI text.
- The viewport source view is sRGB-aware, whereas egui expects a normal
  unorm texture containing gamma-encoded pixels. The frost pass samples
  linear RGB and explicitly gamma-encodes its final dark/light textures.
- Frost design: one separable GPU blur at quarter resolution, capped at
  1024x1024, reused by all panel polygons. The real scene passes through
  unchanged in light mode and with 0.55 linear transmission in dark mode;
  an 18–26% caller-colored film (50–58% for milky white) sits over it. Text and geometry render
  afterward. GPU texture registrations are
  released on resize, source changes, loss of demand and host drop.
- The first frost shader's luminance clamps flattened the actual field into
  nearly uniform gray cards. Its GPU blur test passed, but the native image
  contradicted the requested transparent-glass appearance. Removed those
  clamps. The revised GPU test must preserve coarse scene contrast while
  smoothing fine checker detail. Do not equate shader operation with visual
  acceptance or claim worst-case 4.5:1 for translucent panels over every scene.
- Light-only surface and ghost branches caused geometry to vanish when glow
  was disabled. The new materials separate body coverage from optional emission.
- Real mesh edges need welded position adjacency and crease/boundary selection;
  displaying every tessellation diagonal creates noise. Derived ghost meshes
  are cached separately, without mutating the robot's source meshes.
- Labels, TF names and panels previously used three unrelated visual systems.
- Wire labels use newline separators. Any batched identity/detail extension
  must encode each item's metadata without introducing literal separator lines.

## Accomplished so far

- Current continuation: fixed frozen-draw update placement, selective client
  updates, shape fill/outline and annotation color roles, draped-grid cache
  invalidation, and sampled sensor-camera ordering. Added native regression
  probes and preserved the user's approved materials, tints and energy effects.
- Refreshed the live perception preview on the current release image; controlled
  acceptance probes use separate instances and stop only those instances.
- Unification pass in progress: procedural holograms, shields, field volumes,
  sensor planes, decals/zones and real robot ghosts share `energy_glass` and
  lighting in `draw_common.wgsl`. Hologram/shape-ghost and decal/zone shader
  branches are consolidated. Removed redundant micro-grid overlays and
  double-painted full outlines; box/rect accents keep short panel-like corners.
  Removed automatic shield ground-ring geometry and its now-unused shader.
  Beacons retain their purposeful rings/beams; Tron/ribbon/tube energy is
  unchanged. Backed annotations now use single-pass text like panels rather
  than four duplicated outline glyphs. Explicit unbacked text keeps its
  legibility treatment and supplied RGBA. The first native build/energy
  preview passed, and 211 focused drawing tests passed after adjusting the
  old fixtures that counted double outlines. The live `review-system` scene
  now places hologram, field, shield, robot ghost, sensor frustum and the
  approved white data panel together without overlapping. This is a focused
  implementation preview, not another catalogue capture sweep.
- That preview exposed a remaining inconsistent frustum: it forced arrow-glow
  additive thick strokes and painted the image-plane border twice. Replaced
  them with the shared one-pixel rule, ordinary lit edges, caller-controlled
  additive flags and a single shader-owned image-plane border. Explicit
  caller widths remain unchanged. The later native result is recorded above.
- Removed the 12 repeated panel variants just added to the catalogue. Six
  distinct panel families remain, with all three finishes represented; the
  separate finish-comparison scene remains available. API style IDs and
  scientific palettes are not removed. The main gallery also omits generic
  style samples when a real helper already demonstrates that exact style;
  all distinct style IDs, kinds, shapes and helper roles remain covered.
- The earlier capture recorded 134 individual bays and the energy animation
  before this consolidation. It ended after the viewer disappeared before
  world-controls/interactions; no cause was established. Those captures are
  historical evidence, not acceptance of the new shared material. Do not
  restart a long catalogue sweep instead of showing the unified result.
- Real GPU backdrop blur now visibly transmits the field/tracks while keeping
  text and charts sharp. The user approved this direction after the unclamped
  `review-colors-light.window.png` capture (2026-11-10 01:05:32 +0100).
  `/tmp/gearbox-frost-gpu4.log` passed the GPU checker/edge/color test;
  `/tmp/gearbox-frost-tests4.log` reports 203 passes and one GPU-only test
  ignored there (run separately). The command wrapper later returned 143;
  use individual complete test reports as evidence, not that wrapper status.
- Added `panel_finish=color|white|black`, CLI `--panel-finish`, and a native
  `review-finishes` comparison scene. White/black select opposing ink and
  lower-saturation film at the same 18–26% coverage. Color preserves the
  stronger tinted version and uses `panel_theme` for ink polarity. Missing
  finish follows theme (dark/default -> black; light -> white). Release build
  passed in 1m09s, gallery in 3.30s. Native comparison is open as
  `draw-world-preview-364056`; inspected `review-finishes.window.png` at
  2026-11-10 01:14:04 +0100. All six samples are separated; the middle column
  shows the blurred tracks through the neutralized glass. The user approved
  the direction. The first, insufficient white-only adjustment was opened in
  `draw-world-preview-405163`, captured as `review-finishes.whiter.window.png`
  at 2026-11-10 01:16:46 +0100. Its running executable matches the release
  SHA-256 `4c48fdc0972a237556f1ad548f901e972f6e24c754c4db401cb9ceb50df9ea57`.
  `/tmp/gearbox-finish-build2.log` passed (1m02s including lock wait), and
  `preview2.log` passed. The user resized the window between previews;
  these two screenshots are not pixel-aligned comparisons. Real scene blur
  remains visible and the shared placement avoids overlapping panels.
  `/tmp/gearbox-finish-tests1.log` passed 208 drawing tests, one GPU-only test
  ignored there; `/tmp/gearbox-finish-gpu1.log` passed that GPU test separately.
  `/tmp/gearbox-finish-parity1.log` passed 10 world/19 composite entities.
  The known baseline slab allocator startup error remains unfixed; no new
  shader validation error appeared in this preview.
- Central viewer visual tokens and all-style default profiles, including FIELD.
- Shared crisp label/frame/anchor/icon primitives; fading bands removed.
- Bundled Rajdhani Medium/SemiBold with its SIL Open Font License. This is a
  licensed adaptation choice, not a claim about Halo's original typeface.
  [Upstream font files and license](https://github.com/google/fonts/tree/main/ofl/rajdhani).
- World labels, TF labels, chart headings, values and existing interactions use
  the isolated drawing font families, retaining host fonts and Unicode fallbacks.
- Labels support caller-supplied `label_category`, `label_detail`, `label_icon`
  and `label_selected`. Raw text is not uppercased or interpreted as telemetry.
- Panel alpha now applies to its text, frame, leaders and data. A low gauge
  value no longer invents a warning color; caller color remains authoritative.
- All surface branches and ghost bodies have non-emissive structure; ordinary
  scientific/idle profiles are static. Explicit route/pulse controls remain.
- Shared route stations, waypoint selection, detection corners and world helpers
  replace disconnected pillar/glow treatments. Stable identities and native
  chunking remain the client contract.
- First native release Make build passed in 1m03s; gallery Make build passed.
- First actual mixed preview opened and was inspected at
  `target/draw-vfx/live-preview/halo.window.png`. The native instance was
  `draw-world-preview-3581653`. It contains a real loaded Husky, two ghosts,
  labels, route markers, bounds, field grid, scalar map, ruler and data panel.
  The green ghost and magenta route remain visible with glow disabled.
- No shader validation errors appeared in this first mixed view. The known
  baseline slab-allocation startup diagnostic remains separate and unfixed.

## Rejected static-first pass

The earlier 0.08–0.16 effect defaults combined with tiny shader coefficients
and subpixel halos effectively removed the luminous sci-fi appearance.
Restoring motion alone did not address the user's requirement. The first
exhibition and existing panels are rejected, not a baseline for acceptance.

A comprehensive `exhibition` gallery is being added with individual spaced
bays, a coverage/focus manifest and on-demand modal interaction examples.
World legends opt in through `world_anchor=true`; ordinary fixed legends are
unchanged. `label_max_distance` controls optional label/panel distance fade.
Hidden collision/proxy visuals are excluded from robot ghosts without
excluding legitimate visible USD material subsets.

## Fresh visual pass in progress

- Reinspect the actual liked `gentler-glow/hero.window.png` and archived source,
  plus the Halo reference boards, rather than interpreting glow as unwanted.
- Restore distinct luminous effects, with bounded 2–4 pixel stroke halos,
  bright cores and visible volume/scan/wake details. No global bloom change.
- Rebuild all six panel families around one 256-pixel frame, typography scale,
  cut corners, accent lighting, leader/anchor and data hierarchy. Explicit
  per-series colors, signed bars, units, ranges and colormaps remain data.
- Two close review scenes: `review` has only the six separated world panels;
  `energy-review` shows eight distinct sci-fi effects in two spaced rows.
  The exhibition remains the full coverage case, not a substitute for close-up
  inspection; the latest consolidation reduces it to 104 distinct bays.
- Fresh native build passed (`/tmp/gearbox-restart-build1.log`). Actual
  `live-preview/review.window.png` shows all six panels without overlap.
  `energy-review.window.png` shows eight luminous families; the 7.25-second,
  435-frame `energy-review-20261109-235807.mp4` was inspected at 0.5 and 1.5
  seconds, showing changed ring radii and sensor sweep direction. Camera
  framing is being lowered/closer for volume inspection.
- `/tmp/gearbox-restart-tests2.log` passed 26 API/store, 10 wire, 10 CLI,
  28 Python, 14 gallery, 75 renderer and 10 host drawing tests. Matching
  parity log passed 10 world and 19 composite entities. Later color-contract
  fixes are newer than these results and require another full run.
- Color audit found real gaps: ignored CLI waypoint/particle overrides,
  helper fade overwriting caller alpha, MESH colormap bypassed by a gray
  fallback, per-item TEXT retaining a default blue accent, and GHOST ignoring
  per-pose colors. Scoped fixes and regression coverage are in progress.

## Historical checks and intermediate rejections

- Both opaque cards and fully clear panels were rejected. The first
  `glass_frame` used a caller-tinted 18–30% coverage film and bright bounded
  edges; relying on glyph outlines through this film was also rejected.
  These are translucent materials, not actual backdrop blur.
  `/tmp/gearbox-glass-build1.log` passed; the native `review-colors` preview
  was opened and inspected. The user rejected its interior readability.
- `/tmp/gearbox-glass-tests1.log` passed 195 drawing tests; matching parity
  log passed 10 world and 19 composite entities. These precede the latest
  dark-interior panel contrast experiment and do not validate it yet.
- The dark-interior experiment passed 197 drawing tests and 10/19 entity
  parity, but its actual native preview was rejected as unreadable.
- Fresh primary-reference inspection: Eric Dies' Halo Infinite campaign
  co-op/design-system boards show light text over darkened imagery and dark
  text on genuinely bright selected surfaces. Omer Younas' FOB board uses
  a dark field with light identities and colored accents. These are visual
  observations, not copied game shader settings.
  [Halo design boards](https://www.mrdies.com/halo-infinite-presentation).
- Xbox's contrast guideline calls for 4.5:1 on standard-sized meaningful
  text/elements, measured against the least contrasting background area.
  [XAG 102](https://learn.microsoft.com/en-us/xbox/accessibility/xbox-accessibility-guidelines/102).
- Current replacement: caller-hued dark glass with 90–94% coverage, plain
  light text (no multi-pass glyph outlines), light signal cores with caller
  edges, supplied bar colors and unchanged scientific ramps. A regression
  checks both light text levels against black/white worlds under linear and
  gamma compositing at full entity opacity. Native build/preview passed;
  all six were inspected in `review-colors.approved-dark.window.png`.
  `/tmp/gearbox-panel-contrast-tests2.log` passed 198 tests and its parity
  run matched 10 world/19 composite entities. The user approved its direction.
- Added `props.panel_theme=light|dark`, matching CLI creation option
  `--panel-theme`, and `review-colors-light` with the same six colored panels.
  Light keeps the shared translucent film and frame but pairs pale tinted
  backing with dark ink/graph cores. Contrast, alpha, actual text rendering
  and palette tests exercise both themes. The first light native preview
  opened correctly, but its 220–253 backing RGB was rejected as too bright.
  Latest light backing is 165–205 RGB, still 90–94% coverage, with dark
  foreground and unchanged caller-colored edges. Native build and preview
  passed (`/tmp/gearbox-panel-light-build2.log`, `preview2.log`); all six
  were inspected in `review-colors-light.muted.window.png`. Preview
  `draw-world-preview-60726` remains open. `/tmp/gearbox-panel-light-tests2.log`
  passed 200 drawing tests; matching parity passed 10 world/19 composite
  entities. Muted-light user acceptance remains unconfirmed.
- Latest user question asks whether both whites/blacks are being softened
  and made more transparent. Clarified actual state: only the light backing
  RGB was reduced; opacity remains 90–94% and the approved dark version is
  unchanged. Softer dark backing and increased translucency on both are
  **not implemented**. Keep content contrast when pursuing that next refinement.
- CLI PANEL creation bypassed common style/annotation forwarding; routing
  it through `restyle` now carries panel theme and explicit glow controls
  while preserving chart values, per-item palettes and colormaps.
- `/tmp/gearbox-halo-build1.log`: first native release build passed.
- `/tmp/gearbox-halo-gallery1.log`: first gallery build passed.
- `/tmp/gearbox-halo-preview1.log`: native mixed scene and screenshot passed;
  preview left open for user feedback.
- `/tmp/gearbox-halo-tests1.log`: initial integrated run stopped at a newly added
  CLI test missing positional fixture arguments. The fixture was corrected;
  a complete rerun and parity result are still required.
- Near robot inspection, the full 13-kind/20-style catalogue, disabled controls,
  actual interactions, day/night/density and performance evidence remain pending.
- Subsequent source changes are newer than the first preview binary. Rebuild
  and record exact live executable hashes before final visual acceptance.
- User feedback on the mixed preview has been requested asynchronously; work
  continues while it remains unanswered.

## Next Steps

- Preserve the new implicit-instance terrain and imported-normal regressions.
  Their native previous-binary counterexamples distinguish real fixes from
  cosmetic changes. Keep catalogue verification focused on affected families
  rather than repeatedly replaying every already-accepted element.
- Continuation is active: the user explicitly rejected stopping after a single
  pass. The full native catalogue has now been refreshed after the API and
  projection fixes; acceptance remains checkpoint-specific.
- Preserve verified scalar update, shape fill/outline, label-color, frozen-frame,
  dual-camera exclusion and native grid transition contracts. The pre-fix grid
  counterfactual and updated perception preview are now complete checkpoints;
  continue with remaining API validity and view-dependent rendering cases.
- Perspective stroke sizing, mesh/grid cleanup, ghost streaming/lifecycle and
  actual-GPU frost target resize/release now have regressions. Preserve these
  checkpoints while checking remaining view-dependent behavior. Native window
  resize screenshots now complement the GPU registry test; cross-display DPI
  visual checks are not claimed. Orthographic
  evidence is only relevant if an actual viewer/API path supports that mode.
- Non-finite pose/scale/lifetime/radii rejection now passes store and real-host
  tests, preserving unknown F32 grid cells and signed finite sentinels. The
  pre-fix projection counterfactual and shared surface/map captures are complete.
  The fitted portrait section is now verified. Preserve these renderer and
  asset-lifecycle checkpoints while checking remaining view/resize edges.
- Retain the serial timing results and limitations; they are not a controlled
  statistical benchmark while unrelated host work remains active.
- Preserve the accepted panel films and API colors. Any later user refinements
  should start from this shared system, not restart the design independently.
- Keep the terrain allocator OutOfMemory failures and pre-existing slab error
  visible as separate limitations, not silently described as fixed.

## Key Learnings

1. Terrain continuity requires true endpoint elevation, full-width sampling and
   single-owner joins; adding blur or increasing glow cannot repair geometry.
2. A screenshot can still be written after a GPU allocation failure. Capture
   acceptance must inspect the renderer log before and after each image.
3. Handle focus includes hover; only a real press/hold/release should emit drag
   lifecycle events. Menus must suppress the underlying handle interaction.
4. Plain near-ground lines can be correctly occluded by grass. Compare flat,
   raised and explicit on-top variants before changing depth semantics.
5. Scalar update masks must preserve sibling effect controls; metadata-only
   changes must not invoke the backward-compatible full-header update mask.
6. Frozen drawings retain a frame anchor across revisions, but replacement
   starts a new generation. Local-pose changes compose in the frozen frame.
7. Shape outline tint and annotation backing are separate color roles.
8. Controlled probes need a separate viewer while the user explores the live
   preview; camera interaction invalidates screen-space movement comparisons.
9. Sampling cameras must activate before camera and shadow preparation in the
   same frame. Otherwise extraction can see an active view with no cascade data.
10. A fresh-geometry oracle must create the complete entity in one UPSERT; using
    UPDATE in the reference branch can reproduce the cache bug being tested.
11. World arc lengths and colors need perspective interpolation using endpoint
    clip depths; near-plane clipping must also clip their attributes.
12. Grass depth bias must preserve clip-w or long flat surfaces distort their
    internal patterns despite having unchanged projected vertices.
13. Non-finite geometry/header rejection must leave the retained entity and its
    lifetime unchanged; scientific unknown grid samples remain valid data.
14. Drawing assets return to baseline after clear/expiry through Bevy's handle
    tracking; bounded shared materials remain cached until their cache is dropped.
15. Window-manager tiling can override the requested capture width. Section
    framing must use actual window geometry rather than a landscape assumption.
16. A fitted camera can cross a fixture's distance cutoff. Native framing checks
    must confirm the label is still rendered, not just that its bay fits on screen.
17. Ghost source readiness is not a one-time flag: model parts and mesh assets
    may arrive later. Compare topology while leaving ordinary pose motion frozen.
18. Frost target cleanup can be checked against the actual egui texture registry;
    source identity changes matter even when dimensions stay identical.
19. Native window tests must inspect dispatcher replies and actual captured size:
    a zero exit status can contain an error, and this compositor rounds width by
    one pixel.

## Relevant files

- `bin/gearbox/src/draw/assets.rs` — imported OBJ corner normals, smoothing
  groups, valid topology and shared cached mesh data.
- `scripts/capture-draw-normals.sh` / `tests/draw_normals_images.py` — native
  OBJ/STL shading oracle with stable executable checks and counterexample support.
- `bin/gearbox/src/draw/drape.rs` / `scripts/capture-draw-drape.sh` — implicit
  singleton placement and native equivalent-origin-instance regression.
- `bin/gearbox/src/draw/visual.rs` — shared visual tokens and effect defaults.
- `bin/gearbox/src/draw/mesh.rs` / `draw_stroke.wgsl` — line/point/shape structure.
- `bin/gearbox/src/draw/draw_surface.wgsl` — bounded technical surface recipes.
- `bin/gearbox/src/draw/ghost.rs` / `ghost_edges.rs` / `draw_ghost.wgsl` — cached
  genuine edges and shaded ghost bodies, preserving missing-normal/subset support.
- `bin/gearbox/src/draw/heat.rs` / `draw_heat.wgsl` — scalar/category/unknown data.
- `bin/gearbox/src/host/draw_labels.rs` — typography, icons, frame and placement.
- `bin/gearbox/src/host/draw_frost.rs` / `draw_frost.wgsl` — shared live scene
  blur, bounded GPU targets and per-panel backdrop sampling.
- `docs/DRAW_COLOR_CONTRACT.md` — API finish choices, defaults and examples.
- `bin/gearbox/src/host/draw_overlay.rs` / `host/mod.rs` — panels, TF, interaction.
- `bin/gearbox/assets/fonts/` — bundled unmodified fonts and their license.
- `crates/gearbox-api/src/wire/draw_widgets*` — shared composite geometry.
- `scripts/control/gearbox_client.py` / `crates/gearbox-cli/src/cli/draw.rs` —
  matching client geometry, explicit options and metadata forwarding.
- `crates/gearbox-api/examples/draw_gallery*` — mixed scene and acceptance cases.
- `scripts/preview-draw.sh` — persistent scoped preview with near/far views.
- `scripts/capture-draw.sh` / `.make.lua` — native capture and test gates.
- `bin/gearbox/src/sensor_cameras.rs` — sampled camera activation ordering.
- `scripts/capture-draw-sensors.sh` / `draw_gallery/sensor.rs` — real camera
  transport isolation probes with viewer positive controls.
- `scripts/capture-draw-grid.sh` / `tests/draw_grid_images.py` — native grid
  metadata/frame transitions compared with newly created geometry.
- `bin/gearbox/src/draw/draw_common.wgsl` / `draw_stroke.wgsl` — depth-only bias and perspective
  attribute interpolation shared with terrain heat/surface materials.
- `scripts/capture-draw-strokes.sh` / `tests/draw_stroke_images.py` — near-plane
  and distant coarse/dense geometry comparisons, including pre-fix failures.
- `bin/gearbox/src/draw/lifecycle_tests.rs` — retained geometry and texture
  lifetime checks using production synchronization and Bevy asset tracking,
  including incremental ghost model loading and recovery.
- `scripts/capture-draw-ghosts.sh` / `tests/draw_ghost_images.py` — native real-model
  load/unload/reload checks without changing the retained ghost request.
- `scripts/capture-draw-frost-window.sh` — owned-window three-finish resize
  captures with compositor and PNG dimension verification.
- `scripts/draw-contact-sheets.sh` — manifest-counted native catalogue sheets.
- `crates/gearbox-api/examples/draw_gallery/focus.rs` / `scripts/focus-draw.sh` — aspect-aware exhibition
  camera fitting, preserving explicit non-catalogue review views.
