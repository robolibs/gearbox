# Molla backend integration

`GEARBOX_PHYSICS=molla` selects the CPU Featherstone backend. The default
remains Rapier. Both implementations share Gearbox's existing body, collider,
joint and world traits; the Molla adapter is in `bin/gearbox/src/physics/molla`.

## Paired performance baseline (2026-09-20)

`imported_kubota_backend_timing` is an ignored headless benchmark using fresh
real Kubota fixtures in Rapier/Molla/Molla/Rapier order. Both use 120 Hz and
their production-default settings, with all five controllers outside the
physics-step timer. Molla pressure tyres are enabled at 1.8 bar; Rapier uses
its reference collider tyre behavior. Each parked/driving case has 600 warmup
and 1,200 timed steps. Finite-state/speed checks are assertions; timing output
alone does not certify performance acceptance.

Measured default medians: **Rapier 0.2256–0.2324 ms, Molla 0.9420–0.9487 ms**,
about **4.1x slower**. All 26 bodies were awake on both backends. Molla's
sub-ms median is not Rapier parity or a guarantee about latency spikes.

The test-only `GEARBOX_BENCH_MOLLA_INTERNAL_ITERATIONS` override accepts 1–64
and logs effective settings. Values 1/2 produce 2/4 Featherstone substeps;
the production default remains 4 (eight substeps). Two substeps measured
about 0.25 ms but failed trailer support, hitch tracking, slope and turning
gates. Four measured about 0.48 ms but failed slope and turning gates. Neither
configuration is accepted for production; no thresholds were relaxed.

The existing `imported_kubota_solver_trace` test runs with
`--features gearbox-sim/profile`, `GEARBOX_BENCH_ASSET` and `GEARBOX_TRACE`.
Its instrumented profile shows costs concentrated in Featherstone, joint-loop
responses and tyre forces. The 39-DOF/22-row loop system uses Cholesky, not a
spectral fallback. Traced timings include overhead and must not replace the
uninstrumented comparison.

Molla `9d91efd` also corrects box-face contact midpoints without changing
penetration depths. Final default-settings validation: seven imported tests
pass; both ordinary backend lanes pass 87 tests with eight ignored.
Logs: `/tmp/molla-paired-backend-baseline.log`,
`/tmp/molla-{two,four}-substep-experiment.log`,
`/tmp/molla-current-cpu-profile.tsv`, `/tmp/molla-midpoint-default-imported.log`.

## Latest convex-support and towing validation (2026-09-20)

Molla `c95b4c3` repairs a CPU GJK distance-reduction error that intermittently
dropped penetrating trailer hull/ground contacts. This resolves the residual
low-pressure Krampe rocking described in earlier checkpoints without changing
tyre stiffness, damping, assets or acceptance bounds. The unchanged imported
15 s support gate now reports 0.0000342/0.0000548/0.0000145 m/s at 0.5/1.8/4 bar.
All six imported-machine gates pass. Both explicit-backend binary lanes pass
87 tests with seven ignored, and native `oslo make build` passes.

The Krampe fixture accepts `GEARBOX_BENCH_TRACE_SUPPORT=1` for once-per-second
body energy, contact impulse contributions and tyre loads, plus initial
body/collider identity. Per-contact `step_mean_force` is its impulse divided
by the full step duration; substep contributions must be summed.

Fresh live instances `molla-gjk-tow-low`, `molla-gjk-tow-high`, `rapier-gjk-tow`
completed attachment, an eight-second 2 m/s / 0.4 rad/s command, detachment and
empty attachment-list readback. Molla trailer readouts retained all four
0.5/4-bar settings after detach. Final trailer pitches were
-0.000223/+0.003044/+0.013473 rad respectively. No rejected-step, non-finite or
quarantine diagnostics appeared. All owned viewers were explicitly stopped.
Rear-view screenshots were inspected; calibrated pressure deformation and
all-wheel contact are not proven by those views.

Evidence: `/tmp/molla-gjk-all-imported.log`, `/tmp/molla-support-fixed-gjk.log`,
`/tmp/molla-gjk-{low,high}-*-state.json`, `/tmp/rapier-gjk-*-state.json`,
`/tmp/molla-gjk-{low,high}-detached.png`, `/tmp/rapier-gjk-detached.png`.
The full plan remains incomplete: performance tails, fleet behavior, pressure
save/reload and UI lifecycle, calibrated tyre/soil behavior and GPU parity
still require their own gates. Sequential Kubota timing medians remain about
0.96 ms; this is not a claim of Rapier performance parity.

## Current implementation

- Stable handles, runtime body/collider/joint edits, re-rooting, mass/inertia
  queries, forces, impulses, axis limits/locks and motor controls use Molla's
  runtime scene directly. Accessors share a mutex-protected world rather than
  maintaining stale physics copies.
- Collider insertion/removal and mass-affecting changes recompute parent mass.
  Body additional mass is separate from collider contributions. Collider world
  poses and ray queries read current body transforms without a simulation step.
- Connected-body filtering, per-step pair exclusions, collision masks, material
  combine rules and per-contact impulse readout map through the existing trait.
- Internal state quarantine is exposed through backend body IDs and forwarded
  to `PhysicsWorld`'s entity report after each step.
- All Molla joints are reduced-coordinate joints, including requests that would
  select Rapier constraint joints. `joint_is_reduced` reports this truthfully.
- Soft joints lower to D6 spring freedoms while retaining their logical kind,
  authored frames and semantic motor axes. Runtime frame edits preserve live
  body kinematics and change the spring rest frame. Hitch capture's 8-to-30 Hz
  updates drive real Featherstone forces rather than retaining rigid locks.
- `internal_iterations` maps to twice that many Featherstone substeps, bounded
  to 2–128. Featherstone's direct articulated solve does not use Rapier's outer
  iteration count. Requested settings remain readable.
- Scene wheel metadata registers stable wheel and spin-joint handles with the
  articulated tyre-force layer. Rolling direction follows the bearing/knuckle,
  not the spinning wheel; opposing authored axles receive opposing signed motor
  targets for the same forward command. Geometry/mass changes refresh registration.
- Meadow heightfields, USD terrain meshes and the flat ground slab register as
  tyre ground. Wheel-ground shape pairs are masked from penalty contact without
  masking obstacles. Terrain friction grids are accepted through the backend API;
  current scene builders supply their existing scalar ground/material friction.
- Traction budgets consume mean tyre normal/friction loads. Wheel stamping uses
  tyre contact points and contact state, while link values expose tyre slip,
  slip angle and normal force. Rapier keeps its contact-based path.

## Not yet accepted

Sleeping/waking are no-ops and bodies report never sleeping. CCD is a stored
no-op, as allowed by the Molla integration
specification. Live machine acceptance remains unfinished. Do not treat this
checkpoint as a production backend release.

Tyre parameters currently use nominal supported mass per wheel: 4% radial
deflection at nominal load, damping ratio 0.7, and load-scaled smooth slip gains.
These are initial parameters, not validated tractor calibration. Current field
profiles expose visual ground materials, not a physics friction grid; connecting
authored per-field/cell friction remains separate from the tested grid API.
Deformable terrain, MPM coupling and the live drive/track/slope gates remain.

