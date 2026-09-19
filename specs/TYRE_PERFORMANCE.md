# Tyre rendering performance checkpoint — 2026-09-20

This is a measured improvement, not a general Rapier-parity acceptance result.
Only the isolated `gearbox-physics` checkout was changed. The original Gearbox
checkout, build and active user instance were not modified or stopped.

## Scene and measurements

RTX 4080/Vulkan, development builds with optimized dependencies, 1280x800,
flat terrain, one `kubota_tractor.usdz`, playing at the existing 120 Hz setting.
All 28 rubber meshes deform; tyre support, grip and inflation rate remain active.

| Renderer/backend | Observed stabilized rolling FPS |
| --- | ---: |
| Original Gearbox executable / Rapier | approximately 40–44 |
| Isolated Molla, previous CPU rubber path | approximately 8 |
| Isolated Molla, GPU with full material updates | approximately 34–36 |
| Isolated Molla, persistent GPU buffers | approximately 42 |

The final buffer run logged rolling averages 41.59 and 42.48 before camera/window
interaction. Exclude its later 69 FPS reading from this comparison. A subsequent
close-up pressure/driving run was approximately 35–38 FPS. These are interactive
samples, not confidence intervals or a guarantee at every camera/resolution.
Other user work and the original instance continued running; background load
changed. The original renderer has independent changes and is not pixel-identical
to the isolated renderer. The CPU baseline used system tracing, whereas the final
FPS checks did not. Repeat controlled fleet/terrain scenarios before declaring
complete parity.

The original executable was copied read-only to
`target/perf-artifacts/gearbox-original`, SHA256
`f311de5b3e1e5bca81185ade3134cd67064592f78f0306c82550f5012d273959`.
Its source checkout HEAD at snapshot was
`e48d7b72302a2af914f68d3af89d92e173416e3b`; that does not independently prove
the binary's build provenance.

## Measured bottlenecks and changes

Chrome traces excluding initial loading:

| System | Previous CPU path, mean ms/frame | Intermediate GPU path, mean ms/frame |
| --- | ---: | ---: |
| `update_tyre_meshes` | 88.64 | 1.23 |
| mesh allocation/free | 19.96 | no longer a leading cost |
| full material preparation + bind groups | — | 2.21 + 1.39 |

The first change removes CPU vertex deformation, normals rebuilding and repeated
mesh uploads. Per-frame ancestor lookup is memoized, and wheel/link association
no longer repeatedly scans every USD prim. The final refinement replaces dynamic
material uniforms with persistent 320-byte storage buffers: parameter writes no
longer reprepare the entire PBR material and texture bindings. GPU rubber does
not retain an extra CPU copy of the reference positions.

CPU fallback is retained with `GEARBOX_TYRE_RENDER=cpu`. Unsupported material,
skinned or morphed meshes keep the reference path. No solver substeps, physics
frequency, visual geometry detail or pressure features were reduced.

Physics remains a separate gap: sampled Molla steps were approximately 1.5–1.6 ms
versus 0.35–0.46 ms for original Rapier in these runs. Roughly 1,200 fixed steps
per ten seconds were logged. This work does not establish solver or fleet-scale
parity.

## Solver follow-up — 2026-09-20

Rebuilt the isolated viewer after Molla's tyre mass-factor and triangle-mesh
BVH caching changes. The real 26-body, 4,917 kg Kubota on flat ground still
measured roughly 1.47–1.59 ms per step at 120 Hz, driving straight at 2 m/s.
The mesh gate improved to 28.9 microseconds/call, but the flat-ground scene is
not that mesh benchmark. Do not extrapolate its speedup to the live tractor.

Focused stage traces after warmup measured approximately 19 microseconds for
collision, 148 for external forces (including 66 for wheels), and 67 for a
Featherstone substep (including 36 for drive inertia). There are eight substeps.
These inclusive timings overlap and tracing adds overhead: the traced full
step was approximately 2.00 ms, not the untraced 1.5 ms measurement.

The loop-closure and motor inertia paths also repeated singular mass
decompositions across right-hand sides. Reusing those factors within the
current articulation/substep reduced a synthetic 26-body pressure-tyre scene
with a compliant closure and motor from median 0.855 to 0.527 ms/step (38%).
Replay, support, closure convergence and driving checks pass. This fixture is
not the imported Kubota. The subsequent live run still measured 1.51–1.61 ms
in typical windows, with 1.72–2.06 ms spikes during concurrent user work.
No real-machine gain or Rapier parity is established by that run.

Molla release solver regression coverage: 371 passed, 0 failed, 3 ignored.
Isolated Gearbox Molla binary tests: 78 passed, 1 ignored. Original Gearbox
remains independently running/developed and was not modified or stopped.

Evidence: `/tmp/molla-cache-live.log`, `/tmp/molla-physics-stages.json`,
`/tmp/molla-physics-stages-summary.txt`, `/tmp/molla-loop-perf-before.log`,
`/tmp/molla-loop-perf-after.log`, `/tmp/molla-loop-cache-full-tests.log`,
`/tmp/molla-loop-cache-final-tests.log`, `/tmp/molla-loop-live.log`, and
`/tmp/molla-loop-state.json`. The last telemetry sample reports all four tyres
at 1.8 bar and approximately 53.5–54.3 mm deflection while driving at 2 m/s.

