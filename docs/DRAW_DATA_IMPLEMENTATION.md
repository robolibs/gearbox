# Spatial data implementation checkpoint

Active objective: implement `plans/PLAN.md` in `.work/draw-vfx`, preserving the full
Release A and B scope. Started 2026-11-10 from `a957b38` plus the existing dirty drawing
work. No commits or unrelated application changes are authorized.
Do not run `oslo make docs`: that existing recipe stages and commits the working tree.

## Status

| Phase | Status | Evidence / remaining work |
|---|---|---|
| 0 baseline | Verified | Existing CPU/parity/GPU/live/camera gates pass; native cloud/voxel/grid/panel/trail baseline inspected; seven deterministic fixture/profile/oracle tests pass |
| 1 upload expiry | Verified | Old guard fails three regressions; all seven plugin tests, 345 focused tests, and native live lifecycle pass with the fix |
| 2 typed contracts | Contract gates verified | 146 CPU/native-service/JSON/wire checks; generic identity facade, bounded status/list shapes, capture metadata and 37 cross-client golden packets. This is not runtime dataset storage or high-level streaming-client acceptance |
| 3 storage and streaming | Verified | 139 stream + 149 contract + 349 drawing checks; actual Bevy compatibility, reset and retained capacity pass; deterministic off-lock validation/publication/listing regressions pass |
| 4 compact cloud renderer | Verified | [Requirement matrix](../plans/research/phase4-acceptance.md): finite projection, compact GPU records, scientific color/alpha/depth/frost, page deltas and native lifecycle gates pass; 500 broad drawing/GPU/frost executions, rebuilt normal preview and three inspected views |
| 5 spatial hierarchy, LOD and residency | Verified | [Provider integration](../plans/research/phase5-provider-integration-checkpoint.md) and [Phase 9](../plans/research/phase9-release-a-checkpoint.md): projected-error selection across datasets, hysteresis, complete-parent fallback, provider-backed paging with public `PageResidency`. Selection runs off the render thread. Known limits: a delta over paged storage leases every base page and fails with BUDGET rather than exceeding the cap; LOD error and upload work are recorded per frame by `draw-probes`, not in `STATUS` |
| 6 occupancy octrees | Verified | [Octree checkpoint](../plans/research/phase6-octree-checkpoint.md) and [Phase 13](../plans/research/phase13-catalogue-checkpoint.md): scoped topology validation, compact renderer, ROI unknown cells, 100k/500k bench. Opaque cells draw apart from translucent ones, and faces covered by a same-state neighbour are dropped (not in wire style) |
| 7 sensor and external adapters | Verified | [Adapters checkpoint](../plans/research/phase7-adapters-checkpoint.md): capture-pose export, LiDAR assembly, `robot-scan`; `test-draw-adapters` passes in the final run |
| 8 picking, clipping, inspection | Verified | [Picking checkpoint](../plans/research/phase8-picking-checkpoint.md): exact picks for every kind (cloud, octree, surfel, mesh, volume slice, terrain), turned and inverted clips; `test-draw-data-native` 36 PASS |
| 9 Release A | Verified | [Phase 9 checkpoint](../plans/research/phase9-release-a-checkpoint.md): low-memory and reference pass every gate on the clean rerun after the removals (100k live stored→visible p95 229 ms against 250); see [acceptance](DRAW_DATA_ACCEPTANCE.md) |
| 10 surfels and mesh blocks | Verified | [Phase 10 checkpoint](../plans/research/phase10-surfaces-checkpoint.md): oriented discs, indexed blocks, normal arrows, dirty-block reuse, `surface-lab`; `test-draw-surfaces` |
| 11 sparse volumes | Verified | [Phase 11 checkpoint](../plans/research/phase11-volumes-checkpoint.md): TSDF/ESDF/scalar blocks, slices with unknown checker, bands and contours, `FIELD_QUERY` with support and spacing, `distance-field-lab`; `test-draw-fields` |
| 12 layered elevation | Verified | [Phase 12 checkpoint](../plans/research/phase12-elevation-checkpoint.md): shared geometry, real holes, layer-only updates, rolling windows equal to a fresh map, `terrain-layers`; `test-draw-fields` |
| 13 final catalogue | Verified, EDL deferred | [Phase 13 checkpoint](../plans/research/phase13-catalogue-checkpoint.md): `preview-draw-overview`, `capture-draw-overview`, shared page acquisition, guide and protocol docs. Eye-Dome Lighting is not implemented. User visual approval is separate and not yet given |

## Removed 2026-11-13 by the user

Everything that was neither visualization nor the simulator:
- the PLY/PCD readers, the PointCloud2 layout converter and the OctoMap leaf converter;
- the CLI `volume`, `elevation` and `roll` commands, which read JSON files;
- the `gearbox-python` extension and the Python typed-data client (`gearbox_data.py`,
  `gearbox_data_wire.py`) with their tests; Python keeps the legacy `gb.draw` calls;
- the vendored peerbus fork and its Cargo patch: requests use the pinned upstream peerbus;
- the research, audit and recheck documents.

Sections below that describe these are history.

## Compact point records — 2026-11-14

Cloud points drop from 48 to 12 bytes of position plus the attribute words their color
mode reads: 16 bytes for RGBA, scalar, axis, range and category colors, 20 for age, 12
for constant color, plus 4 with a radius attribute and one validity bit where values can
be missing. Source rows moved to a CPU-only table that is empty when every row is valid.
Legacy compact points use 12-byte positions and samples. The residency probe's GPU cap
scales with the record (256 to 64 MiB), keeping one bank resident and both over cap.

## Releases A and B — final checkpoint, 2026-11-13

- Every phase is implemented. The gate list and both bench profiles ran on the final
  tree; results and commands are in [DRAW_DATA_ACCEPTANCE.md](DRAW_DATA_ACCEPTANCE.md),
  evidence under `target/draw-vfx/3d-data/final/`.
- EDL is deferred.
- User visual approval of the overview is recorded separately.
- The sections below are earlier checkpoints, kept as written; where one says
  something is unavailable or open, the table above supersedes it.

## Verified page-provider foundation — earlier checkpoint

The [provider checkpoint](../plans/research/phase5-page-provider-checkpoint.md)
separates charged page metadata from payload leases and verifies bounded exact-byte
reloads. PagedGeneration retains dataset/scan metadata without owning the original
resident Generation. Eleven tests cover release/reload, source identities, failure
atomicity, cancellation, concurrent loads and memory pressure. The final streaming,
normal/feature cloud and native LOD/residency regression chain passes. Native motion
regression records 992 view-frame observations and 736 completed draws with no
failed/skipped frames; exact CPU/GPU cleanup remains intact. Three normal preview
captures were inspected and the matching live binary remains open.

This is not integrated runtime CPU paging: registry/worker/render demand still
uses resident generations. Connecting providers to those owners and dataset
lifecycle is next. No Phase 5 checkbox or later release is closed by this foundation.

## Continuous camera-motion audit — preceding checkpoint

The [motion checkpoint](../plans/research/phase5-motion-audit-checkpoint.md) adds a
fixed-size, probe-only audit for every rendered frame during actual camera paths.
The 100k/1M/5M cases audit 212/196/90 frames per view, including many pending fine
replacements, with no skipped/missing/empty/invalid cuts or resource violations.
Actual completed-draw receipts and 18 new motion images complement the original
static image gates; this is not pixel readback of every frame. All 76 native image
checks and 201 feature / 193 normal CPU tests pass, plus 19 focused cases. Exact
cleanup, unchanged caps and source-bank identity are preserved. The rebuilt normal
preview is hash-matched, inspected and open. Providers, telemetry, remaining
lifecycle/fairness and broader moving-region scenarios still prevent full Phase 5
acceptance.

## Admission headroom — preceding checkpoint

The [headroom checkpoint](../plans/research/phase5-admission-headroom-checkpoint.md)
charges shared styles once per dataset and reserves actual bounded coarse indices,
preserving both upload windows and all auxiliary/external allocations. The rebuilt
normal overview now admits 77 fine pages and no coarse pages; its sparse edge strip
is gone in inspected captures. The near view still shows sampling texture. Suites
pass 198 feature / 193 normal CPU tests, 20 focused tests and 58 native image checks.
All three native suites use one renderer hash. Divergent views receive 40/39 fine
pages with source-exact coarse coverage, peak at 258,457,744 GPU bytes, and clear to
the exact 71,011,921-byte CPU baseline and zero GPU bytes. A caught stale camera
readiness assertion now checks actual source banks before and after capture. This
does not establish every-frame motion, providers, telemetry, lifecycle or Phase 5
acceptance. The hash-matched normal preview remains open.

## Mixed-page residency — preceding checkpoint

The [mixed-page checkpoint](../plans/research/phase5-mixed-residency-checkpoint.md)
adds byte-fair, deterministic page admission and source-exact mixed fine/coarse cuts.
Shared pages are charged once; inactive shares and unaffordable pages do not block
other refinement. Native cameras looking at separate 3.15M-point banks each retain
38 fine and 10 coarse pages, including after swapping regions. All 58 native image
checks pass at one renderer hash, including unchanged color reuse, pressured
replacement, source scale and exact cleanup. CPU runs pass 196 feature / 191 normal
tests with 12 standalone GPU skips each. Mixed-case GPU peak is 244,191,840 bytes
under the 256 MiB cap. The normal preview was rebuilt/restarted, hash-matched and
inspected in three views; close-up texture and a narrow far-edge sampling pattern
remain follow-up work. This does not
close providers, every-frame continuity, telemetry, remaining lifecycle gates or
full Phase 5; the mixed case retains higher conservative error on its coarse pages.

## Spatial sampling coverage — preceding checkpoint

The [coverage checkpoint](../plans/research/phase5-sampling-coverage-checkpoint.md)
reproduces the long survey bands before rasterization, then removes them using
deterministic in-place spatial partitioning and outward-rounded Euclidean projection
bounds. The unchanged 5M-source overview regression covers every ground bin with
960,288 source samples and 1.724904 px maximum error. All 183 normal / 188 feature
CPU tests and 54 native image checks pass; the CPU runs explicitly skip 12 standalone
GPU tests each. The residency quality oracle now requires settled source-exact cuts
within the original error/cap limits rather than gratuitously filling the point cap;
16 invalid-cut self-tests and all original pixel checks pass. Exact CPU baseline and
zero-GPU cleanup are preserved. Final normal near/grazing/overview captures were
inspected and the hash-matched preview remains open. Fine point/LOD texture close up
and the remaining Phase 5 gates are not accepted by this checkpoint.

## Earlier selection and Phase 4 checkpoints

The preceding [progressive sampling checkpoint](../plans/research/phase5-progressive-sampling-checkpoint.md)
adds nested leaf prefixes with exact source identities, projected-error bounds and
intermediate hysteresis. The current source passes 186 probe-feature CPU tests,
181 normal CPU tests and 54 native image checks. The five-million-source normal
preview was rebuilt and inspected in three views, but substantial banding remains
even with 1M actively selected points. This is verified selection/resource progress,
not visual-quality or full Phase 5 acceptance. The remaining quality, provider,
fairness, continuous-motion and lifecycle requirements are unchanged.