Compliance uses scalar backward-Euler force regularization with effective
articulation inertia, not a fully coupled implicit spring solve. Static
deflection depends on the substep size. Soft angular coordinates use D6's XYZ
chart; unwrapped multi-turn soft revolute position targets are not implemented.
The hard revolute path retains its scalar coordinates.

Authored `excludeFromArticulation` edges now become compliant loop-force
joints, separate from the reduced tree, initially 30 Hz and critically damped.
Their coupled articulated response, unilateral bounds and motor caps produce
substep body wrenches; Featherstone remains the rigid integrator. Ordinary
unmarked tree cycles remain rejected. Loop orientation uses the principal
rotation log, not unwrapped multi-turn coordinates. Free-acceleration/bias
prediction is not included in the implicit loop force solve.

Invalid body descriptions or impossible
joint insertions fail explicitly; fallible runtime edits log rejection and
retain the last valid operation state. Collider replacement followed by mass
recomputation is not yet one combined transaction and needs failure-path
hardening before final acceptance.

## Pressure-dependent tyres

The Molla wheel path now enables the core pressure brush model for discovered
machine wheels. Rapier keeps its existing contact path. Width and unloaded
radius come from wheel collider geometry; these optional link values override
the uncalibrated reference configuration:

| `gearbox:value:` suffix | Unit | Reference |
| --- | --- | --- |
| `tyre_pressure_bar` | gauge bar | 1.8 |
| `tyre_min_pressure_bar` | gauge bar | 0.5 |
| `tyre_max_pressure_bar` | gauge bar | 4.0 |
| `tyre_pressure_rate_bar_s` | bar/s of simulated time | 0.2 |
| `tyre_width_m` | m | measured geometry |
| `tyre_carcass_stiffness_pa_m` | Pa/m | 400000 |
| `tyre_tread_stiffness_n_m3` | N/m³ | 2600000 |
| `tyre_damping_ratio` | dimensionless | 0.7 |
| `tyre_hysteresis_fraction` | dimensionless | 0.1 |

These defaults are not calibrated Kubota/Fendt tyre data or operational
inflation recommendations. Backend pressure values are gauge Pa, converted
explicitly from the authored/UI gauge bar values. The normal spring is
replaced by pressure contact support, not added to it. Traction uses the
brush grip budget and loaded rolling radius. Field stamps consume the actual
patch length/width rather than a fixed longitudinal footprint.

The Machine sidebar has per-wheel and all-wheel
target sliders, actual pressure, loaded radius and tread contact area.
The all-wheel slider requires overlapping supported ranges. Targets apply
atomically per machine through the same backend path as CLI edits:

```sh
target/debug/gearbox -i INSTANCE machine set-value wheel_front_left \
  tyre_target_pressure_bar 1.0 --machine tractor
target/debug/gearbox -i INSTANCE --json machine state tractor
```

`set-value` queues a request; its immediate echo is not acceptance evidence.
Invalid groups are logged and readback returns the accepted targets. Paused
edits change targets only, retaining actual pressure and previous contact
support. Inflation/deflation advances only when simulation advances. Targets
survive unchanged registration and unrelated scene rebuilds, not save/reload
yet. Link telemetry includes `tyre_pressure_bar`, `tyre_target_pressure_bar`,
bounds, `tyre_loaded_radius_m`, `tyre_deflection_m`, patch length/width,
`tyre_tread_area_m2` and rolling moment magnitude. Area is summed curved
tread-cell area, not projected ground footprint area.

Pressure pods appear before variants and collapsible controller sections, so
their response offsets do not depend on folded sections. Sliders synchronize through Mara's typed memory store,
not egui's separate persisted-value store. Regression tests cover tab routing,
all-wheel versus individual edits and authoritative slider memory. The direct
sidebar layout was visually checked in `/tmp/molla-sidebar-pressure.png`;
the corresponding Molla binary suite passes 64 tests and native build passes.

Live Kubota evidence: straight 2 m/s with zero turn reaches about 2.01 m/s;
lowering front pressure from 1.8 to 1.0 bar increases settled deflection from
about 53 to 67 mm, while raising rear pressure to 2.4 bar reduces it to 47 mm.
The four loads total about 48.2 kN. A captured all-wheel UI edit changed all
accepted targets from 1.8 to 2.58427 bar without changing paused pressure or
deflection (`/tmp/molla-pressure-controls-all.{json,png}`). The mixed-target
view exposed stale slider memory; the subsequent typed-memory correction is
unit-tested, but its final live CLI-to-slider recheck was interrupted when
the test instance disappeared. Do not count that recheck as passed.

Remaining tyre requirements include axle controls, saved pressure state,
visible mesh flattening/bulging, nonplanar/per-cell scene friction, calibration,
pressure-extreme towing/slope gates, deterministic replay, GPU parity and the
timing target. Footprint plumbing is tested, but visual pressure-sweep stamp
comparison is still pending. This is an integration checkpoint, not full
`FULL_COMPACT.md` acceptance.

## Build notes

### Imported Kubota slope gate

The unattended `controller::benchmark::imported_kubota_slope_parking` test
imports the real 26-body Kubota, rotates the machine and ground together by
both -10 and +10 degrees, commands zero drive, and tests 0.5 and 4.0 bar.
After 10 simulated seconds for placement/pressure settling, it measures 60
seconds at 120 Hz. The gate requires less than 10 mm maximum ground-tangent
displacement and 1 mm/s maximum chassis speed throughout that interval.
Each step checks finite, enabled bodies and no quarantine; each simulated
second checks four supported wheels, applied pressure, the sloped ground
normal and total normal load within 500 N of `mass * g * cos(10 degrees)`.

At Molla `b6d8f27`, two runs reproduced these failures:

| Slope | Pressure | Maximum drift over 60 s | Maximum speed |
| --- | --- | --- | --- |
| -10 degrees | 0.5 bar | 7.998699257 m | 0.133314298 m/s |
| -10 degrees | 4.0 bar | 9.031795585 m | 0.150532806 m/s |
| +10 degrees | 0.5 bar | 7.973623087 m | 0.133080483 m/s |
| +10 degrees | 4.0 bar | 9.042705112 m | 0.149730977 m/s |

The support/pressure/normal/finite-state checks passed. Diagnostic output
shows nonzero wheel rotation and slip ratios with magnitudes about 0.19–0.22.
This is a reproduced parking failure, not an interaction or window-lifecycle
inference. No production physics, gains, brakes or tolerance were changed to
make this test pass; it remains explicitly failing when invoked. Its ignore
attribute is for the external asset requirement, not acceptance exemption.

```sh
GEARBOX_PHYSICS=molla \
GEARBOX_BENCH_ASSET=/home/bresilla/data/code/OUSD/machines/usd/kubota_tractor.usdz \
nix develop --impure -c cargo test --release -p gearbox-sim --bin gearbox \
  controller::benchmark::imported_kubota_slope_parking \
  -- --ignored --nocapture --test-threads=1
```

Logs: `/tmp/gearbox-slope-{initial,diagnostic}.log`. This fixture is Molla-only;
it is not a Rapier slope result or a visual acceptance check.

