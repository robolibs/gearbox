# gearbox physical wheels — raycast → simulated-contact transition

## Goal

Replace the raycast vehicle model with physically-simulated wheels, **inside
rapier**. Today every preset vehicle rides on rapier's
`DynamicRayCastVehicleController`: wheels are downward raycasts, suspension and
tire forces are computed analytically, and the wheel colliders are explicitly
kept *off* the ground. That analytic friction model is tuned for fast, light
cars — on a heavy, slow, high-torque tractor it produces unrealistic slip. With
real wheel contact, traction becomes `normal_load × friction`, so the vehicle
slips exactly when commanded torque exceeds available grip. Slip becomes a
*tunable physical quantity* instead of a model artifact.

## Guiding principle

The change is **contained to `gearbox-physics`**, plus two new fields in
`gearbox-core`. The `BodyProxy` / `WheelsProxy` boundary in
`vehicle_physics.rs` is what makes this contained: the drive controllers
(`drive/ackermann.rs`, `differential.rs`, `omni.rs`) keep their logic — only
the *meaning* of `set_engine_force` / `set_brake` shifts from raycast-wheel
fields to joint/body torques.

Side benefit: this **converges the two vehicle representations**. The
USD-robot path in `bin/gearbox/src/controller.rs` already drives real rapier
joints (`controller.rs:2336-2450`); only the preset/spec path
(`Sim::spawn_vehicle`) uses the raycast hack. After this work both speak the
same language.

## Target architecture — the wheel rig

Each wheel becomes a real dynamic body. Per wheel, a light intermediate **hub**
body carries suspension + steering; the **wheel** body carries spin:

```text
chassis (root)
  └── hub[i]        ── joint A: prismatic (suspension, sprung)
        │                      + revolute (steer, steered wheels only)
        └── wheel[i]  ── joint B: revolute (spin, driven)
```

- **Joint A (chassis → hub):** prismatic along `suspension_dir` with a spring
  motor (rest target = 0) and travel limits. For steered wheels, also a
  revolute about the kingpin (`suspension_dir`) with a stiff position motor.
  Non-steered wheels lock the steer DOF.
- **Joint B (hub → wheel):** revolute about `axle_dir`. Drive torque is
  applied here.

Using a hub for *every* wheel keeps one uniform code path — non-steered wheels
simply lock the steer axis. The hub is light (~2–5 kg).

**Joint set:** use `MultibodyJointSet` — one multibody per vehicle, chassis as
root. Reduced coordinates give stable suspension, no joint drift, and adjacent
links auto-skip contact. Fallback if multibody setup proves fiddly:
`ImpulseJointSet` — simpler, but softer suspension.

## Phase breakdown

### Phase 0 — Core spec + scaffolding

**`gearbox-core/src/vehicle/wheel.rs` — `WheelSpec`** (`wheel.rs:834`):

- Add `pub mass: f64` — wheel mass (kg). There is no mass field today; wheels
  need real inertia now.
- Add `pub hub_mass: f64` — default ~3.0.
- Reinterpret existing suspension fields: `suspension_stiffness` /
  `suspension_damping` now feed a prismatic **spring motor**;
  `suspension_rest_length` becomes the prismatic limit midpoint;
  `max_suspension_force` becomes the motor force cap.
- Delete `friction_slip` (real contact friction replaces it). Add
  `pub tire_friction: f64` — collider friction coefficient (~1.0–1.4 for ag
  tires).
- `ChassisSpec.mass` stays, but note total vehicle mass is now
  `chassis + Σ(hub + wheel)` — presets must be re-balanced.

**`gearbox-core/src/presets/*`** — populate the new fields, re-tune. Tractor
rear wheel mass ≈ 80–150 kg.

**`gearbox-physics/src/world.rs`:**

- `wheel_groups()` now collides with `GROUND ∪ CHASSIS ∪ WHEEL` (today it
  explicitly skips ground — `world.rs:32-34`).
