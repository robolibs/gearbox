# Contact-driven machines and towing

Torque limits of the wheels driven by `builtin:ackermann_cmd_vel`
(`bin/gearbox/src/controller.rs` `cap_wheel_torque`). Diff-drive wheels mode
and tracks have their own caps ([CONTROLLER_SPEC.md](CONTROLLER_SPEC.md),
[TRACKED_DRIVE.md](TRACKED_DRIVE.md)).

## Torque limits

Each driven wheel runs a Force-model velocity motor. Its torque cap is the
lowest of:

| Limit | Value |
|---|---|
| Grip | `0.9 × grip force × effective radius`, for a wheel with solved support (grip force > 0) |
| Unloaded budget | `wheel body mass × effective radius² × 4 rad/s²`, for a wheel without solved support |
| `gearbox:controller:<n>:maxWheelTorqueNm` | per-wheel ceiling, when authored |
| `gearbox:controller:<n>:maxPowerKw` | shared power: every cap is scaled by `min(1, P / Σ τ·max(abs(ω), 0.5 rad/s))`, with ω the wheel's spin relative to its joint partner |

- **Grip force** is Molla's tyre output (`grip_force`) for a wheel with a
  registered tyre ([TYRE_PRESSURE.md](TYRE_PRESSURE.md)). Any other wheel
  sums friction × normal impulse / dt over its solved Molla contact
  manifolds whose normal points up (normal · chassis up > 0.1).
- **Effective radius** is the tyre's loaded radius, else the radius of the
  wheel's largest collider (a cylinder's own radius, otherwise its largest
  local half-extent), else the controller's `wheelRadius`.
- The motor damping is `cap / 0.1 rad/s`, so the full cap acts at 0.1 rad/s
  of speed error.
- Contact loads include the load a hitch transfers. A trailer's resistance
  reaches the tractor through the joints; its mass is not added to the
  tractor's tyre loads.

An unloaded wheel still turns through its own joint toward its speed target;
nothing invents a ground contact or chassis force for it. Power limits torque
at speed; low-speed pulling is bounded by wheel torque and grip.

## Parking

When the machine is parked (no command and not rolling), every driven wheel's
cap is `maxWheelTorqueNm`, else 6000 N·m, whatever its load, and power does not
scale it. Wheel joints with a joint coordinate are held by a Force-model
position motor at the angle captured when parking began: stiffness
`cap / 0.01 rad`, damping `cap / 0.25 rad/s`, max force `cap`. The reference
follows the wheel when the error exceeds 0.25 rad. Other wheel joints keep a
velocity-0 motor with damping `cap / 0.005 rad/s`. A lifted wheel therefore
stops rather than spinning freely. The ground contact, not the brake torque,
decides how much the machine can hold.

## Traction control

With `tractionControl` true (default; `GEARBOX_TRACTION_CONTROL=0` turns the
default off), each supported driven wheel's target is clamped to
`ground speed ± (8 % × |ground speed| + 0.15 m/s)` and never changes the sign
of the request. Ground speed is the wheel's velocity along its rolling
direction in the support plane (axle × support normal), so it follows slopes.
Traction control cannot command rollback when the requested direction is
uphill, or release a zero-speed hold by tracking downhill wheel speed. Wheels
without solved support are not clamped.

## Passive wheels

While driving, passive wheels get a velocity motor at their ground speed with
2 % of the largest driven cap. Parked, they are held like driven wheels.

## Telemetry

The Machine panel shows, per ackermann controller, the authored drive power
and wheel torque ceiling, the powered and loaded wheel counts (loaded means
solved grip > 0, not nonzero motor torque), the total drive or holding torque
budget, and whether power limiting is active. The budget is a ceiling, not a
measurement of applied torque. No torque or power value depends on the
machine's name.

## Authoring

- List driven rolling joints in `gearbox:machine:role:poweredWheelJoints`
  (a controller's `driveWheelJoints` replaces that list when non-empty), never
  steering joints. Keep free-rolling wheels in `passiveWheelJoints`; a wheel
  should not be in both.
- Author `maxWheelTorqueNm` and `maxPowerKw` on the drive controller. Without
  them loaded wheels are limited by grip alone, so a calibrated towing vehicle
  needs a realistic power rating.
- Use wheel-output torque, not engine-shaft torque. Gearing, efficiency and
  auxiliary loads are folded into the authored values. Engine speed, clutch,
  gear changes, deformable soil and differential locks are not simulated.
- A zero command coasts by default. Servo-geared robots author
  `zeroCommand = "brake"` so the wheel motors stop the machine before the
  parking hold takes over.
- A towed trailer needs physical mass, freely rolling wheels, released brakes
  and a hitch joint ([TOOLS_SPEC.md](TOOLS_SPEC.md)). `builtin:brake` releases
  by default while the trailer is attached.
