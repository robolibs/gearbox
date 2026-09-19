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

Evidence in this session: `/tmp/molla-before-trace.json`,
`/tmp/molla-gpu-final-trace.json`, `/tmp/perf-rapier-matched.log`,
`/tmp/perf-molla-buffers.log`, `/tmp/molla-fast.log`,
`/tmp/tyre-buffer-low.png`, `/tmp/tyre-buffer-high.png` and corresponding JSON.
These temporary artifacts are not tracked and may disappear after reboot.