With Molla `73f50bf`, retained per-cell pressure-tyre shear is active in the
Featherstone force path. The imported driving/pressure benchmark and exact
replay pass, but the unchanged slope gate still fails:

| Slope | Pressure | Maximum drift over 60 s | Maximum speed |
| --- | --- | --- | --- |
| -10 degrees | 0.5 bar | 3.944096269 m | 0.065777049 m/s |
| -10 degrees | 4.0 bar | 4.485188503 m | 0.074782764 m/s |
| +10 degrees | 0.5 bar | 3.836886247 m | 0.064224411 m/s |
| +10 degrees | 4.0 bar | 4.418200279 m | 0.073664558 m/s |

At +10 degrees/4 bar, slip magnitudes fell to about 0.00068–0.00213 while
the wheels still rotate under the unchanged parking velocity motors. Parking
hold remains unfinished; neither friction nor acceptance tolerances were
increased to hide that. Source regressions cover retained shear, frame
projection, airborne reset, unrelated rebuilds, rollback and teleport.
The ordinary release binary suite passes 84 tests with four environment-dependent
tests ignored (three imported-asset gates plus the Vulkan parity test).
Final imported log: `/tmp/gearbox-shear-imported-final.log`.

That run measured full-machine CPU-step medians of 0.7423–0.7587 ms across
parked/driving and 0.5/1.8/4 bar; p95 was 0.8522–1.1040 ms, maxima up to
2.0531 ms. This is not rendered FPS, paired speedup evidence or proof of
Rapier parity. The debug viewer was not rebuilt/launched for this checkpoint.

#### Bounded parking hold: passing checkpoint

The Ackermann controller now captures the current motor-space wheel coordinate
when parking engages. Molla exposes unwrapped hard-revolute and prismatic
coordinates; unsupported joints/backends report no coordinate, retaining the
existing velocity brake. In particular, Rapier's brake behavior is unchanged.

Parking uses a force-based position motor with the existing authored/shared
torque budget. Reference gains are `torque / 0.01 rad` stiffness and
`torque / (0.25 rad/s)` damping. Captured-target error is bounded to 0.25 rad
under sustained slip, and discontinuous coordinate jumps rebase the hold.
Driving releases the reference and normal velocity control clears stiffness.
This is a compliant, torque-limited angular hold, not a body freeze or a
calibrated dry-friction brake model.

Both independent imported runs pass the unchanged gate, including all four
pressure/slope combinations and their support/finite-state checks:

| Slope | Pressure | Maximum 60 s drift | Maximum speed |
| --- | --- | --- | --- |
| -10 degrees | 0.5 bar | 0.000000393 m | 0.000002449 m/s |
| -10 degrees | 4.0 bar | 0.000000489 m | 0.000002610 m/s |
| +10 degrees | 0.5 bar | 0.000000207 m | 0.000001517 m/s |
| +10 degrees | 4.0 bar | 0.000000250 m | 0.000001653 m/s |

The final command also passes imported pressure/driving and exact replay:
three tests passed, zero failures. Three focused brake tests cover multi-turn
capture, loaded hold, release/re-engagement, zero budget, removed joints,
overload slip with bounded reference, and Rapier's unchanged fallback.
Ordinary release binary suites pass 87 tests each with explicit Molla and
Rapier selection; four environment-dependent tests are ignored in those ordinary
suites (three imported-asset gates and the Vulkan parity test). Native
`oslo make build` passes. The viewer is rebuilt, not launched.

Final CPU-step medians are 0.7320–0.7448 ms; p95 0.7643–1.2219 ms and maxima
up to 2.3572 ms on the shared host. No other assistant test/build overlapped
those samples. These remain headless step samples, not hard real-time or
Rapier-parity acceptance. Logs: `/tmp/gearbox-parking-{slope,imported,unit}.log`,
`/tmp/gearbox-parking-{molla,rapier}-regression.log` and
`/tmp/gearbox-parking-build.log`.

### Five-controller imported fixture

The headless imported fixture now executes production service controllers as
well as the drive controller each tick. Earlier headless checkpoints did not
execute the service-controller schedule; their timings and states are not a
five-active-controller baseline. The service schedule is test-only wiring and
does not change the live application's existing service path.

`controller::benchmark::imported_kubota_hitch_and_pto_controllers` requires the
authored five controllers, resolves each service's target joint/link, and
writes the same link values consumed by production service controllers. It
checks actual motor-space coordinates, not merely requested motor targets:

- Hitch commands 0.75 and then 0.15 move both joints, stay within authored
  limits at the raised position, and track within 0.05 rad. The front hitch's
  authored range is negative. Raised front/rear positions were -0.295727 and
  +0.686723 rad; returned positions -0.058691 and +0.106347 rad.
- Front/rear PTOs use authored 1000/540 RPM values. Measured over one simulated
  second after settling, rates were 104.404530/56.378446 rad/s, within the test's
  0.5 rad/s tolerance of their targets. After disengagement and settling, every
  step of a one-second observation is checked below 0.01 rad/s; measured peaks
  were about 1.39e-8/0 rad/s. All bodies stay finite/enabled with no quarantine.

With the corrected fixture, all four imported tests pass: hitch/PTO actuation,
pressure/driving, exact replay and slope parking. Physics-step medians across
parked/driving and pressure settings were 0.7472–0.7992 ms, p95 0.7945–1.4431 ms,
maxima up to 1.8752 ms on the shared host. Controller execution remains outside
the timed physics step; this is not rendered FPS, full-frame timing or matched
Rapier performance. No assistant build/test overlapped those timing samples.

Reproduce using the slope command above with the test filter replaced by
`controller::benchmark::imported_kubota_`. Logs:
`/tmp/gearbox-five-controller-gates.log` and
`/tmp/gearbox-five-controller-final.log`. This is Molla headless actuation
coverage, not Rapier service actuation, CLI/network delivery, visual validation,
or attached-master/slave service propagation.

Ordinary release binary suites pass 87 tests on each selected backend, with
five ignored: the four imported-asset tests above and one Vulkan parity test.
Earlier summaries describing every ignored test as external-asset-dependent
were imprecise; Vulkan parity is a separate environment gate.

### Straight driving, turning and track acceptance

`controller::benchmark::imported_kubota_straight_turn_and_track_contacts`
imports the real Kubota into both backends. Each fresh scene settles for 5 s,
then drives at 2 m/s for 4 simulated seconds with yaw commands 0 and ±0.4 rad/s.
Molla repeats at 0.5, 1.8 and 4 gauge bar. Production drive, hitch/PTO and
`record_wheel_tracks` systems run; the fixture clears track inputs every tick.
Every step checks finite, enabled bodies and no quarantine. The final 3 s
require four finite, ground-level, normalized track inputs per tick.

| Backend / pressure | Straight displacement (m) | Positive turn (rad) | Straight mean patch length (m) |
| --- | ---: | ---: | ---: |
| Rapier | 7.8084 | 1.49472 | geometric fallback |
| Molla / 0.5 bar | 7.3399 | 1.47032 | 0.60443 |
| Molla / 1.8 bar | 7.3020 | 1.46211 | 0.49442 |
| Molla / 4 bar | 7.2920 | 1.46090 | 0.40880 |

