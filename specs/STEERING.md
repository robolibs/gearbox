# Ackermann steering and wheel-speed targets

The geometric solver is `bin/gearbox/src/controller/steering.rs`; the steer
servos are in `bin/gearbox/src/controller.rs` (`steer_torque_cap`,
`set_steer_motor`, `configure_fallback_steer_motor`). It uses the
no-lateral-slip kinematic constraint of
[ROS 2 Control's steering library](https://control.ros.org/master/doc/ros2_controllers/steering_controllers_library/doc/userdoc.html).
This is differential speed allocation, not a mechanical differential or a tyre
slip-angle model; steering transients and terrain still produce slip.

## Geometry

`gearbox:controller:<n>:steeringGeometry` (default `ackermann`) selects the
path:

| Value | Path |
|---|---|
| `ackermann`, `six_wheel_ackermann`, `oxbo`, `rear_counter_half`, `counter_steer_half` | geometric solver, falling back to legacy mappings when a requirement below fails |
| `parallel`, `crab`, anything else | legacy mappings |

Skid steering is separate (`builtin:diff_drive_cmd_vel`, `builtin:tracked_cmd_vel`).

## Solver requirements

Steer joints are taken in this order, one per body pair: `:steerLeftJoint`,
`:steerRightJoint`, `:steerJoints`, then the machine's `role:steeringJoints`.
The whole machine uses the legacy path unless all of these hold:

- every listed joint resolves to a physics joint;
- each joint's axis is within 60° of chassis up (|axis · up| ≥ 0.5); the
  axis sign sets the steer sign;
- each joint's range contains 0. The range is the joint's limits intersected
  with ±min(|`maxSteerDeg`|, 85°), `maxSteerDeg` defaulting to 45; a joint
  without limits uses the bound alone;
- a pivot exists (below).

## Pivot

The pivot `p` is the mean chassis Y of the tyres that are not on a steered
knuckle. When every tyre steers, the multipliers set a virtual pivot:

```
p = (g_f * y_rear - g_r * y_front) / (g_f - g_r)
```

with `y_front` and `y_rear` the smallest and largest steer-joint Y,
`g_f = frontSteerMultiplier` (default 1) and `g_r = rearSteerMultiplier`
(default 0 for `ackermann`, −0.5 otherwise). Axles less than 0.1 m apart or
`g_f = g_r` give no pivot.

## Rolling vector

Chassis coordinates are X left, Y back, Z up. For curvature `k`, pivot `p`
and wheel position `(x, y)`, the rolling vector per unit forward speed is:

```
forward = 1 - k*x
lateral = k*(p-y)
steer = sign * atan2(lateral, forward)
wheel_omega = speed * hypot(forward, lateral) / tyre_radius
```

Steering targets use the steer joint anchors. Drive targets use the tyre
centres (wheel body positions) and the tyre's effective radius (loaded
radius, else the largest collider's radius) when it exceeds 0.05 m, else
`wheelRadius`.

## Curvature

- A bus command uses `k = ω / v` when |v| > 0.01 m/s, else 0.
- A viewer steering input `s` in [−1, 1] uses `k = |s| × k_lock`, where
  `k_lock` is the largest curvature that fits every joint in that direction.
  The solver sees the steering as it stands, not as requested: it moves
  toward the request by `maxSteerDeg` per sweep time,
  `1 s × (1 + 0.9 × softness) × (1 + 1.4 × (1 − rolling))`, where softness
  is the mean of `1 − (pressure − min) / (max − min)` over the machine's
  tyres and rolling is `min(1, |speed| / 1.5 m/s)`.
- All wheels share one curvature. When any wheel's target leaves its range,
  the curvature is reduced by bisection until every wheel fits, instead of
  clipping wheel angles one by one.

Traction control and the torque and power ceilings then limit motor effort
([DRIVETRAIN.md](DRIVETRAIN.md)).

## Servos

The steer torque cap is recomputed each frame from solved tyre loads:

```
nominal = 1.1 * machine_mass * 9.81 / wheel_count * mean_steered_width / 2
loaded  = max over steered tyres of (width * grip_force / 2)
cap     = min(1.25 * max(nominal, loaded), 100 kN·m)
```

Without the machine mass or a steered tyre, the cap is 100 kN·m.

The fallback servo is a Force-model position motor with stiffness
`cap / 0.5°`, damping `cap / (30°/s)` and max force `cap`. These are gain
references, not motion limits.

| Steer joint | Motor written |
|---|---|
| Geometric path, no `PhysicsDriveAPI` authored on the joint | fallback servo with the current cap, every frame |
| Current AngX motor is Force-model with stiffness > 0 | target only; its gains are kept |
| Anything else | fallback servo |

An authored `PhysicsDriveAPI:angular` becomes a Force-model motor only when
`physics:localRot0` equals `physics:localRot1`, so only then are its gains
kept. A `gearbox:motor:*` device on a steer joint replaces these writes every
physics step.

## Diagnostics

`GEARBOX_STEERING_DEBUG=1` logs, once per second, the pivot, the curvature,
the steering targets and actual joint angles, the wheel spin targets and
actual rates, the solved grip and an approximate ground gap per wheel. The gap
is not a contact test. Unit tests in `steering.rs` cover mirrored inner and
outer angles, a three-axle layout, shared curvature saturation, signed wheel
speeds and small steering inputs.
