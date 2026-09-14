# gearbox physical wheels — raycast to contact, in `bin/gearbox`

## Status (2026-09-14)

Phases 0–3 are in, and `contact` is the default traction of
`builtin:ackermann_cmd_vel` (the first half of Phase 4):

- physics steps at a fixed 120 Hz (`GEARBOX_PHYSICS_HZ`), at most 12 steps a  frame, planned in `First` so per-frame impulses know their time span;
- `prepare_machine_physics` turns machine self-collision off and gives every
  `wheel` link a rounded, grippy (≥ 1.1, `max`), CCD tyre, for trailers too;
- wheel motors are force-based (acceleration-based gains scale with the
  joint's inertia, near zero on a light knuckle), capped at
  `0.9 · μ · N · r` with `N` the weight over all wheels, full torque at
  0.3 rad/s error; an integral trim (±10 %) holds the commanded speed;
  passive wheels get a light motor at ground speed while driving, so an
  unloaded one still turns;
- per-wheel turn speeds: the old turn ratio sped up the inside wheels (body
  X points left); Ackermann wheels ahead of the rear axle add the longer arc;
- steer joints keep their authored USD drive gains, else force-based
  50 000 N·m/rad; steer angles use the real speed down to 0.8 m/s;
- wheels are `wheel` links or the wheel side of any wheel joint pair, so
  derived link trees (Hunter) count;
- `link.<wheel>.slip` is in `/state`; a warning names colliders that hang
  near the tyres' contact plane.

Measured 2026-09-14 on the meadow: Kubota, Fendt, Oxbo hold 2.00 m/s; the
Kubota yaws 0.44 rad/s for 0.4 with every wheel under 4 % slip; Claas and
Krampe tow on all four tyres at 2 m/s and steer against the articulation.
Physics costs ~1 ms a step with five machines.

Phase 3 (2026-09-14): `gearbox:machine:role:suspensionJoints` is read and
a non-prismatic target is a link tree warning. The Oxbo's middle wheels
hang from `susp_middle_*` bodies on vertical prismatic springs (1.5 MN/m,
40 kN·s/m, 3 cm preload), so its fixed middle axle carries load. Without an
authored `wheelBase` the steering wheelbase is the front steered axle's
distance ahead of the unsteered axle over the front multiplier (Oxbo:
1.6 m / 0.56), and every wheel's turn arc is measured from that axle. The
Kubota's front axle pivot has a damping drive. Accepted on flat ground:
Kubota and Oxbo drift 1 cm in 10 s and reach 2 m/s within 3 s; the Kubota
yaws 0.60 for 0.5 (steady), the Oxbo tracks ±0.2 at 2 m/s; its steering
limits cap it near 0.3 rad/s at 2 m/s, so 0.5 is out of its reach.

Still open: the diff drive writes chassis velocity (Phase 4, second half),
the raycast code is still there (Phase 5), the Phase 2 overrides
`maxWheelTorqueNm` / `maxPowerKw` and a load-based steering torque cap, the
slope and low-friction acceptance runs, and a USD tyre material on every
asset. The Oxbo and Krampe tyre colliders are still larger than their
meshes.

## Goal

Every machine in `bin/gearbox` already has real wheels: a USD chassis body,
knuckle and wheel bodies, revolute joints between them
(`specs/CONTROLLER_SPEC.md` §7.5), and a cylinder collider on each tyre.
None of that carries the machine. `builtin:ackermann_cmd_vel` switches the
tyre colliders to sensors every frame and rides the chassis on rapier's
`DynamicRayCastVehicleController`; the wheel joints only get a visual spin
derived from chassis motion. `builtin:diff_drive_cmd_vel` writes the twist
straight onto the chassis velocity.

This plan makes the tyres do the work: contact friction between the wheel
collider and the ground carries and propels the machine, the wheel joint
motors deliver torque, and slip becomes `torque > μ · N · r`, a physical
quantity that the asset tunes with mass, friction and torque caps instead of
a model artefact. The raycast controller stays as a per-machine fallback
until every asset drives on contact, then it is deleted.

Everything here lives in `bin/gearbox/src/controller.rs`, the vendored
`bin/gearbox/src/physics` (the rapier adapter), and the specs. The stale
`gearbox-core` / `gearbox-physics` crates are not touched.

## What exists today (the code this plan changes)

`bin/gearbox/src/controller.rs`:

- `apply_builtin_ackermann_cmd_vel` (~981): per machine and frame, computes
  `steer_targets` (`steering_joint_targets`), `wheel_targets`
  (`wheel_joint_targets`: velocity = ground speed / collider radius, with a
  turn ratio per side), `parking_brake_wheel_targets` (velocity 0 when the
  command is under 0.05 m/s), `tire_joint_pairs`, then calls
  `apply_rapier_raycast_vehicle_controller`. When that returns true the
  wheel targets are replaced by `visual_wheel_spin_targets` (soft motors,
  `WHEEL_VISUAL_*`). Finally `apply_articulation_or_impulse_joint_motors`
  writes the motors, on impulse or multibody joints.
- `apply_rapier_raycast_vehicle_controller` (~1639): `set_wheel_colliders_sensor(true)`,
  rebuilds a `DynamicRayCastVehicleController` from
  `raycast_vehicle_wheel_specs_for_controller` (hard point = wheel body
  position + `RAYCAST_SUSPENSION_REST_LENGTH`, radius from
  `body_max_collider_radius`), engine force = speed servo
  `RAYCAST_ENGINE_FORCE_GAIN_PER_REAR_WHEEL · mass_scale` plus slope
  compensation and `attach::TowedMass`, brake proportional to speed,
  fixed `WheelTuning` (stiffness 90, damping 7, `friction_slip` 24).
- `ensure_tire_grip`: forces tyre friction to `TIRE_FRICTION` (2.4, `Max`
  combine) and restitution 0. `set_wheel_colliders_friction` sets 0.05 for
  the diff drive.
- Constants (~1497): `WHEEL_DRIVE_DAMPING` 240, `WHEEL_DRIVE_MAX_TORQUE`
  6000, `WHEEL_VISUAL_*`, `STEER_STIFFNESS` 300, `STEER_DAMPING` 90,
  `STEER_MAX_TORQUE` 1200, `RAYCAST_*`.
- `hold_axle_pivots`: while the raycast carries the chassis, an
  intermediate axle pivot (Kubota `front_axle`) is held at rest.
- `guard_chassis_inertia`: replaces implausible authored inertia per axis.
- `apply_builtin_diff_drive_cmd_vel` (~1209): sets chassis `linvel` /
  `angvel`, tyre friction 0.05, `carry_wheels_with_chassis`.
- `publish_machine_controller_states`: `/state` from the chassis body.

`bin/gearbox/src/physics/rapier/colliders.rs`: a USD `Cylinder`
becomes `ColliderBuilder::cylinder` (a sharp-edged cylinder). USD drives
become joint motors (`joints.rs`, `MotorModel::ForceBased`); the sim pins
`AccelerationBased` on every joint it drives.

`bin/gearbox/src/physics/world.rs`: one rapier
step per render frame at the default `dt` (1/60 s) regardless of frame
time, `num_solver_iterations` 16, `PairFilter` hook dropping the contacts
`PhysicsFilteredPairsAPI` lists, no CCD.

Assets: `bin/gearbox/assets/tractor.usd` and `oxbo.usd` author cylinder
tyre colliders and wheel masses (110 / 200 kg, 260 kg) but no physics
material; `~/machines/usd/*.usdz` author friction 0.9 / 1.1 on the chassis.

## Target

```
ground  ⟷  wheel collider (round cylinder, μ from the asset)
                 │ revolute `roll_*`, velocity motor, torque-capped
              knuckle / axle / chassis (as authored, §7.5)
```

- The tyre collider is solid and carries load. Traction is rapier's
  contact friction. Nothing casts rays.
- The wheel joint motor is the engine: velocity target from the command,
  torque cap from the machine's grip and power, damping as the brake.
- Suspension is whatever the asset authors: nothing (rigid axle, the
  present assets), a pivoting axle (`front_axle` on the Kubota, free again),
  or prismatic suspension joints with USD drives (`role:suspensionJoints`,
  new).
- One machine never collides with itself; it collides with the ground and
  with other machines.
- Physics steps at a fixed rate independent of the frame rate.

## Phases

### Phase 0 — ground under the wheels

Physics changes that help both models and are safe to land alone.

- **Fixed timestep.** `world.rs::step_physics` runs an accumulator: `dt`
  1/120 s, at most 4 steps per frame, remainder carried. The debug build
  at 10 fps today runs physics at a fraction of real time; contacts on a
  fast tyre need the finer step more than the raycast did. Expose
  `GEARBOX_PHYSICS_HZ` for experiments.
- **Machine self-collision off.** `PairFilter` gains a second set: body
  handle → machine id. `controller.rs` fills it when a machine's bodies
  exist (same place `guard_chassis_inertia` runs). Contacts between two
  bodies of one machine are dropped; the asset's `PhysicsFilteredPairsAPI`
  lists stay honoured for anything finer. Attached slaves are separate
  machines and keep colliding with the master except through the coupling
  joint, which already disables its own pair.
- **Round tyres.** `physics::rapier::colliders`: a `Cylinder` prim whose body is
  a `wheel` link, or that carries `gearbox:collider:round = true`, becomes
  `ColliderBuilder::round_cylinder(half_h, r − b, b)` with `b = 0.05 · r`.
  A sharp cylinder edge on a plane is what makes rapier contacts twitch;
  the rolling radius stays `r`.
- **Tyre friction from the asset.** `ensure_tire_grip` stops forcing 2.4.
  It applies the USD physics material when one is bound, else
  `TIRE_FRICTION` = 1.1, `Max` combine, restitution 0. The diff-drive
  0.05 override goes with Phase 4.
- **CCD** on wheel bodies (`RigidBody::enable_ccd`) so a tyre at 10 m/s
  cannot pass a terrain triangle in one step.

Accept: the Fendt and Kubota still drive on the raycast exactly as before
(rest drift under 5 cm in 10 s, steering angles as measured on 2026-09-12),
and `GEARBOX_CONTACT_LOG=1` shows zero intra-machine contact pairs with the
tyres solid.

### Phase 1 — contact traction behind a switch

- New controller attribute `gearbox:controller:<n>:traction` (token,
  `raycast` | `contact`, default `raycast`). `ControllerSpec.traction`.
- In `apply_builtin_ackermann_cmd_vel`, when `traction == contact`:
  skip `apply_rapier_raycast_vehicle_controller` and
  `visual_wheel_spin_targets`; leave the tyre colliders solid
  (`set_wheel_colliders_sensor(false)` once); keep `wheel_joint_targets`
  as the drive, `parking_brake_wheel_targets` as the hold, the steering
  targets, and `apply_articulation_or_impulse_joint_motors`. The chassis is
  not touched by the controller at all.
- `hold_axle_pivots` runs only under `raycast`; under `contact` the axle
  pivot is the real oscillating axle.
- `attach::TowedMass` is ignored under `contact`: the trailer's weight
  reaches the tyres through the coupling joint.
- The `/state` `linear_speed_mps` stays the chassis speed; add
  `link.<wheel>.slip` values (`ω · r / v − 1`, 0 when stopped) so slip is
  observable from the bus and the Machine pane.

Accept, on `bin/gearbox/assets/tractor.usd` with `traction = contact`:
stands still (drift under 2 cm in 10 s), reaches 2 m/s within 3 s of a
`machine move --forward 2`, holds a 0.5 rad/s turn, and reports slip under
0.1 on the flat world.

### Phase 2 — torque, brake and steering from the machine, not constants

- **Torque cap per wheel** replaces `WHEEL_DRIVE_MAX_TORQUE`:
  `τ_max = 0.9 · μ · N · r`, with `N = m_total · g / n_driven` from the
  rigid-body masses (chassis plus every wheel and knuckle) and `μ` the
  tyre's collider friction. Optional overrides
  `gearbox:controller:<n>:maxWheelTorqueNm` and `maxPowerKw`
  (`τ ≤ P / ω`), both documented in `CONTROLLER_SPEC.md` §3.
- **Brake** replaces the parking-brake hack: velocity target 0 with the
  motor damping as the brake, capped at `τ_max`; `builtin:brake` in
  `services.rs` already drives the same joints and keeps working.
- **Steering torque cap** replaces `STEER_MAX_TORQUE`:
  `μ · N_front · w/2` with `w` the tyre width from the collider, so a
  loaded front tyre still turns at standstill and a light one does not
  oscillate.
- Slope: gravity does it. Delete the slope-compensation term.

Accept: the same tractor climbs the 10° slope in `world/peafield.usd` at
1 m/s, and on a `physics:dynamicFriction = 0.2` ground the wheels outrun
the chassis (`slip > 0.5`) under full throttle. That slip test is the
original complaint made measurable.

### Phase 3 — suspension and the six-wheeler

- `gearbox:machine:role:suspensionJoints` (rel[], optional): prismatic
  joints between chassis and knuckle whose USD drive (`stiffness`,
  `damping`, `maxForce`) is the spring. `physics::rapier` already turns the
  drive into a motor; the sim leaves those joints alone. `links.rs`
  validation warns when a suspension joint is not prismatic.
- The Oxbo's six wheels and rear steer axle need no code beyond Phase 2:
  `wheel_joint_targets` already covers every wheel path and
  `steering_multiplier_for_wheel_path` the rear axle. Tune its 15 t on
  contact: friction, torque cap, the guard's inertia estimate.
- The Kubota's `front_axle` pivot: limits stay ±10°, add a small USD
  damping drive to the asset so it settles.

Accept: Oxbo and Kubota drive on contact with the Phase 1 numbers.

### Phase 4 — everything on contact

- Default `traction` becomes `contact`; `raycast` stays selectable.
- `builtin:diff_drive_cmd_vel` moves to contact too: per-side wheel
  velocity targets (`v ± ω · track/2`) with the Phase 2 torque cap; delete
  `carry_wheels_with_chassis`, the 0.05 friction and the direct
  `set_linvel`. Skid steering then costs what it costs: scrubbing tyres.
- Every repo asset and `~/machines/usd/*` gets a physics material on its
  tyres and a torque cap check; record the numbers in the asset READMEs.

Accept: `scripts/tractor_trailer.py`, `scripts/oxbo_follow_points.py` and
`scripts/hunter_drive.py` run to completion on contact.

### Phase 5 — delete the raycast

- Remove `apply_rapier_raycast_vehicle_controller`,
  `raycast_vehicle_wheel_specs*`, `raycast_wheel_spec_for_path`,
  `visual_wheel_spin_targets`, `set_wheel_colliders_sensor`,
  `hold_axle_pivots`, `WHEEL_VISUAL_*`, every `RAYCAST_*` constant, the
  `traction` attribute, and the `rapier3d::control` import.
- `attach::TowedMass` goes with its only consumer.
- Specs: `CONTROLLER_SPEC.md` §3.1 (both controller descriptions), §4
  (mass scaling paragraph), §5 (tyre collider requirements: round, material,
  mass), §7.5 (axle pivot sentence), §8 (raycast assumptions). `PLAN_LATER.md`
  loses the wheels line.

## Testing

Unit tests in `controller.rs` keep covering the targets: add cases for the
torque cap arithmetic, the slip readout, and `traction` parsing. The
physics itself is verified the way this week's work was, with the sim
running and measured over the bus:

- rest drift and per-link wobble (`machine tf` samples, the `tfrel.py`
  method);
- speed reached, turn held, slope climbed (`machine state`);
- slip under throttle on the low-friction ground (`link.<wheel>.slip`);
- `GEARBOX_PHYSICS_LOG` / `GEARBOX_CONTACT_LOG` for contact pairs and
  joint gaps.

A headless harness (`run --headless`, `PLAN_LATER.md`) would let these run
in CI; until then they are scripted acceptance runs.

## Risks

- **Cylinder contact jitter.** Round cylinders and the fixed 120 Hz step are
  the mitigation; if a tyre still chatters on flat ground, a ball collider
  of the tyre radius is the fallback (loses the contact patch width).
- **Mass ratios.** A 3.8 t chassis on 130 kg wheels through impulse joints
  is what jittered this week until the inertia guard. Contact adds load on
  the same joints; the fixed step and `num_solver_iterations` 16 should
  hold, else the machine goes multibody (`ArticulationRootAPI` on the
  chassis, which `physics::rapier` already supports) with the hitch loops
  excluded, as the pipeline authors them.
- **Frame rate.** Debug builds run 10 fps with two tractors; the
  accumulator's 4-step cap means physics slows below 30 fps rather than
  spiralling. Release builds are the real target.
- **Asset tuning is real work.** Friction, torque cap, inertia and tyre
  width per asset; the pipeline should author them rather than the sim
  guessing.

## Files touched

| File | Change |
|---|---|
| `bin/gearbox/src/controller.rs` | `traction` switch, contact drive path, torque and steer caps from mass, slip readout, machine self-collision registration; later deletions |
| `bin/gearbox/src/attach.rs` | `TowedMass` unused under contact, removed in Phase 5 |
| `bin/gearbox/src/links.rs` | `role:suspensionJoints` validation |
| `bin/gearbox/src/physics/rapier/colliders.rs` | round cylinder for tyres |
| `bin/gearbox/src/physics/world.rs` | fixed timestep, per-machine pair filter, CCD |
| `specs/CONTROLLER_SPEC.md` | §3, §4, §5, §7.5, §8 |
| `bin/gearbox/assets/*.usd`, `~/machines/usd/*` | tyre physics materials, torque caps, axle damping |