The gate requires final speed within 0.2 m/s of the command, displacement
over 5 m, straight heading drift below 0.02 rad and lateral drift below 0.1 m,
signed turning in both directions, and Molla heading/speed within 0.1 rad and
0.1 m/s of the corresponding same-scene Rapier result. Straight footprint
length must decrease by over 0.02 m between pressure levels. These are
regression tolerances, not calibrated real-tyre reference data. The historical
Rapier +0.60 rad figure in the plan is not reproduced by this current asset and
controller: both backends turn approximately 1.5 rad over four simulated seconds.

The imported mass snapshots differ: Molla 4916.010223 kg, Rapier 4913.449268 kg
(2.561 kg, about 0.052%). Both are checked independently; mass equivalence is
not claimed and the source of this difference remains to be audited.

Run with the slope command's test filter replaced by the name above.
`/tmp/gearbox-drive-track-final.log` records the strengthened gate passing;
ordinary release suites pass 87 tests on each backend, with six ignored
(five imported gates and one Vulkan gate). Logs:
`/tmp/gearbox-drive-track-{molla,rapier}-regression.log`.

Live CLI/render checks used the native debug viewer, the same Kubota, 1280×800,
Wayland and `GEARBOX_TERRAIN=meadow` in assistant-owned instances
`molla-field-track-gate` and `rapier-field-track-gate`. Both accepted
`machine move kubota_tractor --forward 2 --turn 0.4 --for 4s`, then a straight
4 s command, with visible tracks across grass and harvested wheat. Molla
accepted 0.5 bar before turning and 4 bar before straight driving; fresh state
subscriptions confirmed the applied values on all four wheels. Screenshots
were captured through `instance screenshot` and visually inspected:

- `/tmp/molla-field-before.png`, `/tmp/molla-field-low-after.png`,
  `/tmp/molla-field-high-after.png`;
- `/tmp/rapier-field-before.png`, `/tmp/rapier-field-after.png`,
  `/tmp/rapier-field-straight.png`.

The flat-ground preset retains a plain mesh without field cover/stamping; its
no-track screenshot is not a failed field renderer. The meadow captures prove
visible tracks, not a calibrated visual footprint-size comparison at matched
load/camera. Live wall-clock commands, asynchronous telemetry and loading are
not the deterministic four-simulated-second benchmark. The CLI move summary
can lag; fresh subscriptions were used for end-state inspection. These runs
are not performance-parity evidence. Startup still logs the pre-existing
Vulkan loader and Bevy slab allocator diagnostics on both backends; no
quarantine/non-finite diagnostics were found. All three assistant-owned viewers
(including the initial flat probe `molla-track-gate`) were explicitly stopped.

### Krampe towing and unresolved parked-support regression

Live flat-ground runs used the Kubota and `krampe_trailer.usdz` spawned as
`trailer1`, with `rear_drawbar` / `front_coupler`, teleport attach, an 8 s
`--forward 2 --turn 0.4` command, and detach. Both backends accepted these
commands and the screenshots show the trailer following the turn. Rapier's
attachment list became empty after detach and its trailer pitch read back
0.00199 rad. No rejected-step, quarantine or non-finite diagnostics appeared
in the three run logs. This alone does not establish stable detach on Molla.

Molla's low run used 0.5 bar on the tractor and the trailer's default pressure;
the high run used 4 bar on all eight tyres. Tractor pressure survived the
trailer spawn/attach and towing. The high run returned an empty attachment list
and the detached trailer still reported 4 bar on all four wheels. The low run's
post-detach host API requests timed out despite continuing physics logs and
successful file-based screenshots; that readback remains unverified. The high
run did complete API readback. All owned viewers were explicitly stopped.

**Molla tips the uncoupled trailer nose-up.** High-pressure live telemetry
already showed pitch -0.50662 rad before attachment and -0.55680 rad after
detach, later -0.45518 rad. Thus this is not isolated to joint removal or
teleport. The cause has not been established. Do not mask it by freezing the
trailer, changing its authored mass, or disabling its valid ground contacts.

The new external-asset gate `imported_krampe_parking_support` reproduces this
without any host/network, tractor, teleport or hitch. It projects all 15 real
bodies, 14 joints, four tyres and 19 colliders including the ground, runs the
production drive/service/track systems for 15 simulated seconds at 120 Hz,
and checks body finiteness/enabled state and quarantine every step. Gross-tip
limits are |roll| <0.1 rad, |pitch| <0.2 rad and parked speed <0.05 m/s; these
are regression bounds, not calibrated trailer data. The 0.2-rad pitch bound
permits the reference asset's approximately 0.11-rad nose-down parked angle.

| Backend / pressure | Final pitch (rad) | Final speed (m/s) |
| --- | ---: | ---: |
| Rapier | +0.10997 | 0.01861 |
| Molla / 0.5 bar | -0.32082 | 0.01368 |
| Molla / 1.8 bar | -0.38409 | 0.05080 |
| Molla / 4 bar | -0.42598 | 0.10532 |

All Molla cases lift the front tyres clear of the ground. The gate is
**currently failing**, intentionally retaining its support requirement.
Ordinary binary suites pass 87 tests each with seven ignored (six imported
asset gates and Vulkan); that does not override this failing acceptance gate.

```sh
GEARBOX_PHYSICS=molla \
GEARBOX_BENCH_TRAILER=/home/bresilla/data/code/OUSD/machines/usd/krampe_trailer.usdz \
CARGO_BUILD_JOBS=4 nix develop --impure -c cargo test --release \
  -p gearbox-sim --bin gearbox \
  controller::benchmark::imported_krampe_parking_support \
  -- --ignored --nocapture --test-threads=1
```

Evidence: `/tmp/gearbox-krampe-parking-final.log`,
`/tmp/gearbox-krampe-{molla,rapier}-regression.log`,
`/tmp/{molla-tow-gate,molla-tow-high-gate,rapier-tow-gate}.log` and
`/tmp/{molla-tow-low,molla-tow-high,rapier-tow}-{driven,detached}.png`.
Krampe asset SHA256:
`72a67eb19ad5d7ad981767762e1c124873fdd8d83ff98e8899b1cab52c5bf9a1`.
Next: isolate the uncoupled support dynamics against the same-scene reference,
fix the underlying fault, then repeat all towing/pressure/detach gates and
deterministic replay. These debug runs do not establish performance parity.

### Scalar stop reaction correction (Molla `1a1cf89`)

The nose-up failure above exposed a Featherstone joint-limit bug: scalar
coordinate clamping discarded relative motion without transmitting the stop's
reaction through the other connected bodies. A minimal free two-body slider
lost the entire expected -0.8 kg m/s momentum under a -8 N, 0.1 s load.
Molla now solves unilateral stop reactions with the articulation mass matrix
before the existing integration phase. Coupled active-set validation replaces
an initial fixed-iteration projection that failed a 1000:1:1000 mass test.
The fix does not freeze bodies or change assets, tyre parameters or collisions.