The subsequent [Phase 5 GPU selection checkpoint](../plans/research/phase5-gpu-selection-checkpoint.md)
uses the source-preserving hierarchy for actual camera-driven cuts. Initial native
two-view rendering and resource-lifecycle proof passes. The later native scale gate
passes 100k/1M/5M sources, and the temporary source-count guard is removed without
raising the visible-point or memory limits. Selection now consumes independently
owned spatial metadata instead of requiring resident source payloads. Source-cache
eviction/reload now follows active/pending selection demand, with initial native
oversized-source proof. Compact source-preserving records now pass direct bank
transitions, but fair divergent-view admission, every-frame continuity, and
external providers remain incomplete, so this is not full Phase 5 acceptance.
That normal-preview review found sampling-density bands; the current spatial
coverage checkpoint above fixes that specific defect.

The [Phase 4 matrix](../plans/research/phase4-acceptance.md) closes the renderer
foundation against actual GPU/native evidence. Extreme finite radii no longer cause
NaN, overflowed coverage or false offscreen intersections. The successful chain
includes all required tests, native recovery/fairness/moving frames and three fresh
normal-preview views. Phase 5 hierarchy/LOD and later representations remain open;
this does not establish the controlled performance or final catalogue gates.

## Charged extraction and changed-page rendering — 2026-11-11 (preceding checkpoint)

The [current checkpoint](../plans/research/phase4-extraction-delta-checkpoint.md)
reserves extraction scratch before cloning and keeps that charge until deferred
application or cancellation. Inline appearance names eliminate valid-path temporary
name allocations, measured with a test-only allocator observer. Both name and
extraction tests pass in normal and probe-feature builds.

The live million-point probe changes one full middle page and one short final page.
It verifies exactly 16 position and 32 attribute bytes per changed sample, plus
separate style/placement uniforms; unchanged pages are not reuploaded. Exact new
generation receipts, resource caps, quiescent cleanup and fresh-incarnation reload
pass. Three native survey views and all six current fairness captures were inspected.
Final broad/GPU/frost checks pass 492 executions. The normal preview is rebuilt,
hash-matched and inspected in near/grazing/overview views. Radius/projection evidence
remains open before whole Phase 4 acceptance.

## Bounded continuous uploads — 2026-11-11 (earlier checkpoint)

The recreation-order starvation regression is fixed. Shared uploads now also retain
at most two in-flight batches; completion callbacks release their permits and charges.
Native stress revealed a separate whole-frame backlog in the headless renderer, now
bounded by actual submission fences rather than reducing the producer rate.

The unchanged continuous test passes: fourteen 786,432-point source replacements,
six typed/legacy peer updates in 326–435 ms (3,000 ms limit), final exact generation
15 receipts in both original views, staging at most 8 MiB, and exact clear to
71,011,905 CPU bytes and zero drawing GPU bytes. All six images were inspected. The
secondary test view still has the known brighter/noisier terrain treatment; this is
not an accepted terrain-rendering change. The startup empty-mesh allocator diagnostic
is fixed and now explicitly rejected by the preview/native launch guards.

Evidence: [headless backpressure](../plans/research/phase4-headless-backpressure.md),
[upload window](../plans/research/phase4-upload-window.md),
[empty meshes](../plans/research/phase4-empty-mesh-fix.md), and
`target/draw-vfx/cloud-upload-churn/native-headless-window.log` (exit 0). The fresh
`verify.log` chain also exited 0: native multiview/CPU-denial/million-point recovery,
482 normal drawing/GPU/Python executions, and a rebuilt normal preview with all three
views inspected. Live instance is `draw-world-preview-1487872`, PID 1488241; built/live
SHA256 is `202a985edb4098460a5051c9bfee8ee2a3f82d7ce3f20e218fc569d0359da1ad`.
The
[extraction audit](../plans/research/phase4-extraction-audit.md) identifies remaining
generic extraction scratch to charge. Phase 4 and later releases remain open.

## Receipt quiescence and shared uploads — preceding checkpoint

The historical 232-byte scale discrepancy is resolved by explicitly draining the
actual completion-receipt limiter, not by dropping the empty observer or subtracting
estimated bytes. Both native baselines are exactly 71,013,419 CPU bytes, without
injected pressure. The million-point test passes through style-only updates, clear,
and reload; all three native views were captured.

A shared dispatcher now replaces fixed typed-then-legacy preparation. Stable dataset
groups receive rotating 64 KiB turns under the unchanged 4 MiB/frame allowance.
Charged scratch/state, pruning, work-conserving credit reuse, atomic uniform deferral,
zero-credit readiness and expanded staging ownership are covered by the 117-case
feature CPU suite. In the native contention test, a small typed cloud and compact
legacy drawing finish in frame 232; the 786,432-point cloud runs from frame 232 through
241. Both view receipts, four image assertions and exact 71,011,905-byte cleanup pass.
The four contention/clear captures were inspected. Continuous producer churn is a
CPU-model test, not yet a repeated native stress result.

Evidence: [quiescence](../plans/research/phase4-receipt-quiescence-checkpoint.md),
[shared uploads](../plans/research/phase4-upload-scheduling-checkpoint.md), and
`target/draw-vfx/cloud-fair-uploads/native.log`. Current-source multiview recovery,
typed/legacy CPU pressure, auxiliary frost, legacy lifecycle/moving-frame checks and
480 normal drawing/GPU/Python executions pass (`verify.log`, exit 0). The normal
million-point preview is rebuilt and open as `draw-world-preview-1223032`, PID 1223460;
built/live SHA256 is `d5842b3fed5d94a103da31d3b641ca22b5b95972b93cb2dd22f38e231de12aa4`.
All three `views-final/` captures were inspected. General starvation freedom is still
open: the [recreation-order audit](../plans/research/phase4-upload-recreation-audit.md)
identifies a next required regression/cursor correction, and continuous native streaming
stress remains unproved. The startup slab-allocator diagnostic, full ownership audit,
Phase 4 and later phases/releases remain open.

## Inline diagnostics and compact legacy admission — 2026-11-11 (previous checkpoint)

Valid renderer diagnostics now use bounded inline UTF-8 storage and borrowed registry
keys. Actual allocator-counting tests cover full-budget reporting, reads, replacement,
pruning, and clearing; owned `DataFailure` response payloads remain unchanged. The
focused API gate passes 67 tests; the feature cloud suite passes 109 CPU cases with
five GPU cases explicitly excluded. Typed contract/JSON/wire gates also pass.

Compact legacy allocation failure no longer falls back to a larger generic mesh or
advances visual revision/placement early. Five actual synchronizer tests verify
preservation, automatic retry, clear under pressure, node admission, and normal
unsupported-style fallback. Ten native image assertions pass in both 4x and 1x views;
all captures were inspected. Retried color changes leave position uploads unchanged.
Typed and legacy deletion restore the exact **71,011,905-byte** CPU baseline while
the unchanged **1,073,741,824-byte** cap is still fully committed by test pressure.
The inline report book reduces the static baseline versus the previous checkpoint.
Full typed multiview recovery passes its twenty image assertions and exact baseline.

Evidence: [diagnostics](../plans/research/phase4-diagnostics-decisions.md),
[legacy admission](../plans/research/phase4-legacy-admission-audit.md), and
`target/draw-vfx/cloud-legacy-admission/verify.log`. The first premature legacy capture
failed and is retained; accepted captures wait for queued pages in both views.
The broad drawing/GPU/Python recipes pass 471 executions, and final probe-only
readiness adjustments pass their feature/native and auxiliary checks. The million-point
run verifies buffer size and style-only updates but fails exact clear equality by
232 fewer CPU metadata bytes; reload was not reached. See the unresolved
[baseline investigation](../plans/research/phase4-scale-baseline-investigation.md).
Framework/temporary CPU accounting, shared upload fairness, later phases, and both
releases remain open.
The known startup slab-allocator diagnostic and secondary terrain differences remain.

The normal preview was restored after the failed scale gate: `draw-world-preview-956395`,
PID 957233, with built/live SHA256
`99e4e320acffbe6f19878856fdc504b57db43951dd99b22abedf20d6ed84119d`.
Near/grazing/overview captures in `cloud-legacy-admission/views-final/` were individually
inspected; source colors, glass, and the approved structural styling are preserved.

## Retained CPU cleanup — 2026-11-11 (previous checkpoint)

Shared snapshots, worker caches and per-view selections now prune retired owners
without requiring replacement allocation. Preparation also cancels on retirement
when no newer snapshot sequence could be admitted. The strict native lifecycle
returns tracked CPU allocation exactly to its 71,265,793-byte baseline while the
1,073,741,824-byte cap remains fully committed by injected pressure. Source-page and
drawing GPU bytes are zero; a partial clear preserves the neighboring cloud in both
views. All eight image assertions pass and the captures were individually inspected.

Feature CPU tests pass 109 cases, with five GPU cases explicitly excluded from that
CPU run. A final lease-expiry failure in the first native attempt was fixed by closing
the probe writer after its final mutation, before its cleanup wait; no assertion was
weakened. Full multiview/recovery now returns to exact CPU/GPU baseline with four live
background datasets, after fixing spare cache/list capacity and empty receipt churn.
Auxiliary, legacy/frame and million-point gates pass; normal drawing/GPU/Python recipes
pass 466 executions. The normal preview `draw-world-preview-601090`, PID 601483, is
rebuilt and open after near/grazing/overview inspection. Built/live SHA256 is
`f47236e38b774fb4148d19cf2f40fa082ace6aa353e9736209ba4ecd479c4185`.
See the [retained cleanup checkpoint](../plans/research/phase4-retained-cleanup-checkpoint.md).
This fixes the previous pinning, not all CPU accounting, fair uploads, Phase 4, later
representations or either release. The startup slab-allocator diagnostic remains open.

## CPU ownership and zero-headroom retirement — 2026-11-11 (previous checkpoint)

Charged bounded containers now cover render caches, extraction, scratch, phase-list
capacity and upload/retirement owners. Nine ownership regressions and the prior native
compatibility chain passed. A shared per-incarnation marker now makes deletion
authoritative even when a replacement snapshot cannot be admitted. Native partial clear,
neighbor survival, delete, release and recreation pass in both 4x/1x views, including
eight image assertions. See the [CPU checkpoint](../plans/research/phase4-cpu-checkpoint.md).

This is not complete CPU-pressure cleanup: the tiny native fixture still pins 13,257
CPU bytes while zero-headroom pressure remains active, returning exactly to baseline
after release. Final-source API/native compatibility and 453 normal drawing/GPU/Python
executions pass. The rebuilt normal preview `draw-world-preview-184720`, PID 186659,
is open with matching live/build SHA256
`c7dba6c4965451c1dc1004cc1772cad77a9e605804ba52470dfeab2b4f653762`;
near/grazing/overview captures were inspected. The known startup Bevy slab-allocator
use-after-free diagnostic remains unresolved. Full CPU ownership, upload fairness,
remaining Phase 4 acceptance and all later representations/releases stay open.

## Auxiliary GPU ownership — 2026-11-11 (previous checkpoint)

World drawing frost textures and deferred retirement now share the point-buffer GPU
cap. Both views preserve old complete frames under auxiliary quota rejection, recover
without geometry reuploads, and release their targets after blur disable/clear. The
final-source native fixture passes all twelve image assertions, including actual
secondary-target resizing. The unused compact GPU quad is removed. See the
[auxiliary checkpoint](../plans/research/phase4-auxiliary-checkpoint.md).

The corrected image classifier, fresh legacy lifecycle/frame and million-point gates,
21 native legacy comparisons and 443 normal drawing/cloud/frost checks pass. The normal
preview was rebuilt as `draw-world-preview-3726271`, PID 3726810; all three fresh camera
views were inspected and it remains open. Built/live SHA256 is
`630b24720dc0f92cc6afb676ed42fee3db9031e1c9ff97eb3f53621437685830`.
This verifies the narrow common-frost checklist item. CPU cache/scratch ownership,
upload fairness, complete Phase 4 and Phases 5–13/Release A+B remain open. Older live
preview details below are historical.

