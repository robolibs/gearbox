# Molla backend integration

`GEARBOX_PHYSICS=molla` selects the CPU Featherstone backend. The default
remains Rapier. Both implementations share Gearbox's existing body, collider,
joint and world traits; the Molla adapter is in `bin/gearbox/src/physics/molla`.

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