The final CPU suite passes 382 tests (five ignored). Tests cover force and
torque momentum transfer at lower/upper scalar stops, release, coupled impact
energy loss, translation invariance and large mass ratios. Ball/D6 multi-axis
limits and GPU joint-limit parity are not covered by this scalar CPU change.

With the final solver, all five imported Kubota gates pass. The Krampe's gross
nose-up tip is gone, but its unchanged parking gate **still fails**:

| Pressure | Pitch (rad) | Speed (m/s) | Vertical velocity (m/s) |
| --- | ---: | ---: | ---: |
| 0.5 bar | +0.13173 | 0.07754 | +0.07430 |
| 1.8 bar | +0.13663 | 0.04966 | -0.04925 |
| 4 bar | +0.13286 | 0.00296 | -0.00258 |

The low-pressure failure is mostly vertical rocking, not the previous gross
tip. It remains to be fixed against the existing <0.05 m/s requirement. Logs:
`/tmp/molla-limit-active-full.log`, `/tmp/gearbox-limit-active-imported.log`.
Ordinary selected-backend binary suites pass 87 tests each, seven ignored;
`/tmp/gearbox-limit-active-{molla,rapier}-regression.log`.

Native build passed and a fresh standalone Krampe viewer at 0.5 bar was
observed for 20 s. Final pitch +0.11513 rad, with no rejected/quarantined/
non-finite log diagnostics. The rear-view screenshot was inspected; it is
not a matched side-view comparison or tow/detach validation. Owned instance
`molla-limit-fixed` was explicitly stopped. Evidence:
`/tmp/molla-limit-fixed{.log,.png,-state.jsonl}`.

The mixed functional run's CPU-step medians were 1.018–1.056 ms. They exceed
the target, and overlapping build/test/viewer work prevents a controlled
comparison. Profile/reuse the added mass solve and repeat isolated timings;
do not claim performance parity or completed trailer acceptance from this fix.

### Commands

Use the repository Nix environment; the host Rust compiler is too old for this
Gearbox checkout. `nix develop --impure -c oslo make check` reaches the binary
but currently fails in the unchanged `tests/oxbo_transforms.rs` integration test,
which references removed `openusd::Stage`/`usd_schema` APIs. The specification's
binary test suite is separately runnable with:

```sh
nix develop --impure -c cargo test -p gearbox-sim --bin gearbox
GEARBOX_PHYSICS=molla nix develop --impure -c cargo test -p gearbox-sim --bin gearbox
```

The original 43 binary tests plus twenty-one new backend/integration checks pass
with explicit `GEARBOX_PHYSICS=rapier` and `GEARBOX_PHYSICS=molla` (64 tests each). The new
checks include real capped motor motion, mass/impulse response, stable handles,
heightfield rays/live bounds, pair filtering/contact impulses, quarantine and
its entity-report propagation, compliant frame motion and the actual
`PhysicsWorld::capture_hitch` / fixed-step capture loop.
Tyre checks cover grid friction, load readout without penalty manifolds,
opposite axle signs, configuration failure/removal, and a headless ECS scene
exercising registration, traction and elevated contact-point stamping. The ECS
tyre assertions run with Molla selected; Rapier exercises its unchanged fallback.
Pressure coverage includes backend target validation/ramping/re-registration,
paused contact preservation, brush grip, ECS readback/rejection and patch-sized
stamps. The fields library's eight tests also pass.
This does not establish any of the live tractor/hitch/PTO/slope acceptance gates.

The real Kubota asset now loads without the previous joint-cycle panic: 26
bodies, 4917 kg, four tyres and five controllers. A native Molla run of
`machine move tractor --forward 2 --turn 0.4 --for 4s` drove and steered;
captured heading rose from approximately zero to +0.365 rad (not the specified
+0.60 rad reference). Wheel stamping ran and the tractor was visually checked.
Current debug timing is approximately 5 ms per physics step, not <1 ms.
The initial zero tyre-load telemetry was traced to identical material writes
rebuilding colliders before traction/stamping consumed the previous step.
Unchanged friction/restitution/combine-rule writes now retain scene identity
and readout. Live turning loads total approximately 48.3 kN, consistent with
the 4917 kg machine's weight; acceleration and steering improve accordingly.
The last streamed sample during the same 4 s command is approximately +0.64
rad and 1.6 m/s. Later paused samples include CLI delay and are not an exact
fixed-step reference comparison.
This is partial live evidence, not a parity or full acceptance claim.
Live TF sampling confirms both engaged PTO bodies rotating. Changing hitch
position from 1 to 0 changed rear rockshaft orientation by approximately
0.966 rad and front lower-hitch orientation by 0.395 rad, with nearly constant
chassis heading. Command properties alone were not used as movement proof.
Exact PTO speeds, loaded hitches, trailer towing and the slope gate remain.
The paused teleport/resume corruption is fixed: attachment and terrain lift
use validated pose batches rather than sequential articulated subtree moves.
Writeback publishes changed backend poses even while paused and records the
resulting ECS transform. Resume batches only externally edited transforms;
unchanged bodies retain their pose and velocity. A regression runs both
backends through paused teleport, publication, gizmo-style ancestor edits,
same-frame teleport/resume and unrelated moving bodies.

The first repaired Krampe run attached, resumed and detached without rejected
steps, but towing stalled at about 0.06 m/s. Telemetry identified a second bug:
the large front axle was inferred as an extra tyre even though its joint's
actual wheel was already authored. Its fictitious footprint carried 42 kN
while the four real wheels were unloaded. Known wheel endpoints now suppress
that extra inference, with a large-axle regression test.

After both fixes, an eight-second `--forward 2 --turn 0.4` Kubota/Krampe run
reached approximately 2.05 m/s and 0.44 rad/s, with the trailer following the
turn and all four real trailer wheels supporting load. No rejected steps or
quarantine messages appear in the run log. Screenshots and JSON:
`/tmp/molla-tow-wheels-driven*`; log: `/tmp/gearbox-tow-wheels-live.log`.
The final-build detach recheck was interrupted by control-plane timeout and
the instance subsequently disappeared; do not count it as verified. The
earlier batch-only build's detach did run successfully. Full paired-backend,
pressure-extreme and deterministic towing acceptance remains pending.
Current two-machine debug timing is about 7.55 ms/step, still above target.

Strict Clippy is not green in this checkout: dependency-inclusive linting finds
existing issues in `gearbox-api`, `gearbox-fields` and vendored clouds; the
`--no-deps` binary lane finds eight existing issues outside the new tyre code
(including the Bevy deprecation warning); see
`/tmp/gearbox-pressure-controls-clippy.log`.
No lint suppressions or unrelated source fixes were added to hide those failures.

Local path dependencies currently expect Molla at `../../OUSD/molla` relative
to the Gearbox worktree root. No dependency versions were refreshed. Rapier's
`enhanced-determinism` feature must match the shared Parry feature enabled by
Molla; otherwise Rapier selects incompatible hash-set drain calls.

## Simulated sensors on sensor links (2026-09-25)