- Delete `wheel_raycast_groups()` — no raycasts anymore.
- Add a `PhysicsHooks` impl: a contact filter that **skips contact pairs
  belonging to the same vehicle**. Tag every collider's `user_data` with the
  `VehicleId` at spawn; the hook skips a pair when both sides carry a matching
  tag. This replaces rapier's same-body auto-skip (wheels are separate bodies
  now) and is unbounded — collision groups would cap us at 32 vehicles. The
  hook must be conservative: skip only when *both* colliders carry a matching
  tag, so USD-robot bodies (no tag) still collide normally.

### Phase 1 — The rig in `spawn_vehicle`

Rewrite `Sim::spawn_vehicle` (`sim.rs:103-247`):

1. Chassis body + cuboid collider — unchanged. Keep the explicit
   `MassProperties` (`sim.rs:134-144`).
2. Delete the mass-0 wheel cylinder colliders (`sim.rs:160-180`) and the
   `DynamicRayCastVehicleController` block (`sim.rs:211-231`).
3. For each `WheelSpec`, build:
   - a **hub body** at `chassis_connection`;
   - a **wheel body** at the suspension rest point;
   - a **round-cylinder collider** on the wheel
     (`ColliderBuilder::round_cylinder` — far more stable on a plane than a
     bare cylinder edge), oriented so the cylinder axis matches `axle_dir`,
     with `friction(tire_friction)` and the vehicle-id `user_data`.
4. Wire joints A and B into the per-vehicle multibody. Wheel inertia = analytic
   cylinder (`½·m·r²` about the axle).

**`PhysicsHandles`** (`vehicle_physics.rs:21`) becomes:

```rust
pub(crate) struct PhysicsHandles {
    pub body: RigidBodyHandle,                 // chassis
    pub wheels: Vec<WheelHandles>,
}
struct WheelHandles {
    hub: RigidBodyHandle,
    wheel: RigidBodyHandle,
    susp_joint: MultibodyJointHandle,          // joint A (prismatic)
    steer_joint: Option<MultibodyJointHandle>, // joint A steer DOF
    spin_joint: MultibodyJointHandle,          // joint B (revolute)
}
```

### Phase 2 — Proxy rewrite (keeps controllers stable)

Rewrite `WheelsProxy` / `WheelView` / `WheelCtrl` (`vehicle_physics.rs:97-175`)
— same method names, new bodies underneath:

- `set_engine_force(f)` → apply **drive torque** to the wheel body:
  `torque = f · radius`, via `add_torque` about the axle. Traction-limited slip
  now emerges from contact friction — the entire point of this work.
- `set_brake(b)` → apply a capped opposing torque, or set the spin revolute
  motor to velocity 0 with max force `b · max_brake`.
- `set_steering(θ)` → set the steer joint position-motor target.
- `steering()` → read the steer joint angle.
- `normal_forces()` → sum the normal contact impulses on each wheel collider
  (`narrow_phase.contact_pair`), `/dt`. This keeps Ackermann's weight-transfer
  open differential (`ackermann.rs:60-76`) working unchanged. Alt: read the
  suspension joint reaction impulse. May need a 1-frame low-pass — contact
  impulses are noisy.

`BodyProxy` (`vehicle_physics.rs:36-87`) — unchanged.

### Phase 3 — Step loop cleanup

In `Sim::step` (`sim.rs:533-675`):

- Delete the raycast `update_vehicle` pass (`sim.rs:630-658`) and the raycast
  in `refresh_kinematics` (`sim.rs:509-530`).
- Delete the parking-brake hack (`sim.rs:604-628`) and the `brake_gate` in
  `GroundFrame` (`drive/mod.rs:104-110`). Both exist only to mask the raycast
  controller's copysign-brake oscillation. With a real brake motor, standstill
  holding is honest. Add a small real hold torque if creep needs killing.
- `pipeline.step` — pass the new `PhysicsHooks` instead of `&()`
  (`sim.rs:672`).
- Power tick (`sim.rs:553-569`) — unchanged.

### Phase 4 — Pose readback & editor integration

- `wheel_pose` (`sim.rs:365-439`) → return the wheel body's pose directly
  (`bodies.get(wheel).position()`). The ~60 lines of kingpin-offset / airborne
  fixup math (`sim.rs:381-438`) mostly delete — real geometry handles it.