## Compact legacy integration — 2026-11-11 (previous checkpoint)

Fresh native tests now cover compact legacy mutations, generic fallback/restoration,
PATCH/EXTEND, rejected updates, scoped clear, DELETE and paused/advancing TTL. Actual
GPU/charged CPU usage returns to baseline. Real robot movement is verified in main
4x and secondary 1x views without re-uploading position or attribute records. Both
optimized and full sensor cameras exclude opaque and frosted legacy point clouds
with visible viewer-positive controls. Representative native captures were inspected.

Source review then found transparent sorting used entity origins rather than actual
point bounds. The correction and its generic-mesh-derived CPU test pass; all 21 native
comparisons pass, including overlapping clouds with misleading entity origins. The
original 20 cases remain pixel-exact; the overlap case's mean channel error is 0.000065.
Final normal drawing/cloud/frost regression passed 442 executions. This closes the
narrow legacy adapter checklist item, not Phase 4 or either release. See the
[integration checkpoint](../plans/research/phase4-legacy-integration-checkpoint.md).

The normal million-point preview was rebuilt and restarted as
`draw-world-preview-3158951`, PID 3159386. Its exact receipt acknowledges all million
samples; fresh near/grazing/overview captures were inspected, and it remains open.
Built/live SHA256 is `db62331a0cb459a5d09fc6623fe46a742bf64154323353fb5f1a3cc2da6a6aa7`.
Older preview IDs below are historical.

## Compact legacy points — 2026-11-11 (previous checkpoint)

PLAIN, HAIRLINE, ORB, POINT_GLOW and SPLAT can now reuse the actual legacy stroke
shader through 32-byte compact point records. Decorated, labelled and draped cases
explicitly fall back. CPU parity and twenty native compact/generic comparisons pass
with exactly matching foreground masks/RGB. Native positive controls verify the shared
flat-point correction. Normal drawing/cloud-GPU/frost-GPU regression passes 440
executions. Legacy lifecycle/frame/sensor/resource acceptance is still open; no full
adapter, Phase 4 or release acceptance is claimed.

At that checkpoint the rebuilt preview was `draw-world-preview-2738200`, PID 2738756,
with matching build/live executable hashes. See the
[legacy checkpoint](../plans/research/phase4-legacy-checkpoint.md) for evidence,
failed attempts and remaining gates. Earlier preview IDs below are historical.

## Typed-cloud frame transitions — 2026-11-11 (previous checkpoint)

The real robot moved 3.42 m while the live-link cloud followed and world/odom/captured
clouds remained fixed. Snapshot restyling, local-pose changes and explicit capture
transform replacement preserve point records; independent world-reference captures
agree within 0.023 pixels. This native gate found and fixed redundant attribute uploads
caused by metadata changes. Three worker regressions preserve clipping, source-axis
color updates and acquisition-offset overflow validation.

Two Rust fixture tests, six image-oracle tests, 78 feature-enabled CPU cases and the
native frame gate pass. Final normal drawing/cloud-GPU/frost-GPU gates pass **432
executions**, followed by a normal rebuild, preview restart and inspected near/grazing/
overview captures. Preview `draw-world-preview-2401081`, PID 2401483, remains open on
the million-point survey; built/live hash is
`9175fbc9a30930e34056c0d41e745bc48752732a73255ea114f49e73f6194f24`.
Native acquisition metadata is not claimed: the fixture supplies explicit synthetic
capture poses. The known startup slab diagnostic remains unresolved. See
[frame checkpoint](../plans/research/phase4-frame-checkpoint.md).

## Typed-cloud sensor isolation — 2026-11-11 (previous checkpoint)

The extended native sensor recipe verifies the typed backend independently of legacy
drawings in optimized and full robot cameras. Both opaque and translucent/frosted
cloud revisions have exact Drawn receipts and visible viewer pixels; all real sensor
outputs remain clean. Scoped clear removes the viewer pixels too. Two Rust fixture
tests, five image-oracle tests and both native modes pass; representative actual
viewer/camera images were inspected. Production renderer code and executable are
unchanged, and the million-point preview remains open. This closes the native cloud
sensor isolation gap, not all frame transitions or Phase 4. See
[sensor checkpoint](../plans/research/phase4-sensor-checkpoint.md).

## Integrated million-point cloud — 2026-11-11 (previous checkpoint)

The new `cloud-scale` scene renders 1,000,000 real typed samples in 16 pages through
the public writer and production draw path. Actual GPU Buffer sizes are **16 MB
positions + 32 MB attributes + 8,512 uniform bytes**. Completed frames acknowledge all
million source/selected samples. Style-only edits upload only the 8,512 uniform bytes;
clear returns exactly to baseline and reload gets a new incarnation.

Peak upload is the configured 4 MiB/frame; charged GPU usage peaks at 60,591,216 bytes
in the successful run, including pending staging/retirement. This is not physical VRAM
or controlled performance acceptance. Native overview/near/grazing captures preserve
source colors and the established world-space visual language. The display is a
synthetic point-cloud heightfield, not the later elevation-map backend.

Three fixture tests and the integrated native gate pass. Final normal regression
records **551 passing executions**; the separate multiview gate passes 20 image checks
and 74 feature-enabled CPU cases. Normal preview `draw-world-preview-1875559`, PID
1876111, remains open on the million-point survey; overview/near/grazing captures were
inspected. Its Drawn receipt records incarnation 10/gen 1, view (1,1), frame 7685 and
all million samples. Built/live SHA256:
`dbf77412a8b357aac95ec1040835afbf48bc197c51478b6b6188e9cf1289b31d`.
Remaining Phase 4 and all later phase/release requirements stay open. The known startup
slab allocator diagnostic remains unresolved.
See [scale checkpoint](../plans/research/phase4-scale-checkpoint.md).

## GPU admission and recovery — 2026-11-11 (previous checkpoint)

GPU allocation and upload staging failures now report the exact retained revision and
Rendering phase rather than remaining silently pending. Per-frame page outcomes are
charged and reduced per dataset; only complete preparation clears an admission failure.
Ordinary upload throttling does not create a failure. Device buffer and binding limits
are checked before allocation. Old complete generations remain selected while retrying.

The extended native harness exercises actual drawing-budget denial, not just pipeline
readiness injection. Both cameras keep completing old generations while allocation or
staging is denied, then automatically show the retained replacement after quota release.
All 20 native GPU image checks and 74 feature-enabled CPU tests pass. Cleanup returns
owned drawing memory to baseline. Final normal-build gates pass **542 executions**
(422 drawing, five cloud GPU, two frost GPU, 89 visibility and 24 live Python), plus
native resource/receipt probing and inspected overview/near/grazing captures.
Normal preview **draw-world-preview-1023563**, PID **1024919**, remains playing in
overview. Built/live SHA256:
`a5869c267fe10dba8b8a6e55650d34553c1c8d7731555dbbfa3778493f14bd7a`.
Known startup slab diagnostics and perspective label overlap/cropping remain.
This is not physical VRAM exhaustion or device-loss recovery; full Phase 4 and later
phase/release gates remain open. See [admission checkpoint](../plans/research/phase4-admission-checkpoint.md).

## Native multiview generation proof — 2026-11-11 (previous checkpoint)

An opt-in `draw-probes` feature drives the production queue through the ordinary
interactive camera and a second offscreen camera at 4x/1x MSAA. Delaying one replacement
page only in the second camera leaves both old pages drawing there while the primary
advances. Observations require fresh submitted-GPU-work completion, not historical
receipts. Public waits/status also verify exact history, supersession and view identity.

Sixty-seven feature-enabled CPU cases pass. Native delayed/released replacements,
pause/resume, view destruction/recreation and clear/recreation while behind pass.
All **12 actual GPU image readbacks** contain two complete expected-generation discs;
no mixed-generation colored pixels were found. Captures were also inspected. Cleanup
restores **73,725,907 CPU / 1,537,600 GPU** owned data bytes and the four ordinary clouds.

Probe-only image registration, readiness injection and observations are excluded from
normal builds. This is not two operating-system windows, actual device loss/quota
denial, multiple LOD cuts, octrees or performance acceptance. Those required gates
remain open. Details: [multiview checkpoint](../plans/research/phase4-multiview-checkpoint.md).
Final feature-disabled gates pass **446 executions** (415 drawing, five cloud GPU,
two frost GPU and 24 live Python), plus native captures and resource/receipt probing.
Normal preview **draw-world-preview-548793**, PID **549156**, is left playing in overview;
built/live SHA256 is `1436f48aceb2cbc815c6d07a09b2ffa40033bdb0649b90da158b51fc957e972b`.
The known startup slab diagnostic and label perspective overlap/cropping remain.

## Per-view complete generations — 2026-11-11 (previous checkpoint)

Removed the global dataset-ready bit and main-world active-generation map. Each
registered camera independently selects a replacement only when all nonempty pages
and their required pipelines are ready. Receipts now bind directly to that camera's
selected dataset. Charged holds for the last extracted offer and current selections
prevent the main thread from deleting a candidate that the render thread can still select.

Sixty-six CPU tests, five production GPU tests and 89 API/client visibility cases pass.
Rebuilt native overview/near/grazing captures were inspected; mixed-alpha replacement,
empty/clip/history and deferred-release probes pass. Clear restores **73,725,907 CPU /
1,537,600 GPU** owned bytes. The known startup slab diagnostic and perspective label
overlap/cropping remain; no shader/validation/panic/OOM was reported in these gates.

At that checkpoint, preview **draw-world-preview-4054757**, PID **4055288**, was open in overview.
Built/live SHA256: `5d0e6b6342089f12248b2d386de1a290e08509cb56d54d86365ce248170427ff`.
The native two-camera delayed-switch/recovery gate was **not yet proved** there. That checkpoint
does not establish LOD, octrees, integrated scale, Phase 4 completion or either release.
See [per-view checkpoint](../plans/research/phase4-frontier-checkpoint.md).
Final native/broad gates total **535 successful executions with overlapping coverage**:
415 drawing, 24 live Python, 89 visibility, five cloud GPU and two frost GPU. The earlier
66 focused CPU cases are not added again. Those gates preceded the probe-only additions above.

## Cloud depth, alpha and glow — 2026-11-11 (previous checkpoint)

Opaque samples now retain depth writes inside mixed-opacity pages; fractional samples
use the existing depth-tested transmission pass. MSAA=1 draws smooth coverage edges
separately without applying frost to opaque edges. Pass-aware receipts wait for the full
page. Alpha restyling shares point buffers, and glow preserves its finite API domain.

Focused tests pass 59 CPU cases and five production GPU tests, including 28 HDR raster
cases at 1x/4x samples. Final rebuilt/native/regression gates pass **439 executions**:
24 live Python, 408 drawing, five cloud GPU and two frost GPU. Native mixed-alpha
repair/history/clip/clear probing passes, restoring **73,725,619 CPU / 1,537,600 GPU**
owned bytes. The original four-cloud scene has fresh inspected captures, not a cosmetic
redesign. Perspective label overlap and the known startup slab diagnostic remain.

At that checkpoint, preview **draw-world-preview-3709842**, PID **3710348**, was open in overview.
Built/live SHA256: `b9d210621b75319fce9cba8bef47c89033574a1bb569fd50f6c9d6c6f734df50`.
Details and scoped limits: [alpha checkpoint](../plans/research/phase4-alpha-checkpoint.md).
Remaining Phase 4 and all later Release A+B work are still required.