`bin/gearbox/src/sensors.rs` samples authored `role = "sensor"` links
(`specs/CONTROLLER_SPEC.md` §7.6) with Molla's GPU sensors and streams them on
`/machines/<id>/sensors/<link>`: `datapod.imu.v1` for `imu` links, chunked
`gearbox.lidar_scan.v1` sweeps for `lidar` links and row-banded
`gearbox.camera_frame.v1` color/depth frames for `camera` links. The exact
`odom`/`imu` controller telemetry is unchanged and stays on its own topics.

- The Molla crates now resolve through a `[patch."https://github.com/bresilla/molla"]`
  section to the sibling checkout (`../../OUSD/molla`), which adds
  `molla-sensors`, `molla-compute` and `molla-storage`; the git `rev` pins are
  retained for when that tree is pushed.
- The sensors adopt Bevy's render device through `molla_compute::Engine::from_device`;
  Bevy 0.19.1 and Molla share wgpu 29.0.4, so no second device or copy exists.
  The FEM islands already use the same device this way.
- `PhysicsBackend::with_molla_scene`/`molla_body_handle` expose the Molla runtime
  scene; the rig snapshots collider geometry once per scene revision
  (`SensorScene`, convex hulls and round cylinders included) and copies body
  poses/twists at each sample under the physics lock, never across a GPU wait.
  Results are collected through a readback ring on later frames, so no frame
  blocks on the GPU. Rigs rebuild on scene revision or mount changes and drop
  with their machine; a paused clock issues no samples.
- Sensor rates run on `PhysicsWorld::simulated_seconds`, not render frames.
- The machine agent's shared-memory config now allows 24 publishers; every
  message must fit `MAX_PAYLOAD_BYTES` (16 KiB), so LiDAR sweeps are split into
  `LIDAR_CHUNK_PULSES` (900) pulse chunks sharing one `sample`.
- `bin/gearbox/assets/world/sensor_yard.usda` is the tractor with an
  `imu_link` (100 Hz) and a 360×16 roof `lidar_link` (10 Hz, 60 m).

Verification, RTX 4080/Vulkan, release build, Molla backend at 120 Hz:

- `nixVulkan cargo test -p gearbox-sim --bin gearbox imu_and_lidar -- --ignored`:
  the headless GPU fixture (fixed carrier over a slab with a 10 m wall 4.5 m
  ahead) reads 9.81 m/s² along the link +Z, zero gyro, ROS-remapped
  orientation, and ranges `[inf, inf, inf, 6.364, 4.5, 6.364, inf, inf]`
  with three hits. `links::tests::sensor_links_parse_kinds_defaults_and_ranges`,
  `sensors::tests::*` and `wire::json::lidar_scan_tests` cover parsing, frames
  and the wire type.
- Live `sensors-gate` instance with `sensor_yard.usda` spawned as `sy`:
  `peer sub /machines/sy/sensors/imu_link` parked reads
  `az 9.80, ax 0.30, ay -0.12` (meadow tilt) and zero gyro; during
  `machine move sy --forward 2 --turn 0.3 --for 5s` it reads
  `gyro.vz 0.25–0.38 rad/s`, `ax 1.0–1.4 m/s²` while accelerating and
  `ay 0.3–1.0 m/s²` centripetal, matching the commanded left turn and the
  `odom` yaw rate. `peer sub /machines/sy/sensors/lidar_link -n 8` delivers a
  full sweep as seven chunks of one `sample`: the eight upward rows miss, the
  lower rows hit terrain and the tractor's own body from 0.48 m to 59.9 m.
- Pausing the clock froze both streams (the same LiDAR `sample` 1819 for
  4 s); `scene play` resumed them (1918); `clear machines` dropped the rig
  without diagnostics. Frame rate stayed 8–12 fps with and without the
  machine, so the sensors are not the render bottleneck; physics steps stayed
  0.15–0.44 ms.
- Screenshot after the drive: `/tmp/claude-1000/.../scratchpad/logs/sensor_yard.png`
  (copied to `/tmp/gearbox-sensor-yard.png`).

Camera links (`gearbox:sensor:kind = "camera"`, 160×120 at 5 Hz in the yard)
stream row-banded color and depth frames; the live centre rows read the
terrain at 13–80 m and the bottom rows the hood at 4.5–6 m. A second tractor
spawned 20.3 m from `sy` shows in the LiDAR horizon rows at 18.7–19.3 m over
a 36° sector and, after `machine move other --forward 2 --for 4s`, at
22.9–25 m about 27 columns further round, consistent with its 8 m move
(`/tmp/gearbox-sensor-sweep-{before,after}.json`,
`/tmp/gearbox-sensor-yard-two.png`). The simulator log reports each rig every
ten seconds: with IMU, LiDAR and camera together, 0.3–0.7 ms sample + 0.1–0.4 ms
collect per sample, 1 KB uploaded and 139–191 KB read back per sample, 150
frames per 10 s and zero dropped samples at 15 fps.

Not covered: the first launch of this checkout ran on `llvmpipe` because
`NVIDIA_VERSION` was unset when the dev shell was entered (see `.env.lua`);
set it before `nix develop`. SDF colliders stay invisible to sensors; sensor
noise models remain future work (`PLAN_SENSORS.md`).

### Camera colours, the live viewer and multi-camera cost (2026-09-25)

Camera links trace the Molla collision scene, not Bevy's visual meshes. Every
shape used to shade as white, so frames were grey Lambert silhouettes. Rig
builds now call `SensorScene::set_shape_colors` with one albedo per collider:

- the size-weighted mean `StandardMaterial` colour of the visible meshes under
  the collider entity, or under its nearest ancestor that has any (up to three
  levels), times the mean texel of CPU-resident RGBA8 base-colour textures;
- `GROUND_COLOR` for heightfields and entity-less colliders;
- `NEUTRAL_COLOR` otherwise.

The log line `collider colours: N from visuals, M ground, K neutral` reports
the split. `sensors::tests::collider_colours_follow_visuals_with_ground_and_neutral_fallbacks`
covers the mapping. This does **not** make camera links representative of the
rendered scene: terrain textures, grass and meshes without colliders never
reach the collision scene. That is now the `geometry` render mode; see below.

`bin/gearbox/examples/camera_view.rs` subscribes to camera links and shows the
colour (or depth) of each stream with its delivered rate and bandwidth. Run it
as `camera_view <machine> [link ...]`. It connects as the named identity
`gearbox-camera-view`: `camera_view --did` prints the did for `gearbox run
--allow <did>`. Misses (alpha 0) draw as sky, and `CAMERA_VIEW_SNAPSHOTS=<dir>`
writes each stream's latest colour frame as a PNG every three seconds.
`world/tractor_cameras.usda` mounts six 640×480, 15 Hz cameras (front, rear,
left, right, front-down, rear-down) on the tractor.

Measured on RTX 4080 (release, default meadow world, 32-second windows, fps
from `LogDiagnosticsPlugin`):