The detailed loop trace identifies 39 DOFs, 22 constraint rows and a successful
Cholesky factorization. The full-rank response now uses a normalized Cholesky
Gram product, avoiding the second triangular solve and duplicate symmetric
products. A paired 39-DOF/22-row benchmark measured 15.292–21.525 microseconds
for Gram assembly versus 25.535–43.504 for the previous inverse products;
every paired batch improved. Full/singular inverse-product reference tests
include scales 1e-60 through 1e60. The tiny-scale test also fixes an existing
unnormalized triangular-solve failure. Latest solver suite: 372 passed,
0 failed, 4 ignored; isolated Molla binary tests: 78 passed, 1 ignored.

Real-machine acceptance remains open. Concurrent release builds disturbed
timings. The final `molla-gram-perf` drive reached 2 m/s, later stopped around
x=8.82 m, and the window closed before final telemetry could be collected.
A selection event was logged during the run; the stop's cause is not proven.
Do not count the CLI's completed-duration acknowledgement as successful
continuous driving. Reproduce with controlled inputs/headless import before
claiming that regression gate. A pre-load renderer slab use-after-free error
also occurs in both earlier and latest runs, not only after the Gram change.
Evidence: `/tmp/molla-gram-{paired,full-tests,drive,live}.log` and
`/tmp/molla-gram-stages{.json,-summary.txt}`.

## Correctness and gates

- Binary tests cover conservative bounds, both pressure directions, unchanged
  source meshes, material edits/reassignment/restoration, stable buffer/material
  handles and previous-frame data. Run with both backend selections.
- The ignored Vulkan test executes the production deformation function over
  124,416 cases. Observed maximum error against the f64 CPU reference:
  0.0000001562 metres, below the 0.00002 metre assertion threshold.
- Inspected low/high-pressure rendered screenshots from the storage-buffer path.
  Front deflection changed from 82.61 mm at 0.5 bar to 35.70 mm at 4 bar; rear
  deflection changed from 78.54 to 37.17 mm. Inflation raises the chassis and
  retains the loaded contact patch. Driving reached the requested 2 m/s.
- Native `oslo make test` currently fails to compile the existing
  `bin/gearbox/tests/oxbo_transforms.rs`, which references removed
  `openusd::Stage` and `usd_schema` APIs. Do not describe the full suite as green.
- Multi-machine scaling, nonplanar terrain/camber, every prepass variant and
  calibrated tyre behavior remain separate acceptance work.

## Reproduction

From the isolated checkout, build through its native recipes:

```sh
GEARBOX_PHYSICS=molla nix develop --impure -c oslo make build
GEARBOX_PHYSICS=molla GEARBOX_TERRAIN=flat GEARBOX_WINDOW=1280x800 \
  GEARBOX_TYRE_MESH_TRACE=1 WINIT_UNIX_BACKEND=wayland WAYLAND_DISPLAY=wayland-0 \
  nix develop --impure -c nixVulkan target/debug/gearbox run \
  --name tyre-perf --ephemeral
```

In another terminal:

```sh
target/debug/gearbox instance wait tyre-perf --timeout 90
target/debug/gearbox -i tyre-perf spawn machine \
  /home/bresilla/data/code/OUSD/machines/usd/kubota_tractor.usdz \
  --machine tractor --wait-timeout 90 --timeout 60
target/debug/gearbox -i tyre-perf machine tyre-pressure 0.5 --machine tractor
target/debug/gearbox -i tyre-perf machine tyre-pressure 4 --machine tractor
```

Wait for applied pressure, not just the target acknowledgement. Warm up before
sampling; keep window size, camera, scene, background work and pressure fixed.
Use a separate process/run for CPU fallback or Rapier comparisons.

For system attribution, use `oslo make build-profile` and set `GEARBOX_TRACE` and
`GEARBOX_TRACE_SECONDS` when launching. Do not mix traced and untraced timings
without labeling them. Trace files can grow to several GB; stream event records
instead of loading the complete JSON array into memory during a benchmark.

Set `GEARBOX_TRACE_FILTER='off,molla_solvers=debug'` to record only Molla's
rigid-step, collision, external-force, wheel, loop-closure, mass-matrix and
Featherstone stages. Omit it to retain the existing system/render trace filter.
Debug stage spans are disabled by the normal info-level logger. Inspect nested
spans as inclusive timings rather than summing parent and child durations.

Evidence in this session: `/tmp/molla-before-trace.json`,
`/tmp/molla-gpu-final-trace.json`, `/tmp/perf-rapier-matched.log`,
`/tmp/perf-molla-buffers.log`, `/tmp/molla-fast.log`,
`/tmp/tyre-buffer-low.png`, `/tmp/tyre-buffer-high.png` and corresponding JSON.
These temporary artifacts are not tracked and may disappear after reboot.