## Integer-anchored age precision — 2026-11-11 (previous checkpoint)

Cloud age display preserves narrow windows at large clock origins without f32 timestamp
cancellation. Finite per-page normalization and exact integer indexed-palette thresholds
handle tiny/equal/extreme ranges, future samples and half ties. Point records remain
48 bytes; page/style uniforms are separately charged at 208 / 5,184 bytes. Age range,
clock and palette changes do not rebuild point buffers.

Fifty-four CPU cases and four production GPU tests pass, including **20,403 age-color
comparisons** across nine maps. The native four-case precision review has **12,288 points**
in 128 payload pages and four empty reference scans. All initial exact receipts completed
at frame **5956**. Inspected overview/near/grazing captures preserve gradients, cycles,
distinct zero/future rows and existing world-space energy frames. Close/grazing overlap
remains a catalogue issue; these are numerical probes, not final layout acceptance.

Live Python passes **24 cases**, including drawn/empty/culled/history receipts. The
native failure/history/clip/clear probe returns to **72,861,181 CPU / 637,184 GPU** owned
bytes, with no incomplete/staged/pinned-retired bytes. The first preview attempt used
the wrong retention mode; its failure is preserved and the full scan-set fixture is fixed.

Owned preview **draw-world-preview-3524702**, PID **3525264**, remains open in overview.
Built/live SHA256: `3e2152f2d295e5a33302578909d3ac9d427b054880412fedd90b17a807064272`.
The known startup slab diagnostic remains; no shader/validation/panic/OOM was found.
Final broad/native gates pass **589 executions** with repeated coverage; the
[age checkpoint](../plans/research/phase4-age-checkpoint.md) records the exact recipes and evidence.
This does not establish whole-phase, performance, integrated scale or release acceptance.

## Bounded Python visibility waiting — 2026-11-11 (previous checkpoint)

`DataConnection.wait_visible` now uses the existing native queue, shared exact Rust
receipt validation, frozen view identity, bounded scratch, cancellation and one deadline.
It releases the GIL and returns complete exact-u64 receipts or structured terminal errors.
No separate Python polling/requester or writer lease is created; waiting never deletes data.

Focused gates pass **105 Rust and 24 Python cases**. A fresh native build passes **24
live Python cases**, **395 legacy drawing executions**, **two GPU frost tests** and the
renderer status/resource probe. Eight writer regressions also pass: **558 successful
executions** with repeated coverage.
Python proves drawn, empty, clipped and historical receipts for incarnation 8 at frames
2806 / 2811 / 2817. The native probe returns to its initial **72,758,297 CPU / 635,200
GPU** owned-byte baseline after clearing its dataset; no incomplete/staged/pinned data remain.

Owned preview **draw-world-preview-3236330**, PID **3236680**, is open in overview.
Built/live SHA256: `4c4223db1280f6c5a37144da9d720e301900f2ef907993e30ea79d548eff7a33`.
Near/grazing/overview captures were inspected. Existing perspective overlap and the
known startup slab diagnostic remain; no shader/validation/panic/OOM was found.
See the [Python wait checkpoint](../plans/research/phase4-python-wait-checkpoint.md).
This is not integrated scale, whole-phase, performance or release acceptance.

## Source-clock isolation — 2026-11-11 (previous checkpoint)

Source-age colors now use a charged generation-local index keyed by clock name and
scene epoch, rather than one maximum across unrelated sensors. Source-retention
anchors keep their authoritative history; simulation-retention anchors do not become
source clocks. Six new cases cover exact u64 domains, 4,096-scan capacity, budget
failure/release, style reuse, empty replacement, late input and policy reset.

The focused/native run passes **46 CPU tests, five preview fixtures and three actual
GPU probes**. Inspected near/grazing/overview captures show matching rows from different
clock origins in all four maps. The rebuilt **draw-world-preview-2826628**, PID
**2827114**, remains open. Native clear restores 72,753,929 CPU and 635,200 GPU owned
bytes for the 128-page scene after eliminating unused simulation-age uniform updates.
Broad regression passed 655 executions before that narrow final fix; the final binary
passes 13 live Python cases and native probing, followed by 402 legacy/frost/fixture
rechecks and another native probe at the same baseline. Final-source gates total 464
successful executions with overlapping coverage. See the
[clock checkpoint](../plans/research/phase4-clock-checkpoint.md).
No integrated scale, whole-phase or release acceptance is claimed.

## Scientific colors — 2026-11-11 (previous checkpoint)

Indexed COSTMAP/MAP/RAW/OKABE no longer fall back to grey ramps. All nine maps retain
the 48-byte base record and share prepared point buffers across palette changes.
Exact u64 indexing, declared normalization, missing data and binary/fractional alpha
classification have CPU coverage. Production WGSL color tests pass **20,973 comparisons**
and **1,035 integer-time cases**. Integer-first time conversion fixes a detected
age half-second palette boundary; extreme ranges and mixed-clock age remain open.

The rebuilt palette comparison shows four raw-value band datasets (3,072 points each),
with the existing world-space labels and energized frames. Near/grazing/overview
captures and the public failure/history/empty/clipping/clear probe pass. Current preview:
**draw-world-preview-2592057**, PID **2592702**, left playing in overview. Built/live
SHA256: `2d6c78e52518802123d9bdf06386625c348caaf3fcdeca3a477d1ad77f1154f6`.
Evidence and rejected attempts: `target/draw-vfx/cloud-color/`; see the
[color checkpoint](../plans/research/phase4-color-checkpoint.md).
Focused, broad and live Python gates total **694 successful executions**, with repeated
coverage across recipes. The final native probe returns to the same owned-resource
baseline. This is not Phase 4 or release acceptance.

## Source-frame clipping — 2026-11-11 (previous checkpoint)

Optional presentation clip boxes now select sample centers after scan/page origins
and scan pose, before external placement. Clip-only patches share geometry/attribute
buffers. CPU and production WGSL selection match in 774 actual GPU comparisons,
including large origins, inclusive bounds and general three-axis rotation.

The combined successful gates record **680 test executions** (including repeated
coverage): 269 focused/API/visibility/fixture checks, 385 drawing/GPU/frost checks and
26 corrected Python checks across fake/native hosts. Earlier failed fixture expectations
remain preserved. The final native probe selects 1/2, culls 0/2 and restores 2/2 at
frames **34182 / 34188 / 34195**, returning owned CPU/GPU allocations to baseline.

Preview **draw-world-preview-2233712**, PID **2234113**, remains open in overview.
Built/live SHA256 is `036035f9f43881ebc48a8c2a87708e2712810fd7742c2e0c28a2341c49a57733`.
Inspected near/grazing/overview captures show reference, half-space, fully clipped and
frosted height-slice cases selecting **7,896 / 3,983 / 0 / 1,768** of 7,896 samples each.
The known slab-allocator startup diagnostic and perspective label overlap remain.
Evidence: `target/draw-vfx/cloud-clip/`; details in the
[clipping checkpoint](../plans/research/phase4-clipping-checkpoint.md).

Phase 4 and Phases 5–13 remain open. Next are multiview and device recovery,
legacy adaptation, native frame/sensor and integrated
scale/performance gates. General interactive clipping planes belong to
Phase 8; box clipping does not complete inspection or either release.

## Exact native visibility — 2026-11-11 (previous checkpoint)

Compact clouds now publish exact dataset/view/frame receipts after distinct page draws,
the view pass and submitted GPU work complete. Bounded history preserves previous
success without inheriting newer generations; empty data still requires its view pass.
The Rust wait helper polls STATUS only, freezes the first resolved primary identity,
and returns explicit terminal states. Main-world lifetime tracking covers Gearbox's
actual embedded viewport as well as native window cameras, without admitting sensors.

**637 checks pass**, including signed local client timeout/reuse, 26 CPU cloud tests,
resource/status/transaction regressions, typed contracts, drawing and actual GPU gates.
The rebuilt native probe confirms drawn generation 2 at frame 494, empty generation 3
at frame 498, historical generation 2 still at frame 494, explicit supersession for
never-displayed generation 1, and owned CPU/GPU baseline recovery after clear.

