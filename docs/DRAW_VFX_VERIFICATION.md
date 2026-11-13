# Drawing VFX verification

## Current checkpoint

- **2026-11-09 direction reset:** the user rejected the later label appearance
  and requested coherent Halo Infinite / Halo 5 drawing detail, not additional
  glare or blur. Death Stranding is excluded. See
  [`DRAW_HALO_DESIGN_SYSTEM.md`](DRAW_HALO_DESIGN_SYSTEM.md).
- Implementation subsequently resumed. The user rejected the static-first
  mixed preview and clarified that energized motion must remain, without the
  excessive glare. A comprehensive spaced exhibition is now being implemented.
- Current evidence and remaining checks are in
  [`DRAW_HALO_IMPLEMENTATION.md`](DRAW_HALO_IMPLEMENTATION.md). Older captures
  and test totals below are historical, not acceptance of the current source.

## Historical first-pass checkpoint

- Worktree: `.work/draw-vfx`, branch `feat/draw-vfx`, base `a957b38`.
- Research and implementation cover all 13 wire kinds, 19 explicit looks and
  their composite robotics widgets.
- Native drawing tests: 23 API/store tests, 6 gallery-contract tests and 32
  renderer/geometry tests passed on 2026-11-09.
- All fourteen catalogue scenes were captured in the native Vulkan viewer.
  Actual point/pose picks, changed handle poses and Step feedback passed.
  The corrected robot shows complete ghost bodies and wheels, with no missing
  attribute or shader errors. Only the baseline-reproduced startup warning remains.
- The user approved the visual direction, requested more, and asked for slightly
  less glow in the first path/trail/beacon scene. A 10–12.5% reduction to those
  default glow values passed the same 61 tests and a new native hero capture;
  explicit client overrides remain
  unchanged. The hero's explicit wall override was reduced from 1.1 to 0.95.

## Commands and environment

The project uses Oslo recipes, not GNU Make. The new targets are `test-draw`,
`build-draw-gallery`, `draw-gallery` and `capture-draw`.

```sh
nix develop --offline --command oslo make test-draw
nix develop --offline --command oslo make capture-draw \
  --args='target/draw-vfx/review window hero,geometry,panels'
nix develop --offline --command oslo make draw-gallery \
  --args='INSTANCE maps'
```

The first test run failed because the host's shared-memory filesystem was full
(30 GiB). No user files or shared-memory objects were deleted. Tests passed in
a private Linux mount/user namespace with its own 4 GiB `/dev/shm`:

```sh
unshare --user --map-root-user --mount sh -c \
  'mount -t tmpfs -o size=4G,nosuid,nodev tmpfs /dev/shm &&
   nix develop --offline --command oslo make test-draw'
```

Live viewer and client processes must share the same namespace. `capture-draw`
starts a uniquely named instance, keeps its configuration under the capture
directory and stops only that instance on exit. The script records the binary
hash, base revision, tracked diff, screenshots, scene listings, native log and
at-least-five-second hero video. Capture success alone is not visual acceptance.

## Evidence so far

- `/tmp/gearbox-vfx-tests.log`: initial shared-memory failure, 22/23 API tests.
- `/tmp/gearbox-vfx-tests-isolated.log`: successful 23 + 2 + 29 tests.
- `/tmp/gearbox-vfx-capture-first.log`: completed native build and three scenes.
- `target/draw-vfx/first/`: hero, geometry and panel viewport/window captures;
  `hero.mp4` is 7 seconds / 420 frames / 60 fps. Native Wayland window captures
  are 1226 x 1348 because the compositor tiled the requested 1600 x 1000 window.
- `/tmp/gearbox-vfx-capture-catalogue.log`: nine remaining scenes completed.
- `target/draw-vfx/catalogue/`: sensors, uncertainty, fields, maps, frames,
  plain, robot and the two pick requests. These pick screenshots alone do not
  establish interaction acceptance. Robot meshes really loaded from the Husky
  USDZ; its ghost copies exposed a missing-normal pipeline error.
- First-run inspection: ribbons have transparent shoulders and readable
  chevrons; actual surface energy rises, rather than only running along a
  rail; plain fills remain restrained; heatmap unknown cells remain holes;
  the six panel kinds are readable. Some fixture labels/gizmos overlap.

Existing unrelated Molla unused-variable warnings remain. Source-level review
caught and fixed additive-alpha handling in the new wall/orb stroke paths and
a repeated-endpoint crest segment. The ghost uniform update required Bevy's
mutable asset binding; that compile issue was fixed before the passing run.

The viewer reports a startup `slab_allocator` unallocated-key error before
the gallery sends any drawings. The clean, rebuilt `a957b38` baseline reproduced
it at `2026-11-09T21:03:49.207957Z`, in
`target/draw-vfx/baseline-variants/viewer.log`. Source inspection points to an
extracted empty mesh being copied after allocation was skipped. It predates
these changes and remains unfixed; this is not a clean-log claim. Shader
validation failures and missing vertex attributes are fatal capture errors.

The robot investigation found two older completeness bugs: imported flat-shaded
meshes intentionally omit normals, and material-subset children are not
`UsdPrimRef` nodes. The ghost renderer now handles optional normals with a
geometric fragment-normal fallback and includes actual USD subsets through
their parent prim. Source meshes remain unchanged, and attached overlays are
excluded. Three focused regressions pass; `final/robot.near.viewport.png` confirms
the missing body surfaces are restored.

The negative-glow review also found a newly introduced disappearing additive
trail core. Its source radiance is now separate from the optional halo.
The disabled-controls gallery includes this case.

## First-pass native completion

`target/draw-vfx/final/` contains all fourteen native scenes, a 7.25-second
435-frame hero video, complete robot ghosts, and accepted live point/pose,
changed-handle and Step events. `final-variants/` contains day/night,
near/far/grazing, disabled/transparent controls and dense samples. The
live-executable hash matches the requested binary; changed-source archives
include the explicit drawing source allowlist, not the entire build tree.

A sequential baseline/candidate headless comparison at 1600x1000 was capped at
30 fps. Transparent-pass GPU means were similar in that pair (hero 7.3143 vs
7.2704 ms; dense 7.3294 vs 7.3363 ms); CPU timing varied even in the empty scene.
Other host compilation was active. This is not a statistical performance win,
elapsed-performance acceptance or a memory-optimization claim.

## User-guided extension in progress

See `DRAW_WORLD_LANGUAGE.md` for the current DS2/Halo direction. The user
explicitly excluded game HUDs, requested intermediate live previews, approved
substantially reduced hero glow, rejected flat/unlit new symbols, and clarified
that labels are their highest-priority DS2 reference. The native preview is
under `target/draw-vfx/live-preview/` and is deliberately left open.

The new five-helper API, FIELD style, anchored draping and label rendering need
their own final native/API evidence. The first integrated extension gate passed
23 API/store, 10 API-unit, 8 gallery and 18 offline Python tests, then caught an
incorrect Bevy affine-method name; that was fixed. A subsequent gate passed 39
renderer tests. CLI coverage was expanded after its first filter ran zero tests.
A parity run caught temporarily mismatched default glow while client changes
were still being applied; it must be rerun on settled sources.

Still outside the recorded matrix: actual sensor-camera exclusion, all
world/odom/link/snapshot motion combinations, bloom-off/MSAA-off comparisons,
all palettes and sustained resource-lifetime stress. Do not infer those from
the catalogue captures.