| Setup | Sim fps | Rig cost per rendered frame | Readback per frame | Dropped camera frames |
|---|---|---|---|---|
| No machine | ~12 (noisy, 6–50) | – | – | – |
| Tractor, no cameras | 11.9–12.2 | – | – | – |
| One camera | 11.3–12.3 | 0.7 ms sample + 6.3 ms collect | 2.4 MB | 0 |
| One camera, viewer subscribed | 10.8–11.0 | 0.3 ms + 2.8 ms | 2.4 MB | 0 |
| Six cameras | 6.8–11.8 | 2.9 ms + 16.3 ms | 12.6 MB | 76–120 per 10 s |
| Six cameras, viewer subscribed | 6.8–8.3 | 1.3 ms + 10.6–12.1 ms | 12.7–13.4 MB | 39–77 per 10 s |

Before any machine is loaded, Bevy's own rendering already keeps the board at
100% and about 12 fps, so the cameras' GPU trace is small by comparison. Host
work dominates: map, `f64` conversion, row-band chunking and publishing. Samples
are taken at most once per rendered frame, so a 15 Hz camera delivers about
12 Hz here. Dropped frames are readback-ring backpressure. 1280×720 links were
rejected: link parsing caps width and height at 1024.

### Bevy-rendered camera links (2026-09-25)

`gearbox:sensor:render` picks where a camera link's colour comes from. The
default is `optimized`; `full` and `geometry` are the alternatives.
`gearbox:sensor:channels` picks `color`, `depth` or `color_depth`.

`bin/gearbox/src/sensor_cameras.rs` gives each `optimized`/`full` link a
render-to-texture `Camera3d`:

- **Placement:** a child of the link prim, looking along +X with +Z up, with
  the link's vertical FOV and range as far plane.
- **When it renders:** it stays inactive until the link's rig issues a sample.
  That frame it renders once and a `Readback` copies its RGBA8 image back.
- **What it publishes:** row padding is stripped and the colour goes out
  through the same row-band chunker as Molla frames. Colour carries the
  sample number and simulated time of the rig's sample, so it pairs with the
  Molla depth.
- **Depth-only trace:** the Molla trace then runs only for depth, and not at
  all for `channels = "color"`.
- **Order:** sensor cameras order before the viewer (`order ≤ -1000`).
- **Always off by default:** each frame starts with every sensor camera
  inactive, because mara switches every camera on before each update.

What each mode draws:

- **Both modes:** the weather's exposure, AgX tonemapping, haze, ambient and
  environment light, following the daylight via `bevy_weather::camera_look`
  and `WeatherLook`.
- **`optimized` omits:** bloom, MSAA and vegetation (`gearbox_fields::NoVegetation`).
- **`full` adds:** Bloom and 4× MSAA, and draws vegetation.
- **Neither draws:** the cloud skybox, which now sits on
  `bevy_weather::SKY_LAYER` because it is rendered from the viewer's own cloud
  image. Sensor cameras clear to a flat sky colour instead.
- **Shadows:** both pay the sun's four shadow cascades; a camera cannot opt
  out without losing the sun.

Making a second `Camera3d` safe required changes to several systems:

- **Opting out of the weather:** `bevy_weather::configure_cameras` skips
  cameras with `WeatherOptOut`.
- **Following the viewer:** terrain and biome lookups exclude `SensorCamera`.
  Vegetation streaming and motes follow the highest-order camera instead of an
  arbitrary or single one.
- **Already safe:** planet tile selection and the globe require `CellCoord`,
  so only the viewer drives them.
- **Keeping the sky:** the recorder's layer sets include `SKY_LAYER`, so the
  viewer and its recordings keep the sky.

Measured on RTX 4080, release, default meadow world, six 640×480 links at a
requested 15 Hz, no subscriber. Without a machine the sim renders about 11 fps.

| Links | Sim fps | Delivered colour |
|---|---|---|
| 6 × `geometry` | 10.6–11.0 | ~10 Hz each; 15–17 ms host collect per frame, 12.8 MB read back, ~70 drops per 10 s |
| 1 × `optimized` | 10.2–10.6 | ~10 Hz; 1.2 MB read back per render, no drops |
| 6 × `optimized` | 7.8–8.3 | ~8 Hz each; all renders published |
| 6 × `full` | ~1.9 | ~4 Hz each |

Samples are taken at most once per rendered frame, so delivered rates are
capped by the sim frame rate. In `full` mode vegetation streams only around
the viewer camera, so it thins out beyond the viewer's range in sensor images.

One run with six `optimized` links hit NVIDIA Xid 32 (invalid push buffer)
about 36 s in, after a click in the window, and the device was lost. It did
not reproduce across the four-mode measurement that followed; the cause is
unknown.

### Where the camera cost goes (profiled 2026-09-25)

Profiling switches:
- `GEARBOX_RENDER_DIAGNOSTICS=1` adds Bevy's per-pass render diagnostics to the 10 s log.
- `MARA_GPU_TIMESTAMPS=1` makes mara request timestamp queries so those passes get GPU times.
- `GEARBOX_TRACE` in the `profile` build writes a Chrome trace.
- `GEARBOX_NO_SUN_SHADOWS=1` switches sun shadow maps off, for profiling only.

Findings on the RTX 4080, release, default meadow, tractor with camera links:

- **GPU-bound.** In the Chrome trace the main thread is busy only 56 → 85 ms per
  second of wall time (no cameras → six optimized), and the render thread's
  `Core3d` 22 → 40 ms per second, while frames take 80–160 ms.
- **Grass dominates the base cost.** The viewer's transparent pass, which draws
  the vegetation, takes most of each frame's GPU time. This is why the sim sits
  near 12 fps before any camera exists.
- **Sensor cameras pay per view, not per pixel.** Six optimized cameras at
  160×120 cost as much as at 640×480 (8.5–9.3 against 9.0–9.4 fps, from 12.2).
- **Sun shadows are that per-view cost.** Bevy builds the sun's four 4096²
  cascades, out to 800 m, for every active camera, whatever its range. With sun
  shadows off, six 160×120 cameras hold 11.7–11.9 fps against 12.7 with none,
  and deliver 714 rather than about 550 renders per 10 s.

Bevy 0.19 has no per-camera opt-out for directional shadows. A light's
`RenderLayers` also filter which entities cast into its maps, so hiding the sun
from sensor cameras would stop the world casting shadows. Removing the cost
needs a small Bevy change (a per-camera "no directional shadows" marker in
`bevy_light` cascades and `bevy_pbr` light preparation) or globally cheaper
shadows. Neither is applied.

### Generic sensor links: the Webots set (2026-09-25)

Every device in the Webots sensor guide now has a sensor-link kind
(`specs/CONTROLLER_SPEC.md` §7.6):
- accelerometer, gyro, inertial unit, compass and GPS;
- distance (laser, infra-red or sonar), light, position and radar;
- range finder, touch (bumper, force or force-3d), receiver and emitter;
- camera recognition.

Molla (`molla-sensors`, pinned at `61ff952`) computes every reading. Gearbox
supplies only what the environment knows:
- the site frame (north +X, up +Y, east +Z);
- each site's datum on the planet, for GPS;
- the sun and sky of the weather, for light;
- this step's contact forces, for touch;
- the emitted packets, for radio.

Readings stream as `gearbox.measurement.v1`. Controllers send packets as
`gearbox.emit_request.v1` on `/machines/<id>/emit`.