- `wheel_spin_angle` / `wheel_steering_angle` → read the revolute joint angles.
  Downstream consumers keep their API: `gearbox-viz/src/spawn.rs:281,286`,
  `gearbox-viz/src/sync.rs:17`, `gearbox-editor/src/selection.rs:249`.
- `set_vehicle_pose` (`sim.rs:285-295`) → apply the pose delta to chassis +
  all hub + wheel bodies, zeroing every velocity. Teleporting only the chassis
  would explode the joints.
- `despawn_vehicle` (`sim.rs:259-271`) → remove all hub/wheel bodies + the
  multibody, not just the chassis.

### Phase 5 — Tuning & solver

- `IntegrationParameters`: raise `num_solver_iterations`; consider 120 Hz
  physics — the viz fixed-step accumulator (`gearbox-viz/src/step.rs`) already
  supports it, just lower the step.
- Keep `ccd_enabled` on the chassis; evaluate CCD on fast-spinning wheels.
- Per-preset tuning pass: tire friction, suspension stiffness/damping, wheel
  mass, hub mass.

## Testing

- `tests/headless.rs::tractor_settles_and_drives` must still pass (settle,
  then drive > 1 m).
- New tests:
  - wheel does not tunnel through the ground;
  - wheel spin angle increases under throttle;
  - **slip test** — high throttle on a low-friction collider makes wheel
    angular speed outrun vehicle linear speed (proves real slip — the
    original complaint).

## Risks / watch-items

1. **Cylinder-on-plane jitter** — a bare cylinder edge-contacts a plane and
   rapier's contact generation gets twitchy. Mitigated by `round_cylinder`; if
   still twitchy, fall back to a sphere wheel collider (loses the contact
   patch).
2. **Multibody setup complexity** — fallback to impulse joints, documented
   above.
3. **`normal_forces` fidelity** — contact-impulse summing is noisy frame to
   frame; low-pass before feeding the open differential.
4. **Preset re-tuning is real work** — every tractor/robot preset needs a
   suspension + friction pass. Budget for it.
5. **USD path shares the world** — verify the same-vehicle contact hook only
   skips when both colliders carry a matching vehicle tag, so USD robots
   collide normally.

## Files touched

| File | Change |
|---|---|
| `gearbox-core/src/vehicle/wheel.rs` | `WheelSpec`: add `mass`, `hub_mass`, `tire_friction`; drop `friction_slip` |
| `gearbox-core/src/presets/*` | populate new fields, re-tune |
| `gearbox-physics/src/vehicle_physics.rs` | rewrite `PhysicsHandles`, `WheelsProxy`, `WheelView`, `WheelCtrl`; `BodyProxy` unchanged |
| `gearbox-physics/src/sim.rs` | rewrite `spawn_vehicle`, `step`, `wheel_pose`, `wheel_spin_angle`, `wheel_steering_angle`, `refresh_kinematics`, `set_vehicle_pose`, `despawn_vehicle` |
| `gearbox-physics/src/world.rs` | collision groups; new `PhysicsHooks` contact filter |
| `gearbox-physics/src/drive/mod.rs` | simplify/remove `GroundFrame::brake_gate` |
| `gearbox-physics/src/drive/{ackermann,differential,omni}.rs` | none — proxy contract preserved |
| `gearbox-physics/tests/headless.rs` | keep existing test, add slip test |

Untouched: `convert.rs` (rapier types stay), the USD-robot path in
`bin/gearbox/src/controller.rs` (already real joints), `gearbox-world` /
`bin/gearbox/src/world.rs` ground inserts, the `DroneController` (airborne, no
wheels).

## Suggested rollout

Land incrementally:

1. Do Phase 0–2 for one preset (`tractor_articulated` — the headless-test
   vehicle), keeping the raycast path compiled for other presets behind a
   `WheelSpec` flag.
2. Compare slip behavior against the old model.
3. Convert remaining presets.
4. Delete the raycast code — the `rapier3d::control` import,
   `DynamicRayCastVehicleController` — once all presets are migrated. The
   rapier `control` feature can then be dropped from `Cargo.toml`.