Preview `draw-world-preview-2077354`, PID 2077727, is left playing in overview;
built/live SHA256 is `268c9b220775a40e4e40346fd5c939cbba56d9eec93f6737f620284f9a532dc4`.
Fresh near/grazing/overview captures were inspected. Caller/scientific colors and
approved annotations remain; perspective label overlap is not final visual acceptance.
Evidence and rejected intermediate attempts are in `target/draw-vfx/cloud-visibility/`.
See [visibility checkpoint](../plans/research/phase4-visibility-checkpoint.md) and
[API semantics](DRAW_DATA_PROTOCOL.md#exact-native-cloud-visibility).

This is not Phase 4 completion. Native multi-view frontier preservation, stalled-upload
and device-failure acceptance, clipping, remaining scientific color/age cases, legacy
adaptation, Python wait parity and integrated scale/performance still remain. All
Phases 5–13 and both releases remain required. The known slab-allocator error is unfixed.

## Renderer resource and failure status — 2026-11-11 (previous checkpoint)

The renderer now shares its GPU allocation ledger with the public status API, including
queued staging and deferred retirement. Exact-revision preparation failures are visible
without changing a successful retained commit. A bounded worker retry recovers CPU
budget/allocation failures without another commit and is cancelled by replacement or clear.

Seven API/resource cases, 52 transaction cases, 149 typed contract checks, 370 drawing
checks, three GPU cases and the typed preview fixture pass: **582 checks**. Native
probing has also demonstrated a retained precision failure, successful replacement,
and GPU/CPU allocation return to baseline after clearing only the probe namespace.
The retry-enabled native rebuild, guarded probe and inspected near/grazing/overview
captures pass. Preview `draw-world-preview-1785478`, PID 1786020, remains playing;
built/live SHA256 is `962eb2830d262043ab41b3014d95a45177f17f223b3e45b44282609b284749ba`.
The probe verifies return to CPU 39,122,818 and GPU 1,537,344 owned bytes. An initial
guarded attempt cleared the replacement before activation and is explicitly rejected;
the corrected probe waits for exact activation, without claiming a completed-frame
receipt. See the [resource/status checkpoint](../plans/research/phase4-status-checkpoint.md).

This does not add completed-view/frame receipts or rendering capabilities. Integrated
GPU pressure, million-point native rendering, clipping, color/age edge cases, legacy
adaptation and the remaining phases are still required.

## Typed cloud runtime — 2026-11-11 (previous checkpoint)

The worker, incremental compact GPU uploads, procedural opaque/transmitting passes,
shared frost and world-frame placement are connected. The public typed writer sends
four separated raw/intensity/category/RGB datasets, each with 7,896 points. The
annotations use the approved legacy drawing language; the point data do not use a
legacy POINTS/SPLAT substitute.

The native overview, close-up and grazing captures were inspected. Preview
`draw-world-preview-1489443`, PID 1490039, is left playing. Built/live viewer SHA256:
`5eb499466bf162800884b1a096b83548cb076562e1c59b2451f5ed6bfb991e60`.
The known slab-allocator startup diagnostic remains. The first attempt's procedural
pipeline-cache panic is preserved as **not accepted**; the corrected native rerun
has no new shader/validation/panic/OOM diagnostic.

`test-draw-cloud` passes eighteen CPU preparation/worker cases. The final checkpoint
has **520 passing checks**: 149 typed contracts, 367 drawing regressions, one actual
GPU layout probe, two GPU frost cases and one typed-preview fixture. The three GPU
exclusions in the CPU drawing recipe pass separately. `regression.log` contains the
completed contract suite but was interrupted with exit 143 during later compilation;
`regression-final.log` completes the drawing/GPU/example gates. These logs are under
`target/draw-vfx/cloud-runtime/`. Full details and remaining work are in the
[runtime checkpoint](../plans/research/phase4-runtime-checkpoint.md).

The API still explicitly lacks exact rendered-view/frame acknowledgement. This is
not Phase 4 completion, integrated million-point acceptance, LOD, occupancy rendering,
sensor integration or Release A/B completion.

## Compact cloud preparation — 2026-11-11 (previous checkpoint)

The first Phase 4 implementation separates geometry from selected attributes:
16-byte position/source-ordinal records plus 32-byte attribute records. f64 page offsets
are rebased before conversion, with explicit finite/one-millimetre local error rejection.
Validity compaction preserves exact source IDs and original measurements in pinned pages.
Preparation is charged to the backend CPU budget and shares geometry across color changes.

Thirteen CPU tests cover all scalar types, normalization, nullable colors/categories/
radii, exact IDs, source-pose axis/range mapping, precision and ownership/budget failures.
The actual GPU probe verifies Rust/WGSL offsets and **48,000,000 allocated base bytes for
one million records**, with result/readback/transient probe writes separately reported.
Exact integer-limb age subtraction preserves nanosecond differences near u64::MAX.

These are **preparation/layout gates only**. The records are not yet integrated into
Bevy draw passes; no new rendering, sensor exclusion, residency or visible-generation
claim is made. The preview remains the legacy Spatial Data Lab. Full regression/frost
and preview-refresh verification now pass. Evidence under `target/draw-vfx/cloud-renderer/`:

- `contracts.log`: 149 passed; `regression.log`: 362 passed, including the thirteen
  preparation tests. The three GPU-only exclusions pass separately: `gpu-layout.log`
  (one layout/age probe) and `frost.log` (two existing frost tests).
- `preview-refresh.log`, `preview/`: rebuilt, inspected and left playing as
  `draw-world-preview-1184587`, PID 1184837. Built/live SHA256 matches
  `93a6e792120f33b8ea7b43ee3ddd3253ae01dd82645b0ae5d6420042ea494cbb`.
  The known slab-allocator startup diagnostic remains; no new OOM/shader/panic appears.
- `source.sha256` records the verified source checkpoint. The Phase 3 native capacity
  run was not repeated: this step adds preparation/layout code, not service routing
  or integrated typed drawing. Its earlier evidence remains separately labeled.

Phase 4 and all later release gates remain open; see the [session record](../plans/research/implementation-phase4-session.md).

## Phase 3 completion — 2026-11-11

Resume at **Phase 4 compact point rendering**. The full Phases 4–13 and Releases A+B
remain required; no new renderer or visible-generation acknowledgement is implemented.
The [acceptance audit](../plans/research/phase3-acceptance.md) maps every Section 7
compatibility row to executable checks and records the exact limitations.

Final evidence is under `target/draw-vfx/phase3-final/`:

- `stream.log`, `contracts.log`, `regression.log`: **637 passing checks**, with two
  GPU-only frost cases excluded from the CPU drawing recipe.
- `parity.log`: **115 Rust/Python entities match**.
- `native-20261111T043246Z/`: all 27 retained compatibility/lifecycle checks, owned-scene
  reset, twelve native Python cases, the public writer and 100k/1M retained capacity
  pass against actual Bevy. The probe `draw-vfx-live-912320`, PID 912326, stopped and
  was verified gone. Queues were drained after each measured mode.
- Listing now seeks exact namespace/descendant ranges instead of scanning unrelated
  objects; each lock copies one bounded summary and JSON encoding happens off-lock.
  Concurrent mutation returns explicit `STALE_CURSOR`, even on the first page.
- Rejected publication keeps a local generation Arc until registry unlock. Its actual
  destructor regression fails with the old behavior; the restored fix passes. Cloud
  and occupancy page/generation validation are also tested with concurrent reads/clear.
- Preview `draw-world-preview-863978`, PID 864212, was rebuilt, inspected and left
  open. Built/live/probe viewer SHA256 matches
  `8fc473c68a8175eadf8946948780d01f05bcaefea347edd69e53522b0cce5e85`.
  The known slab-allocator startup diagnostic remains; no new OOM/shader/panic appears.

The first final native attempt exited 143 during build and is explicitly marked not
accepted; the separate rerun passed. Local capacity measurements are not rendering,
process RSS, controlled speedup, sensor-frequency or cross-machine acceptance. The
preview is still legacy-rendered, not a substitute for Phase 4. Sections below retain
earlier checkpoints as implementation history; this section is authoritative for status.

## Native Python sender — earlier checkpoint, 2026-11-11

The optional `gearbox-python` crate builds a worktree-only PyO3 extension. Its Python
facade uses the same signed negotiation, fixed workers, request barriers and atomic
transaction owner as Rust. Existing legacy imports and calls remain unchanged; typed
calls explicitly require the extension and never use legacy automatic retries.

Prepared sources retain immutable encoded payloads by reference and freeze mutable
contiguous byte buffers once under a separate source budget. Replace/delta/removals,
cooperative cancellation, retained receipts and structured cleanup outcomes use the
native writer. An Arc-owned writer keeps its pipeline alive safely. Explicit close,
request waits and uploads release the GIL; context cleanup preserves an active exception.
Implicit native destruction can still wait for bounded cleanup under the GIL; explicit
close is required when responsiveness and cleanup confirmation matter.

Review corrected two issues before acceptance: canonical inner payload hashes differ
from peerbus transport/layout hashes, and packing a complete Python request before queue
admission would leave one hidden copy per blocked producer. The facade now passes a
112-byte header plus borrowed sections, concatenated only after native admission.
Oversized mutable pages are rejected before freezing. Independent golden bytes remain
unchanged; concurrent tests disable full `DataRequest.pack` to catch regressions.

Current verification evidence is under `target/draw-vfx/python-sender/`:

- `borrowed-parts.log`: **16 pipeline**, **12 Python client**, and **19 wire** cases
  passed after the copy correction. Coverage includes 256 concurrent Python requests,
  completed-result quotas, cancellation during another thread's wait, GIL-free close,
  100k-point replace/delta, corrupt data preserving the old generation, occupancy,
  immutable source ownership, source limits and context error preservation.
- `final-contracts.log`: **149 passed** on final source, including the independent wire checks.
- `final-regression.log`: **349 passed**, two separate GPU-only frost tests ignored.
  The initial combined runner exited 143 before completion; its incomplete
  `complete-regression.log` is not acceptance. The standalone rerun passed.
- `final-stream.log`: **135 passed** — 85 storage, seven Rust transport, seven native
  Python transport, sixteen pipeline, eight writer and twelve Python client cases.
- `native-20261111T040203Z/native-live.log`: **all twelve Python client cases passed
  against the actual Bevy host**, followed by native 100k/1M retained capacity, raw CLI
  capabilities and legacy Rust/Python/CLI lifecycle/updates. The Rust public writer also
  passed 100k-point replace/delta/cleanup. No queued/running/outstanding requests remained
  after the measured modes. This is retention, not compact rendering, process RSS,
  controlled speedup, sensor-frequency or cross-machine acceptance.
- The isolated probe `draw-vfx-live-598677`, PID **598683**, stopped and was verified
  gone. Its executable hash matches the preview below; extension/harness hashes are in
  `python-binary.sha256`. The native log retains the known startup slab diagnostic and
  no new OOM/shader/panic failures. No failure marker exists in the accepted directory.
- `parity.log`: **115 Rust/Python entities match** (10 world, 24 composite, 81 helper).
  The final CPU/native-service regression inventory is **633 passing checks**, plus
  the separate native Bevy and entity-parity gates; two GPU-only tests remain ignored.

The owned preview was rebuilt and inspected, left playing as
`draw-world-preview-580846`, PID **581105**. Release/live SHA256 matched
`db5f2a88401cd166ab0432aaf8832a82daefabcf8c30a7d8dea9e462a6dc171e`.
The fresh image, instance metadata and log are preserved in `python-sender/preview/`.
The known Bevy slab allocator startup diagnostic remains; no new shader/OOM/panic was
present. This remains the 52-entity, **legacy-rendered** Spatial Data Lab, not a new
compact renderer or a claim of visual progress from sender work.

Remaining Phase 3 work is explicit in `plans/research/phase3-thread-audit.md`: complete
native compatibility/adversarial cases and executable lock/concurrency evidence.
Typed convenience builders, whole-frame client policy, rendering and all later planned
representations remain required. No Release A/B or full-plan completion is claimed.

## Bounded Rust pipeline and transaction writer — 2026-11-11 (previous batch)

Implemented fixed-worker request admission with byte and operation caps, explicit
wait/reject, absolute deadlines and FIFO barriers around every non-CHUNK operation.
Unconsumed results remain charged. Queued cancellation drops payload before releasing
quota; result consumption releases the operation slot synchronously. Borrowing input
until admission avoids hiding arbitrary blocked producers' payload copies outside the
budget. Expensive metadata validation runs only on the fixed charged workers.

The public `DrawDataWriter` borrows source pages, reserves 2 MiB of sender workspace
from the same queue budget, owns its writer lease and supports replace/delta/removals.
It checks all bounded-window chunk results before COMMIT. Failures cancel queued chunks
and attempt a separate 250 ms CANCEL barrier; explicit outcomes distinguish cancellation,
already-retained data and unconfirmed cleanup. Drop closes the lease best effort within
250 ms; explicit close reports errors. Retention is not rendering/visibility.

Two actual capacity attempts exposed transport bugs rather than usable measurements:
idle SHM requesters first failed on overwritten foreign replies; the first Bevy attempt
then failed on a temporarily pinned request slot. Requesters now drain old broadcasts
and wait for pre-publication loans within their original deadlines. Neither change
retries a delivered mutation or masks post-publication response loss. The failed Bevy
directory `pipeline/native-20261111T025837Z/` retains `NOT_ACCEPTED.txt`.

Verified logs under `target/draw-vfx/pipeline/`:

- `complete-stream.log`: **119 passed** — 85 storage, seven Rust transport, seven native
  Python transport, thirteen pipeline and seven writer cases. Native pipeline coverage
  includes eight concurrent producers with 512 unique requests and a 100,000-point
  retained replacement. A deterministic pinned one-slot ring verifies timeout without
  publication and later exactly-once publication after release.
- `writer-current.log`: all **seven writer cases passed again** on final source,
  including 100k-point replace/delta, page/scan removals, occupancy upload, corruption
  preserving the old generation, cancellation, retained-after-invalid-ack, forty lease
  drop cycles and full/tight workspace budgets.
- `complete-contracts.log`: **149 passed** (133 Rust + 16 Python).
- `complete-regression.log`: **349 passed** (300 Rust + 49 Python); two GPU-only frost
  tests explicitly ignored. Existing molla warnings remain.

The dedicated Spatial Data Lab was rebuilt/restarted, its new native image inspected,
and left playing as `draw-world-preview-251801`, PID **252504**. At capture, release/live
SHA256 matched `584675b2445ae92e328c0aa5bb428feb9de732c51fdfb79384c7bcce1c91988d`.
The 52-entity, legacy-rendered lab is unchanged: three 5,852-sample colored cloud views
and two glassy occupancy views. This is not the Phase 4 compact renderer. The pre-existing
Bevy slab allocator startup diagnostic remains; no new shader/OOM/panic was present.

`native-20261111T031408Z/native-live.log` now **passes** after the slot fix: actual Bevy
retention/lifecycle/accumulation, public writer 100k-point replace/delta/cleanup, raw CLI
capabilities and legacy Rust/CLI/Python lifecycle/updates. The owned probe
`draw-vfx-live-277391`, PID **277412**, stopped and was verified gone. `parity.log`
records **115 matching Rust/Python entities**. Native and preview executable hashes match.
The native viewer also retains the known startup slab-allocator diagnostic, without
new OOM/shader/panic failures. No failure marker exists in this successful directory.

`capacity.json` and `capacity-million.json` verify exact retained source/valid counts,
page counts and generations after every upload, including warmups. Every measured mode
ends with zero queued/running/outstanding requests and clears only its own namespace.
Observed local SHM complete-upload durations (fixture generation and connection setup
excluded; no compact rendering):

| Request mode | 100k points, median of 3 | 1M points, one measured upload |
|---|---:|---:|
| Direct sequential | 0.096818 s | 0.690325 s |
| Four workers, window 1 | 0.110050 s | 0.681295 s |
| Four workers, window 4 | 0.036256 s | 0.421632 s |
| Four workers, window 8 | 0.033995 s | 0.373599 s |

Peak charged request-pipeline ownership reached **8,873,472 bytes** against the
32 MiB cap; the separate public writer probe peaked at **10,902,680 bytes**, including
its reserved workspace. These figures exclude caller-owned source pages, native SHM
mappings/runtime/thread stacks and are not process-RSS measurements. The native machine
was an i7-13700KF with an RTX 4080; other applications were active and the GPU was highly
loaded. These small observations do not establish controlled speedups, percentile
latency, sensor-frequency streaming, cross-machine throughput or renderer performance.
The earlier FakeHost run is a separate 2 ms polling service, not the native measurements.
Full Phase 3 and all Phases 4–13 remain required.

## Native request deadlines — 2026-11-11 (previous batch)

The exact pinned peerbus source is now vendored under `vendor/peerbus`, with provenance
and a graph-wide Cargo patch covering agentio as well as Gearbox. No shared cached source
or installed Python wheel was patched. New timeout/deadline methods bound the
actual SHM/QUIC requester; QUIC cancellation resets/stops its streams without spawning
an abandoned worker. Already delivered mutations can still complete after timeout.

Typed Rust negotiation and operations default to five seconds, with explicit relative
and absolute overrides. All signed directory/capability calls in a negotiation share
one absolute deadline. Unsupported/schema/host-ownership errors remain distinct from
`DrawDataError::Timeout`; wire contracts are unchanged. Native Python raw and datapod
requesters expose additive timeout methods raising `TimeoutError`. A non-default build
feature produces a worktree-only extension, not an installation into the user's runtime.

Verified logs under `target/draw-vfx/transport/`:

- `final-stream.log`: **97 passed**: 85 storage, five real Rust transport and seven native
  Python transport cases. Transport coverage includes forced QUIC with explicit peer ACLs,
  dial timeout, zero/elapsed/overflow deadlines, late-response rejection, repeated timeout
  recovery and sharing an absolute deadline. These are functional bounds, not a throughput
  or process-RSS acceptance measurement.
- `final-contracts.log`: **149 passed** (133 Rust + 16 Python), including fourteen native
  FakeHost/host cases. Silent capability service negotiation terminates within its budget;
  expired calls leave the full writer quota available. Frozen wire fixture hashes match.
- `final-regression.log`: **349 passed** (300 Rust + 49 Python), with two GPU-only frost
  tests explicitly ignored. The pre-existing molla warnings remain.
- `final-compat-rust.log` and `final-compat-python.log`: the five Rust and seven Python
  transport cases passed again with legacy-call warmups, confirming old calls still work
  before deadline/recovery operations on the same requesters.
- `native-20261111T021139Z/native-live.log` and `native-report.json`: actual Bevy retained
  cloud/occupancy, accumulation, late capture, retries/counters, reset, and all previous
  typed/legacy lifecycle checks passed with the patched transport and bounded Rust client.
  Isolated probe `draw-vfx-live-4037522`, PID **4037529**, stopped and was verified gone.
- `native-20261111T021139Z/parity.log`: **115 matching Rust/Python entities**.

Preview `draw-world-preview-4004196`, PID **4010804**, was rebuilt/restarted, inspected
from its fresh native capture, and left playing. Release/live SHA256 is
`bac9bad23ce026d0f84f9ed4c3debdc3dea3328ae1fec013e6d599c5c872ae6a`.
Capture, instance metadata, renderer logs and image hashes are preserved in the native
evidence directory. The preview remains the unchanged, explicitly legacy-rendered lab.
Both viewer logs retain the pre-existing slab allocator startup diagnostic, without new
OOM/shader/panic failures. Shared GPU load does not establish performance acceptance.

The full Phase 3 gate remains open: bounded high-level sender queues, limited pipelining,
transport capacity measurements and the complete native stress matrix are still pending.
Phases 4–13 and Release A/B acceptance remain required. No compact renderer or new visual
finish is claimed by this transport-only batch.

## Accumulation checkpoint — 2026-11-11 (previous batch)

Added bounded accumulation with whole-scan eviction by selected capture time, scan count
and payload bytes. Full candidate validation precedes selection; eviction cannot hide
invalid pages or duplicate source IDs. A scan too large to fit rejects its submission
without replacing the previous complete generation. Retained-map updates are not evicted.

Source age subtracts exact integer timestamps before conversion and preserves a charged
clock/epoch watermark anchor across empty generations and presentation updates. Simulation
age advances only with simulation time and publishes atomic shared-page generations without
refreshing object lifetime. Rejected/stale commits and retries do not repeat eviction counts.
Maintenance allocation/permit failures remain visible in status, preserve old data, and
recover when capacity returns, including while paused. Latest failure ordering is shared
between submissions and automatic maintenance. See `DRAW_DATA_PROTOCOL.md` for policy details.

Current-source verification under `target/draw-vfx/3d-data/storage/`:

- `accumulation-final-policy.log`: **85 passed** (19 page + 14 generation + 52 transaction),
  including sixteen accumulation cases and 256 generated submissions checked against an
  independent whole-scan reference model. The existing million-point transaction still passes.
- `accumulation-final-contracts.log`: **147 passed** (131 Rust + 16 Python), including
  twelve native FakeHost cases and unchanged frozen fixtures.
- `accumulation-final-regression.log`: **349 passed** (300 Rust + 49 Python), two separate
  GPU-only frost tests explicitly ignored.
- `accumulation-native-20261111T014806Z/native-live.log` and `native-report.json`: actual
  Bevy cloud/occupancy retention, whole-scan accumulation, source age, late capture, counters,
  retries, reset and all previous native checks passed. Legacy Rust/CLI/Python lifecycle and
  updates passed as well. Accumulation is now advertised; rendered dataset kinds remain empty.
- `accumulation-native-20261111T014806Z/parity.log`: **115 matching entities**.

The dedicated preview was rebuilt/restarted and left playing: `draw-world-preview-3779186`,
PID **3779671**, with matching release/live hash
`815aa7c85163815fb092e0f92b405232fdb1f525d49ce4f8f75aafacc60ba1df`.
Its fresh native capture was inspected and preserved alongside logs/hashes in the native
evidence directory. The 52-entity lab remains legacy-rendered with unchanged scientific
colors, structural accents and frosted octree surfaces; it does not demonstrate compact
typed-data rendering. Probe `draw-vfx-live-3786953`, PID 3786960, stopped and was verified gone.
Neither viewer had new OOM/shader/panic failures; both retain the pre-existing slab allocator
startup diagnostic. Shared GPU load and functional tests do not establish scale performance.

At this previous checkpoint, full Phase 3 remained incomplete: bounded high-level senders, real deadlines/backpressure,
transport measurements and the entire native adversarial matrix remain required. A read-only
audit confirmed the unpatched pinned node requester's remote path lacked an exposed cancellable
deadline; standalone timeout constants do not prove otherwise. Source pointers are in
`plans/research/client-transport-discovery.md`. No shared Cargo-cache source was edited.

## Native storage checkpoint — 2026-11-11

Both FakeHost and Bevy now use one charged `DrawingBackend` for legacy drawings and
typed datasets. An exclusive RAII service claim prevents multiplying writer/history
quotas by attaching multiple owners. Legacy mutations cannot bypass identity ownership.
The native API retains cloud and octree pages, supports atomic replace/delta,
presentation-only generations sharing existing pages, mixed bounded listing and status.
Rendering, picking, per-view completion and accumulation remain unadvertised.

Cursor tokens bind query/revision and expire on mutation, timeout or bounded-capacity
eviction. Oversized legacy summaries fail instead of truncating identity. Status progress
and failures describe the requested key; allocation and cumulative counters are host-wide.
Lifecycle tests cover simulation pause/expiry/rewind and all four scene-reset scopes.
Presentation does not restart object lifetime. Removed generations are released outside
the registry mutex, and remain charged while readers pin them. Latest acquired scan keys
cannot reuse the current scan identity merely because capture metadata is identical.

Verified current-source gates under `target/draw-vfx/3d-data/storage/`:

- `native-final-stream.log`: **69 passed** (19 page + 14 generation + 36 transaction).
- `native-final-contracts.log`: **147 passed** (131 Rust + 16 Python), including
  twelve native host tests and unchanged frozen packet hashes.
- `native-legacy-regression.log`: **349 passed** (300 Rust + 49 Python), including
  eight real Bevy App scheduling/reset tests; two GPU-only frost tests explicitly ignored.
- `native-retention-20261111T012623Z/native-live.log`: real Bevy retained cloud/occupancy
  probe plus legacy Rust/CLI/Python lifecycle and updates passed. The raw CLI capability
  reply agrees with the Rust connection. The isolated instance `draw-vfx-live-3656217`,
  PID 3656223, stopped after verification.
- `native-retention-20261111T012623Z/native-report.json`: received/retained separation,
  reordered chunks, exact retry, ownership conflicts, cursor replay/invalidation, shared
  presentation, stale/shared deltas, delete/recreate, occupancy, typed deletion of legacy
  data, terminal invalid-flags/CRC/overlap failures, wrong-route/epoch isolation and reset.
- `native-retention-20261111T012623Z/parity.log`: **115 matching entities**.

The dedicated preview was rebuilt/restarted, left playing, and its fresh screenshot
inspected: `draw-world-preview-3654285`, PID **3654648**. Release and live image SHA-256:
`b63ba9c99c42a621ee070916ebd7ab7d5bdf94641d339c7e4fc5257cf67972ab`.
Captured evidence and hashes are in `native-retention-20261111T012623Z/`; the launch
record is `native-preview.log`. The unchanged 52-entity lab still renders through the
legacy drawing path. Three scientific cloud palettes and two glassy octree views remain
visible. It is not a typed-renderer preview or a new visual acceptance claim.

Neither fresh viewer reported OOM, shader-validation or panic failures. Both retain the
pre-existing Bevy slab-allocator startup `Use-after-free` diagnostic. Other applications
shared the GPU, so there is no controlled FPS/performance acceptance. This is partial
Phase 3 progress: accumulation eviction, bounded senders, full native stress/compatibility
coverage and transport measurements remain open, as do all later plan phases.

## Baseline evidence

Older checkpoint sections below record evidence at their own source snapshots;
the native storage checkpoint supersedes their statements that runtime retention is unavailable.

`target/draw-vfx/3d-data/baseline/20261110T205049Z/`:

- `test-draw`: 338 passed; separate GPU tests not counted here.
- `test-draw-parity`: 115 matching Rust/Python entities.
- `test-draw-frost`: two GPU tests passed.
- `test-draw-live`: retained lifecycle and Python/CLI updates passed.
- `test-draw-sensors`: optimized/full camera isolation passed; zero magenta leakage,
  mean channel deltas 0.3948697917 and 0.1030078125, respectively.
- `native/`: full-window dense cloud, spatial, scalar/categorical grid, shared blur,
  and three panel-finish captures inspected. Labels, caller palettes, restrained
  structural energy, glass panels, and 0.45 m TRON walls remain visible.
- Native input regression passed. No OOM, shader validation, or missing-attribute
  errors; the pre-existing startup slab-allocator diagnostic is still present.
- `voxels/`: native plain scientific point/voxel/box reference inspected. This capture
  uses the expiry-fixed binary, with unchanged drawing geometry/materials.
- Rust 1.96.1, Bevy 0.19, RTX 4080/NVIDIA 595.112.07. Actual native windows were
  2468x1348, despite requesting 1600x1000; dimensions were read from PNG headers.
  MSAA source default is four samples (`vendor/bevy_render/src/view/mod.rs`), but
  runtime MSAA/presentation-cap telemetry is not exported by the baseline runner.
  Native FPS overlays are not controlled performance data.
- The frozen future timing profile explicitly requires verified target size, MSAA,
  and uncapped timing; missing runtime telemetry will fail Phase 9 setup.

This is a baseline, not new-data acceptance. The 20k cloud and unrelated running
simulator do not establish million-point timing. The blur scene has no source robot
loaded, so its ghost slots are not evidence of robot rendering.

## Fixed pending upload scheduling

**What:** Plugin ticks now expire incomplete uploads even with no expiring live entity.
**Why:** The old live-lifetime guard prevented the store's pending expiry/rewind cleanup.
**Where:** `crates/gearbox-api/src/plugin/draw.rs`, `src/draw.rs`, `.make.lua`.
**Learned:** Store expiry tests alone miss a scheduler guard; exercise a real Bevy App
with the plugin, simulated pause/advance/rewind, reset, and late chunks.

Red proof: `target/draw-vfx/3d-data/expiry/red.log` — 3 failed, 4 passed with the old
guard. Green proof: `green.log` — 7 passed. `test-draw.log` — 345 focused tests passed;
`test-draw-live.log` — native retained lifecycle and Python/CLI update checks passed.
The full recipe now includes the plugin regression tests.

The new `DrawStore::pending_len()` is count-only diagnostics. The fix preserves the
legacy 30-second simulation-clock timeout; it does not add the new protocol's wall-time
lease, aggregate quotas, or bounded client queues.

## Next steps

1. Complete Phase 3 accumulation eviction, bounded high-level Rust/Python/CLI
   submission, the full native stress/compatibility matrix and measured backpressure.
   Current CPU/envelope/service contracts are in `docs/DRAW_DATA_PROTOCOL.md`.
2. Keep the full Release A/B scope active. No new-data renderer, sensor adapter,
   scale result, or new-representation user acceptance exists yet.

## Bounded page substrate

`src/draw_data/{memory,page}.rs` now owns real packed cloud/occupancy allocations.
Before allocating, a page reserves payload, fixed byte-coverage/chunk-start bitmaps,
conservative struct metadata and peak validation scratch from one shared CPU ledger.
Reservation-to-allocation transfer does not double count. Dropping failed/cancelled
uploads releases their resources. Immutable Arc pages retain one charge until the
last reader releases them. Accounting excludes allocator internals and process RSS.

The borrowed validators in `wire/draw_data/pages.rs` share layout/value rules with
the owned decoders. Cloud source IDs use bounded caller-owned sort scratch; occupancy
keys use sorted Morton prefixes, including signed tiles and depth 21. Packed payloads
are not copied into decoded retained columns or per-leaf objects. Original source
order is preserved. Completed staging retains chunk boundaries for exact retries;
subranges, concatenations, partial overlaps and conflicting bytes remain errors.

`test-draw-stream` currently covers this substrate, not the complete Phase 3 gate:
**19 passed**, including 16 simultaneous budget contenders, all three-chunk arrival
orders, one-byte fragmentation, staged retries, invalid CRC/kind/count/semantics,
65,536-point storage, empty pages, frozen all-scalar layouts, signed extreme octree
tiles, corruption parity, shared pins, and actual stored-page publication through
the generic identity facade. The latter is not a host/client retained-dataset test.
`test-draw-data` passed again: **146 checks**, including the unchanged frozen wire
fixtures and 11 native service tests. Evidence: `target/draw-vfx/3d-data/storage/contracts.log`.

At that page-only checkpoint, live delta assembly, quotas, leases, history, policies,
service integration and bounded senders remained required. The transaction-owner
checkpoint below supersedes its kernel status. No runtime storage/rendering capability
has been enabled. No Phase 3
checkbox is complete yet. At the page-only checkpoint, full legacy regression passed **348 checks** (299 Rust and
49 Python); two GPU-only frost tests remain explicitly ignored in that recipe.
Evidence: `target/draw-vfx/3d-data/storage/legacy-regression.log`. The rebuilt native
preview was inspected and left playing (`draw-world-preview-2767602`, PID 2768994).
Its live image matched release hash `5fff2eecd861508e909157ba4c7b672cb52fbf00b17b586685ceaa49e5e51689`.
The refresh stopped only the prior dedicated preview because its executable differed.
No visual material/geometry changes were made in this storage batch.

`test-draw-live --args=new` passed native Rust/CLI companion negotiation and legacy
Rust/Python/CLI lifecycle checks with that same binary. It explicitly confirmed that
new-data retention/rendering remain unavailable; it is not the Phase 3 retained-data
matrix. The isolated headless instance `draw-vfx-live-2777800` (PID 2777806) was stopped.
`test-draw-parity` passed 115 entity comparisons. Logs/capture are under
`target/draw-vfx/3d-data/storage/`; no controlled scale/performance claim is made.

### Immutable generations

`draw_data/{metadata,generation}.rs` adds charged immutable descriptor/scan records
and sorted shared-page manifests. Construction verifies exact scan totals, page
address uniqueness, schema/kind compatibility, one budget identity, cross-page source
IDs (including invalid observations), and cross-page occupancy topology. Scan-local
identity permits repeated IDs in different scans without replacing their capture poses.
Accumulated manifests check scan/byte bounds and common epoch/source-clock domains;
age-watermark eviction and runtime policies still require the transaction service.

Reservation splitting now transfers capacity into page construction without charging
the declared bytes twice. Shared manifests retain one page allocation while charging
their own reference tables. Construction validates outside the identity registry;
the existing exact-base publication facade can atomically swap complete manifests.

`test-draw-stream` now passes **33 tests** (19 page + 14 generation), including a real
**1,048,576-point, 16-page** CPU manifest and two shared generations under a **64 MiB
owned-allocation budget**. This is not process RSS, GPU rendering, transport throughput
or frame-time acceptance. All **146 typed-contract checks** pass again. Evidence:
`target/draw-vfx/3d-data/storage/generation-contracts.log`.

The current generation build also passes **348 legacy checks**, native Rust/CLI
companion negotiation and Rust/Python/CLI lifecycle, and **115 parity comparisons**.
See `generation-legacy-regression.log`, `generation-native-live.log` and
`generation-parity.log`. The native probe still verifies unavailable data retention;
it is not the new retained-data compatibility matrix.

The preview was rebuilt/restarted and its fresh capture inspected:
`draw-world-preview-2872498`, PID 2873677, release/live hash
`143fe5e41e48bb337b522ca16126a16575c2127f0fc130f059f5146cb6ec4181`.
It is left playing. The isolated native probe `draw-vfx-live-2949642`, PID 2949663,
was stopped. No OOM/shader/panic was reported; the pre-existing slab allocator
diagnostic remains. Other applications saturated the GPU during this checkpoint
(including another Gearbox and Astrocraft); the preview FPS is not a benchmark.

## Transaction owner checkpoint

`draw_data/{transaction,stream}.rs` now assembles and publishes real packed generations
through a single service-thread owner. Payload and merge/validation construction
capacity are reserved at BEGIN; immutable pages transfer their reservation into owned
storage. Decoding and generation validation run outside the identity mutex. Static
cloud deltas can replace counts, upsert/remove pages and share unchanged allocations;
occupancy parent removal/child insertion is validated as one complete generation.

Writer leases are random bearer tokens within existing transport ACLs, not authenticated
remote identities. Fake monotonic clocks drive independent writer/transaction expiry.
Global and per-writer transaction/incomplete/payload limits, terminal upload errors,
exact BEGIN/chunk retries and bounded completion history preserve the previous complete
generation. Transaction IDs increase per lease. A history miss never reexecutes an
old ID. Delete/clear barriers conservatively expire completion history globally, while
revoking only matching pending publications. Registry pins retain their own index charge.

Acquired scan metadata is immutable and new scan keys exceed the incarnation's committed
high-water mark, including after empty replacement or scan removal. Latest-frame admission
reserves a valid replacement before superseding pending work. Delta schema, octree
placement/configuration and stream policy remain unchanged; changes require replacement.

`test-draw-stream` passes **58 tests** (19 page + 14 generation + 25 transaction).
The transaction suite includes a real **1,048,576-point, 16-page** upload/publication and
shared delta inside a **160 MiB owned-allocation budget**, stale-base conflict, malformed
or overlapping chunks, CRC failures, page/writer/global limits, cancellation, independent
fake-clock expiry, retry eviction, acquired scan non-reuse, occupancy overlap and legacy
deletion barriers through the shared registry. A separate always-available 2 MiB
request workspace covers bounded JSON expansion beyond raw packet size; invalid
maximum-sized JSON is rejected without retaining a transaction. Evidence:
`target/draw-vfx/3d-data/storage/transactions.log`.

This is a standalone storage/transaction result, not native transport throughput, RSS,
renderer or complete Phase 3 acceptance. Host paths still call `serve_unavailable`;
runtime retention/rendering capabilities remain off. Accumulation age/whole-scan eviction,
simulation lifetime/reset policy, status/list/presentation service, bounded clients and
the actual native compatibility matrix remain required. No Phase 3 checkbox is complete.
All **146 typed-contract checks** pass on this source, including independent Python
fixtures and 11 native wire-only service tests; see `transaction-contracts.log`.

Final current-source verification also passes **348 legacy checks** (299 Rust + 49
Python, two GPU-only frost tests explicitly ignored), native wire negotiation and
legacy Rust/Python/CLI lifecycle, and **115 entity parity comparisons**. Evidence is
under `target/draw-vfx/3d-data/storage/`: `transaction-legacy-regression.log`,
`transaction-native-live-final.log`, `transaction-native-final/`, and
`transaction-parity-final.log`. The native probe still verifies unavailable data
retention/rendering; it is not the future retained-data compatibility matrix.

The dedicated preview was rebuilt/restarted and a fresh capture inspected. Current
instance `draw-world-preview-3334865`, PID **3336005**, is playing and left open;
release/live image hash is
`455ed2afd36435d0c6e862e7e4b29367859d561bb919771b1919be7f1ac589f0`.
The data-lab remains the unchanged 52-entity legacy-rendered scene. No OOM/shader/panic
was reported; the pre-existing slab-allocator startup diagnostic remains. GPU load
was shared with other applications, so the capture is not a performance result.
The isolated native probe `draw-vfx-live-3335378`, PID 3335387, passed and was stopped.

## Typed cloud foundation

`crates/gearbox-api/src/wire/draw_data/` now defines all eight planned scalar types,
semantic/normalization metadata, bounded schema JSON, packed little-endian columns,
position/attribute validity, exact u64 source IDs, and page validation. Color,
intensity, and categories coexist. Valid floats, metric radii, normals, confidence,
time units, byte counts, mask padding, and per-page ID uniqueness are validated.

The first `test-draw-data` checkpoint passed 12 new schema tests and 7 fixture/profile tests.
Evidence: `target/draw-vfx/3d-data/schema/typed-columns.log`. Its scope is CPU data
validation only, not the rest of the wire/store/renderer contract.
The full 345-test drawing suite also passes after this addition; evidence is
`target/draw-vfx/3d-data/schema/legacy-regression.log`. All owned test/capture instances
were stopped, and no tool process remains running at this checkpoint.

## Per-scan metadata, occupancy and page/envelope continuation

This continuation implements the revised plan's per-scan model rather than storing
one mutable capture frame on an accumulated dataset:

- Cloud descriptors own schema/presentation/policy; scans own capture pose/datum/time,
  source identity, source count and organized dimensions. Octree placement remains
  on its shape descriptor. Acquired frames require nonzero scan keys; static retained
  clouds use key zero. Cross-commit non-reuse still needs the store.
- Scan-table checks reject duplicates, excess scans, mixed accumulation epochs and
  incompatible source clocks. f64 CPU transform helpers test origin/pose order,
  delayed capture and anchored ECEF conversion; no renderer/sensor integration is claimed.
- Occupancy property fixtures exercise all 22 supported depths, signed extreme tiles,
  exact negative-coordinate floor division, Morton child order, invalid probabilities,
  explicit unknown states, and duplicate/ancestor overlap rejection.
- Binary cloud/occupancy page formats validate exact lengths before column copies,
  preserve explicit masks and absent values, and reject truncation/trailing bytes.
  Page descriptors validate complete-page CRC-32/ISO-HDLC. The already locked
  `crc32fast` 1.5.2 is now a direct dependency; no other dependency version changed.
- Distinct datapod request/reply envelopes preserve `gearbox.draw.v1`. Checked methods
  enforce section caps, actual packed size, routing fields and canonical wire layout.
  Reply stages separate received/retained/view-scoped visible states. These are wire
  contracts, not an implemented visibility state machine.
- Fixed an ROI validation edge case: finite opposite corners can still yield an
  infinite extent. A focused regression now rejects that region.

Fresh `test-draw-data`: **60 passed**, zero ignored/failed:
12 schema + 15 descriptor/frame/presentation + 9 envelope + 10 page + 7 octree +
7 fixture/profile tests. Evidence: `target/draw-vfx/3d-data/schema/final-contracts.log`.
The initial resumption also reran all seven pending-expiry plugin cases successfully
(`resume-expiry.log`).

After the final ROI validation fix, fresh `test-draw` passed **345** focused cases
(296 Rust + 49 Python; two separately invoked GPU frost cases remain ignored by that
recipe). Fresh `test-draw-parity` matched **115** entities (10 world, 24 composite,
81 helpers). Evidence: `schema/final-legacy.log` and `schema/final-parity.log` under
the same `target/draw-vfx/3d-data/` root. No GPU/native capture test was rerun in this
continuation, and these CPU/parity passes are not new-renderer acceptance.

At that earlier 60-case checkpoint no companion topics were registered. Envelope
validation does not replace typed operation decoding, descriptor/store checks,
quotas, generation checks or identity authorization. The data lane must use checked
wire decoding rather than relying only on legacy `serve_req`.
No new native scene was launched because no new renderer exists yet; Phases 3–13
and full Phase 2 acceptance remain pending.

## Companion service and negotiation continuation

- Added typed operation decoding, intrinsic presentation validation, resource-limit
  profiles, and capabilities that separate decodable, retained, and rendered kinds.
- Registered four companion routes in HostBus, polled by FakeHost and the Bevy draw
  thread. Wire/metadata checks precede dispatch; stale epochs, wrong routes and
  malformed packets receive bounded errors. No storage/render success is advertised.
- Added Rust selected-host capability discovery and synchronous checked requests.
  Old hosts return explicit unsupported without changing legacy drawing calls.
- Native tests caught incorrect normalization of the reserved agentio control topic.
  Corrected discovery to use the raw reserved topic, then reran successfully.
- The first verified checkpoint passed **85** cases: previous 60 + 9 typed-operation
  + 7 capabilities/limits + 9 actual native host/client cases. Evidence:
  `target/draw-vfx/3d-data/negotiation/contracts-host-fixed.log`. The initial failing
  run is retained in `contracts-host.log`; it is not acceptance evidence.
- Added same-endpoint restart and foreign-owner rejection cases; the expanded suite
  now passes **87** cases, including all **11 native host/client cases**. Evidence:
  `negotiation/final-contracts-rerun.log`. A missing borrow in the added test fixture
  caused the preceding compile failure (`final-contracts.log`); it is fixed.
- Fresh legacy regression passes **345** checks (296 Rust + 49 Python), including
  all seven Bevy plugin expiry cases. Fresh Rust/Python parity matches **115**
  entities. Logs: `negotiation/legacy-regression.log` and `final-parity.log`.
- `draw_data_probe` is built and invoked by `test-draw-live` to check the actual
  simulator's companion lane, alongside the existing Rust/Python/CLI lifecycle
  checks. The native run **passed**, then stopped its owned instance normally.
  Evidence: `negotiation/native-live.log`, `native/viewer.log`, and
  `native/binary.sha256`. Instance `draw-vfx-live-1626675`, PID 1626682, ran the
  freshly built executable (`aa8e6f943ade2d676e4f81df256d7ba74c3cd468069fc854a0c26c7660f9c911`).
  Companion negotiation reported cloud/octree decoding only and correctly refused
  writer acquisition without storage; all legacy lifecycle and Python/CLI updates
  passed. The baseline Bevy slab-allocator startup diagnostic remains present;
  no new OOM/shader-validation/panic was reported and no NOT_ACCEPTED marker exists.

These service and regression checks are not new-renderer visual/performance acceptance.

Protocol details and transport limitations are recorded in `DRAW_DATA_PROTOCOL.md`.
Actual retained data, live unified lifecycle, bounded streaming,
all new renderers, and Releases A/B remain unfinished. An interim legacy-rendered
visualization is now available; it does not implement typed dataset rendering.

## Identity, capture, status and independent-wire checkpoint

Phase 2's contract gates now pass. This statement covers the contract/model layer,
not the live retained-dataset compatibility matrix assigned to Phase 3:

- A generic `DrawingStore<T>` owns the real legacy store, bounded dataset reservations,
  per-incarnation generations and immutable payload pointers. Thirteen tests cover
  legacy replacement behavior, pending ownership, competing commits, cross-store
  permits, delete/clear cancellation, long legacy keys, and retained Arc lifetime.
- Native `capture.v1` metadata preserves the LiDAR v1 fixed wire layout. Eight tests
  cover capture pose/source/epoch consistency, full u64 identity, exact timestamp
  round trips, malformed props, atomic rejection and metadata-aware pulse capacity.
- A 4,096-value regression exposed a serde_json one-ULP timestamp roundtrip error.
  Enabling `float_roundtrip` fixes it without weakening equality. Initial failure:
  `identity/capture-first.log`; verified fix: `identity/capture-fixed.log`.
- Twelve tests cover bounded mixed listing/status summaries, UTF-8/escaped byte
  budgets, exact generation/view acknowledgement, failure/progress shapes and
  unique resource totals. Real counters, cursors and view acknowledgements are not
  implemented by these shapes alone. An initial byte-budget fixture used too few
  escaped bytes; the corrected fixture exercises the real 12 KiB limit.
- Five API JSON and two raw CLI tests enforce checked metadata and lossless section
  bytes. All four companion topics are registered in the CLI schema table.
- Three Rust/CLI plus sixteen independent standard-library Python tests verify
  **37 frozen packets and three page fixtures**, all eight scalar types, signed/unsigned
  extrema, masks, CRCs, known/unknown occupancy, capture props and unchanged legacy
  drawing/LiDAR headers. Ordinary tests check hashes; regeneration is explicit.
- The full `test-draw-data` gate passes **146 checks** (130 Rust + 16 Python), including
  all eleven native host/client cases. `identity/final-contracts.log` records the run.
  `identity/final-wire.log` reruns the wire gate after tightening Python's rejection
  of JSON integer literals too large for a finite generic JSON number.
- Fresh existing drawing regression: **345 checks** (296 Rust + 49 Python),
  `identity/legacy-regression.log`. Fresh Rust/Python parity: **115 entities**,
  `identity/final-parity.log`.
- A fresh native simulator passed Rust companion negotiation, the actual raw CLI
  capability call, and legacy Rust/Python/CLI lifecycle/update checks. Evidence:
  `identity/native-live.log`, `identity/native/{viewer.log,binary.sha256,cli-capabilities.json}`.
  Instance `draw-vfx-live-2006681`, PID 2006687, used binary SHA-256
  `81eb171b4c08e49dae7cba6b88b1921f1a986b511fe617be75f0e82de6691039` and stopped normally.
  No new OOM/shader/panic or failure marker appeared. The pre-existing Bevy slab
  allocator startup diagnostic is still present; this is not a clean-bill-of-health
  claim for that unrelated baseline issue.

All evidence paths above are relative to `target/draw-vfx/3d-data/`.
The native run is a **headless functional test**, not a new representation preview.
No new renderer, native sensor capture producer, million-point performance result,
or user visual acceptance is claimed. Capabilities still advertise no retained or
rendered dataset kinds. Actual page ownership, live facade integration, safe bounded
transport and Release A/B remain open; continue with Phase 3 without narrowing scope.

## User-requested live spatial preview

During the Phase 3 continuation the user asked to see data in Gearbox immediately
and reaffirmed that every data family must retain the same sci-fi language. Added
`data-lab`: three attribute views of 5,852 synthetic points and two presentations of
a valid 113-leaf mixed-resolution occupancy tree. It explicitly uses legacy drawing
primitives while the real backend is unfinished. See `DRAW_DATA_PREVIEW.md`.

The first capture exposed crowded labels; spacing/camera targets were corrected.
The user requested **more energized detail**. The revised scene adds metric rulers,
partial selection arcs, low glass rims and moving level accents, not a global glow
increase. Source geometry and scientific colors are preserved. This is a new useful
visual review, not replay of the old catalogue, and not Phase 3/4 acceptance.

## Key Learnings:

1. Abandoned upload cleanup must be scheduled independently of live entity lifetimes.
2. Native visual baselines and functional gates do not establish controlled scale performance.
3. Each accumulated scan needs independent capture metadata and source identity.
4. Page integrity, envelope validity, store authorization and renderer visibility are separate gates.
5. The raw agentio resolver topic must not pass through application slash normalization.
6. Successful negative discovery is feature absence; a transport failure is not.
7. Exact capture timestamps require round-trip float parsing, not approximate matching.
8. Legacy identity bounds must not be silently tightened to fit new dataset keys.
9. Independent golden codecs test byte compatibility; neither they nor model stores prove live retention.