- `bin/gearbox/src/sensor_generic.rs` samples the generic kinds.
  - Distance, light and radar rays are cast on the CPU (`RigidRays`) against
    the live rigid scene. The first live run cast them with Molla's synchronous
    GPU raycaster. That made the first rig of each frame wait for the renderer's
    queued work: 77–128 ms per sample against 1.3 ms for the next rig, at
    7–9 fps. On the CPU the same rigs take 2.8 and 0.4 ms, at 12–13 fps.
  - Light uses the weather's own split. The beam is the clear-sky sun dimmed by
    `SkyLight::mean_direct`. Sky light is `0.25 × beam × sin(elevation) ×
    diffuse_gain`, a fifth of the global light under a clear sky.
- `bin/gearbox/src/sensor_radio.rs` puts every machine's receivers and emitters
  on one Molla `RadioMedium`. Infra-red line of sight uses the backend's new
  `cast_ray_filtered`, which skips both end bodies. Rapier and Molla implement
  it.
- Recognition adds a shape-index channel to the camera trace. Objects are rigid
  bodies other than the machine's own. Labels are `<machine>:<body prim>`.
- `contact_forces` resolves collider bodies before taking the Molla scene lock.
  Doing it under the lock deadlocked the first GPU test.

Verification, RTX 4080, release, Molla backend at 120 Hz:
- `sensors::tests::imu_and_lidar_sample_the_molla_scene` (`oslo make
  test-fem-gpu --filter=imu_and_lidar`) now also covers accelerometer, gyro,
  inertial unit, compass, distance, light, radar, touch, position and
  recognition. It checks each against the fixture's geometry.
- Unit tests cover:
  - `sensor_generic::tests`: GPS against the site datum, the daylight split,
    recognition records;
  - `sensor_radio::tests`: radio versus infra-red behind a wall, and range;
  - `links::tests::generic_sensor_links_parse_their_attributes`;
  - the `wire_json` measurement round trip.
- Live, `world/sensor_suite.usda` spawned as `ss`, with a second copy 20 m
  ahead:
  - **At rest:** the inertial unit's roll 0.01787 and pitch −0.00787 give
    gravity `(0.0772, 0.1753, 9.808)` in the link frame, equal to the
    accelerometer. Yaw 89.55° matches odom's 89.5° and the compass heading of
    0.45°. GPS sits 0.5 m behind odom, as authored, with zero speed.
  - **Radar:** sees the other tractor's rear wheels at 16.0 and 16.8 m and its
    chassis at 17.9 m. While driving at 1.5 m/s it reads −1.43 to −1.51 m/s
    radial speed, down to 2.4 m.
  - **Recognition:** lists `ss2:/World/Tractor_01/chassis` and both rear
    wheels at about 17 m.
  - **Radio:** a packet from `ss2`'s emitter reaches `ss` at strength 0.00289
    (18.6 m) from straight ahead, and `ss2`'s own receiver at 0.8 m.
  - **Bumper:** driven into the other machine 0.9 m off-centre, the chassis
    bumper reads 16 contacts and 10.5 kN pushing back along −X. Head-on, the
    two tractors' wheels meet first: the rear tyres stick out 0.3 m past the
    hull, so the bumper stays clear.
  - **Position:** the rear-wheel sensor follows the wheel angle, 25.9 rad
    after a 17.6 m roll.

Tyre–ground forces come from the tyre model, not contact manifolds. A touch
sensor on a wheel therefore reports only collisions with other bodies. A
sensor mounted inside its own body's collider sees that collider first. The
radar in the first draft of the suite sat on the hull's front face and saw
nothing.

### Actuator devices: the Webots set (2026-09-25)

The Webots actuators, less pen, LED, display, speaker and muscle, now run
inside Molla's rigid step (`molla_solvers::devices`, pinned at `030b965`):
- motors (rotational and linear) and brakes on revolute and prismatic joints;
- propellers;
- belts, as conveyors or tank tracks;
- connectors.

The emitter came with the radio work. Gearbox only authors, commands and
reads them (`specs/CONTROLLER_SPEC.md` §7.7).

Molla core:
- **Brakes:** `Control::joint_damping` is solved backward-Euler together
  with the drive on its DOF.
- **Effort readback:** `FeatherstoneSolver::joint_efforts()` gives every DOF's
  drive and brake effort, averaged over the step. That is motor torque
  feedback.
- **Loop reactions:** `loop_forces` returns each loop joint's reaction on its
  child. Connectors break on it.
- **Belts:** `ColliderDesc::surface_velocity` makes penalty friction drag
  contacts along a running surface.
- **Motors:** a velocity drive under Webots' P-controller, or a direct effort.
- **Propellers:** thrust and reaction torque added each substep.
- **Connectors:** link through compliant fixed loop joints.
- **Tests:** `molla-solvers --test devices`; each device checked against an
  analytic answer.

Gearbox:
- **Backend trait:** `PhysicsBackend` gains an engine-agnostic device API.
  Its defaults are "unsupported", so Rapier is untouched. `MollaBackend`
  maps it in `physics/molla/device.rs`.
- **Joints:** `PhysicsWorld::entity_to_joint` resolves authored joint prims.
- **`devices.rs`:**
  - discovers devices with the machine;
  - registers them once their bodies, joints and colliders exist;
  - applies `/actuate` commands before the physics steps;
  - publishes `actuators/<device>` after.

Verification, RTX 4080, release, `world/device_yard.usda` spawned twice as
`lead` and `tow`:
- Both machines register all 7 devices.
- **PTO motor:**
  - Velocity 30 rad/s runs at 29.98 rad/s on 0.30 N·m.
  - Position 200 rad from 594 rad turns at exactly −5 rad/s, the commanded
    speed.
  - 20 N·m of torque spins the free stub up without bound (1143 rad/s),
    Webots' documented force-control behaviour.
- **Propeller:** ω 60 rad/s gives thrust 1800 N (0.5·60²) and a reaction
  torque of 36 N·m (0.01·60²).
- **Roof belt:** 1.5 m/s, with its travel integrating.
- **Rear-wheel brakes:** 5000 N·m·s/rad while driving hold the wheels to
  −0.004 rad/s. Released brakes report zero damping.
- **Connectors:**
  - `tow`'s latched, auto-locking nose docks with `lead`'s passive tail at
    spawn. Both report the peer by name.
  - Driving `lead` loads the link to a steady 11.1 kN tensile: `tow` is parked
    on its own wheel motors, so it holds.
  - `unlock` releases the link while the peer is still present. `lead` then
    drives 3 m away and presence clears.
  - Re-latching links again as the tractors come back within tolerance.
- Full `oslo make test` passes. The only Molla failure,
  `joint_control::scalar_equality_limits_resist_effort_and_preserve_authored_motor`,
  fails identically on the previous pin.

Found on the way:
- A 1 kg box on Molla's default penalty contact chatters. Its four contact
  points' explicit damping gives `c·dt/m ≈ 4` at the 1/960 s substep, so the
  conveyor test uses a 50 kg crate.
- The ackermann controller did not reverse `lead` on `--forward=-1`. That is
  unrelated to the devices and not investigated.
