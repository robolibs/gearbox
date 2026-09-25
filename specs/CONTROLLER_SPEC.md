# Gearbox controller and runtime specification

The controllers a machine authors, what each one does every frame, and the
bus API a running simulator offers. How to author the machine itself
(stage, bodies, joints, roles, link tree, wheels, devices, sensors) is in
[MACHINE_SPEC.md](MACHINE_SPEC.md); section numbers prefixed `M` refer to
it.

Source files: `bin/gearbox/src/controller.rs` (discovery, drive controllers,
agents, state), `controller/steering.rs`, `controller/parking.rs`,
`controller/traction.rs`, `controller/tracked.rs`,
`controller/wheel_forces.rs`, `services.rs` (service and work controllers,
master/slave exchange), `devices.rs`, `sensors.rs`, `load.rs`, `attach.rs`,
`crates/gearbox-api` (wire types and agents).

Drive, service and work controllers run every frame in the Bevy `Update`
schedule before the physics step, only while physics is active. Rates,
forces and speeds are SI; angles are radians unless an attribute name says
`Deg`.

## 1. Loading and discovery

| Entry | Runs machine discovery |
|---|---|
| CLI argument (`gearbox path.usd`) | yes |
| Add USD in the viewer | yes |
| `/gearbox/usd/load` with category `machine` | yes |
| `/gearbox/usd/load` with any other category (`static`, `variant`, `world`, `terrain`) | no: spawned by the gearbox-api loader as a prop, world or terrain |

Props of a machine-category `UsdLoad` (all strings):

| Prop | Effect |
|---|---|
| `path` | USD file, absolute or relative to the working directory or the bundled assets. Required. |
| `machine_id` | Replaces the id of the machine (several machines in the asset: `<machine_id>_<id>` each) and the namespace of every controller. |
| `label` | Scene label; default `machine_id`, else the file name. |
| `lat`, `lon`, `alt` | Place on Earth. Without them the `UsdLoad` x, y, z are metres north, up and east of home. `yaw_deg` turns the machine about up. |
| `datum_lat`, `datum_lon` | The machine's own datum for reported poses (with `machine_id` only); default: where it is placed. |
| `start_paused = true` | Keep physics paused after the machine is aligned to the terrain. |
| `tyre_snapshot` | JSON tyre pressures to restore; needs a fresh, explicit `machine_id` and a single-machine asset. |
| `variant.<n>` | `prim\|set\|option` variant selections. |

Physics pauses while a machine-category load aligns to the terrain and
resumes afterwards unless paused on purpose. The `/gearbox/usd/load` reply
only acknowledges the request; load failures and rejections arrive as
`machine_rejected` events.

Once the USD projects, discovery scans the projected stage (M§2). Every
machine found, valid or not, enters the controller inventory. A machine
whose discovery recorded errors (M§16) gets no agent: the errors are logged
once and a `machine_rejected` scene event carries `machine_id` and
`reason.<n>`. A valid machine gets an agent and a `machine_ready` event
(`did`, `machine_id`).

The agent is named after the enabled controller whose `commandInterface` is
`cmd_vel`, using that controller's namespace; else the first controller's
namespace (instances sorted by name); else the machine id. Namespaces
default to the machine id. Topics live under `/machines/<agent name>/`.

A variant swap rescans the machine and keeps its id. `/gearbox/usd/delete`
(or a machine load with the remove or delete flag) removes a machine by id,
controller namespace or asset label and publishes `removed`. A scene clear
with scope `all` or `machines` drops every machine, agent and session.

## 2. Controller instances

All controller attributes live on the machine prim as
`gearbox:controller:<n>:<attribute>`. An instance `<n>` exists when:

- `GearboxControllerAPI:<n>` is in the machine prim's `apiSchemas`, or
- `<n>` is `drive`, `arm` or `implement` and `gearbox:controller:<n>:type`
  is authored.

Other names without the API schema are not found. Relationship targets
under `/robot/` are rebased onto the machine prim as in M§2.2.

Attributes every type reads:

| Attribute | Type | Default | Effect |
|---|---|---|---|
| `type` | token | `builtin:unknown` | §3. An unknown type is inert. |
| `enabled` | bool | true | A disabled controller does nothing. Must be a `bool`; other types are ignored. |
| `commandInterface` | token | none | `cmd_vel` on an enabled controller makes it the one that receives the agent's twist (§4.1) and names the agent. The agent exists without it. |
| `stateInterfaces` | token[] | [] | With `pose` or `velocity` on the `cmd_vel` controller, `/state` reports that controller's state (with wheel encoders); otherwise the body pose (§9.6). Also listed in `MachineInfo`. |
| `namespace` | string or token | machine id | Agent name when this is the `cmd_vel` controller (§1). A `machine_id` load prop replaces it. |
| `body` | rel | `gearbox:machine:body` | Chassis for the drive controllers. |
| `target` | rel | none | Joint of a service controller (§5.1); function or bin link of a work controller (§6). |
| `requests` | token[] | [] | Master functions this controller wants when its machine is a slave (§8). |
| `namespacePolicy`, `updateRateHz`, `frameConvention`, `usesRoles`, `driveWheels` | | | Parsed, unused (§10). |

## 3. Controller types

| Type | Group | Section |
|---|---|---|
| `builtin:ackermann_cmd_vel` | drive | §4.2 |
| `builtin:diff_drive_cmd_vel` | drive | §4.3 |
| `builtin:tracked_cmd_vel` | drive | §4.4 |
| `builtin:copter` | drive, synthesised from a copter device | §4.5 |
| `builtin:hitch`, `builtin:pto`, `builtin:hydraulic_valve`, `builtin:joint_position`, `builtin:joint_velocity`, `builtin:brake`, `builtin:trailer_steer` | service | §5 |
| `builtin:section_control`, `builtin:rate_control` | work | §6 |
| `external:process` | process | §7 |

Author one drive controller per machine. Every enabled drive controller
runs; only the `cmd_vel` one receives commands, so a second one sees a zero
command and holds the machine.

## 4. Drive controllers

### 4.1 Commands

- The agent's current twist (§9.4) goes to the `cmd_vel` controller every
  frame: forward speed `linear.vx`, yaw rate `angular.vz` (`angular.vy` when
  `vz` is 0). When the twist is zero and a slave's granted `speed` or
  `steering` request exists, that request is used instead (§8). Keyboard and
  gamepad driving in the viewer write the same command.
- Ackermann and diff drive shape the command:
  - dead band: |v| < 0.03 m/s → 0, |ω| < 0.02 rad/s → 0;
  - clamp: ±16 m/s, ±4.8 rad/s;
  - slew: speed changes at 1.2 m/s² while speeding up and 1.6 m/s² while
    slowing or reversing; yaw rate at 1.5 rad/s²;
  - the frame time used is clamped to [1/240, 1/20] s.
- The tracked drive takes the raw command (§4.4).

### 4.2 `builtin:ackermann_cmd_vel`

Wheeled machines with steered axles. The chassis rides on Molla pressure
tyres (M§8); wheel joint motors are the engine and brakes, capped by what
each tyre grips. Torque limits are also described in
[DRIVETRAIN.md](DRIVETRAIN.md), the steering solver in
[STEERING.md](STEERING.md).

| Attribute | Type | Default | Use |
|---|---|---|---|
| `driveWheelJoints` | rel[] | [] | Driven wheels; replaces the machine's powered role when not empty. |
| `passiveWheelJoints` | rel[] | [] | Passive wheels, with the machine's passive role. |
| `wheelJoints` | rel[] | [] | Treated as passive (idle rolling, parking); driven only in the last-resort case below. |
| `frontLeftWheelJoint`, `frontRightWheelJoint`, `rearLeftWheelJoint`, `rearRightWheelJoint` | rel | none | Tyre pairs; all four driven only in case 3 below. |
| `steerLeftJoint`, `steerRightJoint`, `steerJoints` | rel, rel, rel[] | none | Steering joints, merged with the machine's steering role. |
| `steeringGeometry` | token | `ackermann` | §4.2.6. |
| `maxSteerDeg` | float | 45 | Steer clamp; the solver caps it at 85. |
| `wheelBase` | float, m | derived, else 2.37 | Legacy steering: the steer angle from a twist and the turn rate behind legacy wheel speeds. Derived: the distance from the unsteered axle line (mean of non-steered tyres) to the frontmost steered tyre, if within 0.3–20 m, divided by the absolute `frontSteerMultiplier` (at least 0.1). |
| `trackWidth` | float, m | 1.5675 | Default for the two below. |
| `frontTrackWidth` | float, m | `trackWidth` | Legacy inner/outer steer angles. |
| `rearTrackWidth` | float, m | `trackWidth` | Legacy wheel speeds when a wheel's offset cannot be measured. |
| `wheelRadius` | float, m | 0.45 | Radius when the measured one is ≤ 0.05 m. |
| `frontSteerMultiplier`, `middleSteerMultiplier`, `rearSteerMultiplier` | float | per geometry | Legacy per-axle multipliers; front and rear also place the solver's virtual pivot when every axle steers. |
| `frontLeftSteerDifferentialDeg`, `frontRightSteerDifferentialDeg`, `rearLeftSteerDifferentialDeg`, `rearRightSteerDifferentialDeg` | float | 0 | Legacy per-corner offsets. |
| `maxWheelTorqueNm` | float | none | Per-wheel torque cap; parking torque (6000 when unset). |
| `maxPowerKw` | float | none | Power shared by the driven wheels. |
| `tractionControl` | bool | true (`GEARBOX_TRACTION_CONTROL=0` makes the default false) | §4.2.4. |
| `zeroCommand` | token | `coast` | `brake` makes a zero command brake to rest instead of coasting. |

Without any resolvable wheel joint pair it warns once
(`no wheel joint pairs found`) and cannot drive.

#### 4.2.1 Driven wheels

First non-empty of:

1. the controller's `driveWheelJoints`;
2. the machine's `role:poweredWheelJoints`;
3. with `steeringGeometry = ackermann`: the four corner relationships;
4. the controller's `wheelJoints` plus the powered role.

Passive wheels: the machine's passive role, the controller's
`passiveWheelJoints` and `wheelJoints`.

Motor writes go to every joint between the pair's two bodies, with each
registered tyre's drive sign (M§8.3).

#### 4.2.2 Wheel speed targets

- With the steering solver: `v × |rolling vector| / r` per wheel, where the
  rolling vector follows from the turn curvature and the wheel centre.
- Legacy: `v` scaled by the wheel's lateral offset in the turn
  (`1 − ω·x / v`, clamped 0.25–1.75, with `ω = v·tan δ / wheelBase`), and
  combined with the wheel's distance ahead of the unsteered axle line
  (except for `parallel` and `crab`).
- `r` is the tyre's loaded radius, else the collider radius, else
  `wheelRadius` (M§8.3).
- An integral trim holds the commanded ground speed: while |v| ≥ 0.05 and the
  speed error is under 40 % of v, the trim grows at 1.5 × error per second,
  limited to ±(0.1·|v| + 0.05) m/s. It resets below 0.05 m/s.

#### 4.2.3 Torque caps

Per driven wheel, and per passive wheel while parked:

| Wheel | Budget |
|---|---|
| parked | `maxWheelTorqueNm`, else 6000 N·m |
| unsupported (tyre reports no grip) | `wheel mass × r² × 4 rad/s²`, at most `maxWheelTorqueNm` |
| supported | `0.9 × tyre grip force × r`, at most `maxWheelTorqueNm` |

- `maxPowerKw`: all budgets are scaled by one factor so that
  `Σ τ · max(|ω_rel|, 0.5 rad/s) ≤ P`, with `ω_rel` the wheel's spin relative
  to the body it is jointed to. Not applied while parked.
- The motor is a Force-model velocity drive with `max force = cap` and
  `damping = cap / 0.1 rad/s` (`cap / 0.005 rad/s` while parked).
- The machine's drive panel shows the budgets, the number of driven and
  supported wheels and the power scale. They are limits, not measured torque.

#### 4.2.4 Traction control

When on, the target of every supported driven wheel is clamped to its
ground speed ± (0.08·|ground speed| + 0.15 m/s), converted by its radius. It
never reverses the requested direction and keeps a zero target at zero.

#### 4.2.5 Neutral, coasting and parking

- **Neutral**: the raw request (before shaping) is below 0.05 m/s and
  0.02 rad/s. **Rolling**: |forward speed| > 0.25 m/s.
- `zeroCommand = coast` (default), neutral and rolling: every wheel, driven
  and passive, is only rolled at its ground speed with zero torque, so the
  machine free-wheels. The shaped command follows the real speed, so the
  next command picks up from it.
- `zeroCommand = brake`, neutral and rolling: the driven wheels follow the
  shaped command down to rest (1.6 m/s²).
- Neutral and not rolling: **parked**. Driven and passive wheels are held.
  Each parked wheel joint with a joint coordinate gets a Force-model position
  hold at the angle captured when parking began: stiffness `cap / 0.01 rad`,
  damping `cap / 0.25 rad/s`, max force `cap`. The hold point follows the
  wheel when the error exceeds 0.25 rad (the wheel slips). Loop-closure
  wheel joints keep the zero-velocity motor.
- While driving, passive wheels are idle-rolled at their ground speed with
  2 % of the largest driven cap.

#### 4.2.6 Steering

Geometries:

| `steeringGeometry` | Steering | Legacy mapping |
|---|---|---|
| `ackermann` | solver | inner/outer split on a left/right pair |
| `six_wheel_ackermann`, `oxbo`, `rear_counter_half`, `counter_steer_half` | solver | per-axle multipliers front 1, middle 0, rear −0.5 (1 when the path names no axle) |
| `parallel` | legacy | multiplier 1 on every axle |
| `crab` | legacy | multiplier 1, rear −1 |

Legacy multipliers are overridden per axle by
`front/middle/rearSteerMultiplier`; the axle comes from the joint path
(M§17).

**Solver** (`controller/steering.rs`):

- Joints: `steerLeftJoint`, `steerRightJoint`, `steerJoints` and the steering
  role, one per body pair.
- It falls back to legacy for the frame when any listed joint does not
  resolve, an axis is more than 60° from chassis up, a joint's limits
  (clamped to ±`maxSteerDeg` ≤ 85°) exclude 0, a non-steered tyre's centre
  cannot be found, or, when every axle steers, the steered wheels span less
  than 0.1 m or the front and rear multipliers are equal.
- Pivot: the mean chassis-Y of the non-steered tyres. When every axle steers,
  a virtual pivot `(f·rear − r·front)/(f − r)` from the front multiplier `f`
  (default 1) and rear multiplier `r` (default 0 for `ackermann`, −0.5
  otherwise).
- One curvature for all wheels: `ω / v` from the bus (0 when |v| ≤ 0.01), or
  the gamepad's normalised steering times the largest feasible curvature. It
  is bisected down until every wheel angle fits its joint limits. Each
  wheel's angle is `atan2(k·(pivot − y), 1 − k·x)`, signed by its joint axis.
- Every steered wheel shares the same turn centre; wheel speeds follow
  §4.2.2.

**Legacy** (`parallel`, `crab`, or the solver off): the centre angle is
`atan(wheelBase · ω / v)` with |v| raised to at least 0.8 m/s, clamped to
±`maxSteerDeg`. The first non-empty of:

1. `ackermann`: inner/outer angles (from `wheelBase` and `frontTrackWidth`) on
   `steerLeftJoint`/`steerRightJoint`, else on the first steering-role paths
   containing `left` and `right`;
2. the centre angle on `steerLeftJoint`/`steerRightJoint`;
3. `crab`, `parallel` and the counter-steer tokens: every steering-role joint
   at centre × axle multiplier + differential degrees (scaled by
   |centre| / max);
4. the centre angle on the role's left/right paths;
5. the centre angle on every `steerJoints` and role joint.

**Sweep**: the steering angle a command asks for is reached at a limited
rate: lock to lock takes `1 s × (1 + 0.9·softness) × (1 + 1.4·(1 − min(|v|/1.5,
1)))`, with softness the mean over tyres of `1 − (p − p_min)/(p_max − p_min)`.
This applies to the legacy angle and to gamepad steering fed to the solver;
bus commands reach the solver as a curvature directly.

**Servos**: every joint between a steered pair gets a position target.

- A joint whose motor is a Force-model drive with stiffness > 0 keeps its
  gains. That is an authored `PhysicsDriveAPI:angular` with `targetPosition`
  and `localRot0 == localRot1` (M§6.4).
- Otherwise the runtime servo: Force model, stiffness `cap / 0.5°`, damping
  `cap / (30°/s)`, max force `cap`, with
  `cap = min(1.25 × max(1.1·m·g/N · w̄/2, max(w·grip/2)), 100 kN·m)` over the
  steered tyres (`m` machine mass, `N` wheel count, `w` tyre width, `grip`
  the tyre's grip force), and 100 kN·m before any load is known. With the
  solver, joints without any authored drive get it every frame.

#### 4.2.7 State

Every frame while the body resolves: body position, heading, roll, pitch,
speed (length of the body's 3-D velocity, unsigned), yaw rate (world
vertical), and one encoder per driven wheel (spin relative to the chassis and
its integral), in driven-wheel order.

### 4.3 `builtin:diff_drive_cmd_vel`

Skid-steer and differential machines.

| Attribute | Type | Default | Use |
|---|---|---|---|
| `driveMode` | token | `chassis` | `chassis` or `wheels`. |
| `wheelRadius` | float, m | 0.1 | Radius when the measured one is ≤ 0.05 m. |
| `maxWheelTorqueNm` | float | 6000 | `wheels` mode torque per wheel. |
| `maxPowerKw` | float | none | `wheels` mode power. |
| `driveWheelJoints` | rel[] | [] | `wheels` mode, added to the powered role. |
| `passiveWheelJoints`, `wheelJoints`, corner joints | | | Visual spin in `chassis` mode; tyre pairs. |

**`chassis` mode**: every frame the shaped twist is written onto the chassis
body as its horizontal velocity (along the chassis forward) and world yaw
rate, keeping its vertical velocity. Wheel colliders get friction 0.05 with
combine rule Min, wheel bodies are carried at the velocity their place on the
chassis implies, and every listed wheel joint gets a visual spin motor
(Acceleration model, damping 60, 350 N·m) at the side's ground speed. On
registered ground the tyre model takes friction from the ground collider, so
the 0.05 applies only to contacts with other colliders.

**`wheels` mode**: the powered role plus `driveWheelJoints` (one per body
pair) get Force-model velocity motors at `(v − ω'·x) / r`, `x` the wheel's
lateral offset with its sign forced by the side hint in the joint path
(M§17). No grip cap and no traction control.

- Yaw feedback while |ω| ≥ 0.001: error = ω − measured yaw rate about chassis
  up; trim += 3 × error × dt, limited to ±4 rad/s;
  `ω' = clamp(ω + 1.5 × error + trim, ±4 rad/s)`.
- Cap `maxWheelTorqueNm` (6000); `maxPowerKw` scales all caps by one factor
  so `Σ cap · max(|target ω|, 0.5) ≤ P`, except at rest.
- Damping `cap / 0.1 rad/s`; at rest (shaped command under 0.05 m/s and
  0.02 rad/s) `cap / 0.005 rad/s`, which holds the machine.

State (both modes): body pose, speed (3-D length, unsigned), yaw rate; no
encoders. Steering and traction attributes are not read.

### 4.4 `builtin:tracked_cmd_vel`

Torque-driven continuous tracks on Molla track motors; see
[TRACKED_DRIVE.md](TRACKED_DRIVE.md).

| Attribute | Type | Default | Use |
|---|---|---|---|
| `trackWidth` | float, m | **required** | Distance between the belts. |
| `maxWheelTorqueNm` | float | 300 | Torque cap of each track motor. |
| `maxPowerKw` | float | 10 | Each motor gets `maxPowerKw × 500 W`. |
| `body` | rel | machine body | Chassis. |

- Needs two track definitions (M§11) and `trackWidth > 0`; otherwise it
  warns, removes its track motors and publishes no state.
- No dead band or slew. A non-finite command is zero. Yaw is clamped to
  ±2 rad/s.
- While both sprockets touch the ground and |ω| ≥ 0.001: error = ω − world
  yaw rate, trim += 3 × error × dt (±6), ω += 2 × error + trim.
- Belt speeds `v ∓ ω·trackWidth/2`, with v clamped to ±3 m/s and both scaled
  down so neither exceeds 3 m/s.
- Carrier-link values set friction, slip damping and speed gain (M§11).
  Roller links with `rolling_radius_m` get a Force-model velocity motor at the
  belt speed over their radius (damping 1, 2 N·m).
- Writes `track_travel_m`, `track_speed_mps`, `track_torque_nm`,
  `track_force_n`, `track_normal_load_n`, `track_contacts` on the carrier
  link; animates the treads from belt travel.
- State: body pose, signed forward speed, yaw rate, one encoder per track
  (travel / radius, angular velocity).

### 4.5 `builtin:copter`

Not authored. Every copter device (M§12) is listed in `MachineInfo` as a
controller of this type, named after the device, with `commandInterface =
cmd_vel` and state interfaces `pose`, `velocity`.

- While a session is **claimed** on the machine, every frame the session
  twist becomes the copter's velocity command: `linear.vx` forward,
  `linear.vy` left, `linear.vz` up, `angular.vz` yaw rate, in the heading
  frame. This overrides `/actuate` copter commands every frame.
- When the session ends, the copter gets one hover command (zero velocity).
- A twist sent with session 0 while nobody holds the machine does not fly it.
- `gearbox machine move <machine> --forward --left --up --turn` drives it.

## 5. Service controllers

Each drives joints with a motor and is commanded with `/cmd` (§5.4).

### 5.1 Joints

| Type | Joints |
|---|---|
| `builtin:hitch`, `builtin:pto`, `builtin:hydraulic_valve`, `builtin:joint_position`, `builtin:joint_velocity` | `target`, else the first `role:toolJoints` entry |
| `builtin:brake` | the controller's `wheelJoints`, else `role:brakeJoints`, else the powered and passive roles |
| `builtin:trailer_steer` | the controller's `steerJoints`, else `role:steeringJoints` |

A controller without a joint warns once and does nothing. A joint prim must
join two bodies. When a constraint joint and a tree joint join the same two
bodies, the constraint joint is driven. Service controllers run every frame
for machines that have an agent.

### 5.2 Actuation

| Request | Joint with a motor device (M§12) | Joint without |
|---|---|---|
| position | device `Position` command: the device's PID, speed, acceleration, limits and `maxForce` apply | Acceleration-model position servo: stiffness 4000, damping 400, max force 50 kN |
| velocity | device `Velocity` command, clipped to the device's `maxVelocity`, up to its `maxForce` | Acceleration-model velocity servo: damping 200, max force per type (below) |
| brake | the servo is written and the device replaces it every step: no effect | Acceleration-model zero-velocity servo: damping 400 × level, max force 50 kN × level |

Acceleration-model gains act per unit of joint inertia. `builtin:brake` does
not use Molla's joint-damping brake (`gearbox:brake:damping`): on tyred
wheels on a rocking bogie that brake feeds energy into the trailer.

### 5.3 Types

| Type | Keys (default) | Writes |
|---|---|---|
| `builtin:joint_position` | `position` or `value` 0..1 (0); `range` (1, rad or m) | position `position × range` |
| `builtin:hitch` | as `joint_position` | as `joint_position`. `float` and `draft` modes are not implemented. |
| `builtin:joint_velocity` | `velocity` (0) | velocity servo, 2000 N·m. Without `velocity`, a joint that is this machine's coupler `ptoJoint` turns with the master's first engaged PTO while attached. |
| `builtin:pto` | `rpm` (540, clamped 0–1200), `engaged` (false) | velocity `rpm·2π/60` when engaged, else 0; 150 N·m |
| `builtin:hydraulic_valve` | `flow` or `value` (0, clamped −1..1), `rate` (0.5) | velocity `flow × rate`; 50 kN |
| `builtin:brake` | `level` or `value` (0..1; default 1 when not attached to a master, 0 when attached) | brake servo |
| `builtin:trailer_steer` | `angle_rad`; `maxSteerDeg` attribute (35) | position: `angle_rad`, else the negative of the articulation angle to the master (master heading − own heading, wrapped to ±π) while attached, else 0; clamped to ±`maxSteerDeg` |

Flags (`engaged`) are true for `1`, `true`, `on`, `yes`.

### 5.4 Commands and link values

- `/cmd` with props `controller=<instance>` and `key=value` pairs sets
  controller props; they merge over time. The command's numeric `value`
  field becomes the `value` prop when none is given. Commands for a
  controller that is not a service controller are dropped.
- Each numeric prop (`true`/`on`/`yes` → 1, `false`/`off`/`no` → 0) is also
  written as a named value of the link the joint moves (the link whose parent
  joint it is).
- Every frame a controller's props are overlaid with that link's named
  values. So `gearbox machine set-value boom position 0.8` moves the boom
  without naming a controller, and an authored `gearbox:value:range` on the
  link sets the range (M§15.2).

### 5.5 Slave joints bound to the master

While a machine is attached as a slave, on its coupler (M§14):

- `gearbox:coupling:ptoJoint`, unless one of the slave's controllers drives
  it, turns at the master's first engaged PTO speed (150 N·m);
- `gearbox:coupling:valveJoints[n]`, unless driven by a slave controller,
  moves at master valve n's `flow × 0.5` (50 kN).

Motor devices on those joints are commanded as in §5.2.

### 5.6 Conflicts

- `builtin:brake` and a drive controller on the same wheel joints both write
  the joint motor each frame with no defined order; the result is undefined.
  With its default level 1 when unattached, `builtin:brake` on a machine with
  its own drive holds that machine's wheels. Use it on trailers.
- The ownership rule (M§12.4) keeps motor devices off drive and suspension
  joints; nothing keeps service controllers off them.

## 6. Work controllers

They act on element links (M§15) through named values, and accumulate only
while their machine is attached: the ground speed is the master's (a nested
slave uses its master's inherited speed when its own master reports none).
Set their inputs with `/cmd link=<link> name=<Name> value=<v>` or
`gearbox machine set-value`.

**`builtin:section_control`**

- Function: the `target` link if it is a `function` element, else the first
  `function` link; sections: `section` links whose parent is that function.
- Control is on while the function's `SectionControlState` (1) > 0.5. A
  section works while control is on, its `SetpointWorkState` > 0.5 and the
  speed > 0.05 m/s; its `ActualWorkState` is set to 1 or 0.
- The function's `ActualWorkState` is 1 while any working width exists.
  `TotalArea` grows by `speed × width × dt / 10000` (ha) and
  `EffectiveTotalDistance` by `speed × dt` (m) while working, where width is
  the sum of the working sections' `ActualWorkingWidth`.

**`builtin:rate_control`**

- Bin: the `target` link if it is a `bin` element, else the first `bin`
  link. Width: the sum of `ActualWorkingWidth` over every `section` link with
  `ActualWorkState` > 0.5.
- Applying while width > 0, `ActualVolumeContent` > 0 and speed > 0.05 m/s:
  `ActualVolumePerAreaApplicationRate` equals
  `SetpointVolumePerAreaApplicationRate` (l/ha), else 0; the content drains
  by `rate × speed × width × dt / 10000` (l), not below 0.

## 7. `external:process`

Spawns an executable that talks to the bus itself. Denied by default.

| Attribute | Type | Default | Use |
|---|---|---|---|
| `executable` | string or token | none | Canonicalised; must lie under an allowlisted directory. |
| `args` | string[] or token[] | [] | Arguments. |
| `transport` | token | `agentio` | Passed to the child. |

- Requires `GEARBOX_ALLOW_USD_CONTROLLER_PROCESS=1` (or `true`, `yes`) and
  `GEARBOX_CONTROLLER_ALLOWLIST=/abs/dir[:/abs/dir]` at launch.
- The child's environment gets `GEARBOX_MACHINE_ID` (the machine id),
  `GEARBOX_CONTROLLER` (the instance) and `GEARBOX_TRANSPORT`.
- A status per controller (`running`, `exited: …`, `blocked: …`,
  `spawn failed: …`) is shown in the viewer's machine controllers pane.
- The child is killed when its controller is disabled or its machine is
  removed. A child that exits is started again on the next frame.

## 8. Masters and slaves

Attaching, couplings and routing are in [TOOLS_SPEC.md](TOOLS_SPEC.md). The
controller side:

- While attached, a slave's agent refuses `claim` and `cmd_vel`; its session
  is dropped. Commands reach it through the master: `/cmd` on the master
  with `tool=<slave id>` is forwarded to the slave without the `tool` prop.
- Every frame each slave receives its master's state: ground speed (the
  master's reported speed, §4.2.7), heading, roll, pitch, `builtin:hitch`
  positions, `builtin:pto` rpm and engagement, `builtin:hydraulic_valve`
  flows in instance order. The work controllers use the speed,
  `builtin:trailer_steer` the heading, bound joints and `joint_velocity` the
  PTO and valves; hitch positions, roll and pitch are carried but unused.
- A slave asks its master with `/cmd` props `request=<token>` and a value.
  The slave must be attached and the master's `gearbox:machine:grants` must
  contain the exact token:

| Token | Effect on the master |
|---|---|
| `speed`, `steering` | Forward speed and yaw rate the master drives while its own session twist is zero. |
| `hitch:<instance>` | That `builtin:hitch` controller's `position`. |
| `pto:<instance>` | That `builtin:pto` controller: `rpm` = the absolute value, `engaged = value > 0`. |
| `aux_valve:<n>` | The n-th `builtin:hydraulic_valve` (from 0, instance order): `flow`. |

- The `requests` attribute of slave controllers does not gate this. At
  attach time, requests the master does not grant are listed as `denied` in
  the master's tools records and logged.
- The master's `/state` carries each attached slave's service controller
  props as `tool.<slave>.controller.<instance>.<key>`.

## 9. Runtime API

### 9.1 Transport

The simulator hosts one agentio agent (peerbus transport) for the scene and
one per machine. Every message is a `DatapodMsg` envelope around a datapod
named `gearbox.<type>.v1` (or `datapod.<type>.v1` for the shared robot
types). Rust definitions: `crates/gearbox-api/src/wire`; Python mirror:
`examples/python/gearbox_client.py`; CLI: `gearbox` (`docs/CLI.md`,
`gearbox api topics`). A running host writes
`$XDG_RUNTIME_DIR/gearbox/<name>.json` with its `did:key`, endpoint address
and pid.

### 9.2 Host topics

| Topic | Mode | Request → response |
|---|---|---|
| `/gearbox/info` | req/res | `ping` → `host_info` |
| `/gearbox/scene/clock` | req/res | `clock_command` (get, pause, play, toggle, shutdown, step) → `clock_state` |
| `/gearbox/scene/clock/state` | pub/sub | `clock_state` |
| `/gearbox/scene/clear` | req/res | `clear_request` (scope all, machines, props, markers; `pause_clock`) → `status` |
| `/gearbox/scene/list` | que/ans | `list_query` → `scene_object`… |
| `/gearbox/scene/events` | pub/sub | `scene_event`: loaded, pose, harvested, removed, machine_ready, attached, detached, machine_rejected |
| `/gearbox/usd/load` | req/res | `usd_load` → `status` |
| `/gearbox/usd/delete` | req/res | `usd_ref` → `status` |
| `/gearbox/marker/set`, `/gearbox/marker/delete` | req/res | `marker_set` / `marker_ref` → `status` |
| `/gearbox/select` | req/res | `selection` → `selection` |
| `/gearbox/machines/list` | que/ans | `ping` → `machine_ref`… (props `did`, `kind`, `machine_id`, `addr`) |
| `/gearbox/access/grant` | req/res | `grant_request` (prop `did`) → `status` |

### 9.3 Machine topics

Under `/machines/<agent name>/`:

| Leaf | Mode | Payload |
|---|---|---|
| `info` | req/res | `ping` → `machine_info`: `controller.<n>.instance/type/command_interface/state_interfaces`, `machine_id`, `kind`, `did`, `addr`, `link_count`, `links_derived`, `attached_to`, `tools`, `base_link` |
| `claim` | req/res | `claim_request {take, hold_ms}` → `claim_response {session, code}` |
| `release` | req/res | `session_ref` → `status` |
| `session` | req/res | `ping` → `session_info` |
| `cmd_vel` | req/res | `twist_cmd {session, twist}` → `status` |
| `cmd` | req/res | `controller_command {session, value, element, props}` → `status` |
| `state` | pub/sub | `machine_state` (§9.6) |
| `odom` | pub/sub | `datapod.odom.v1`, the odometry of `state` |
| `imu` | pub/sub | `datapod.imu.v1` machine IMU (§9.6) |
| `gnss` | pub/sub | datapod `Gnss` |
| `encoders` | pub/sub | datapod `WheelEncoders` |
| `turn_radius` | pub/sub | datapod `TurnRadius` |
| `links` | que/ans | `ping` → `link_record`… |
| `tf` | pub/sub | `link_pose` |
| `tools` | que/ans | `ping` → `attachment`… |
| `tools/attach`, `tools/detach` | req/res | `attach_request` / `detach_request` → `status` |
| `sensors/<link>` | pub/sub | per sensor kind (§9.7) |
| `emit` | req/res | `emit_request` → `status` |
| `actuate` | req/res | `actuator_command` → `status` |
| `actuators/<device>` | pub/sub | `measurement` (§9.8) |

Sensor and actuator topics register on their first publish.

### 9.4 Sessions

- `claim` grants a new session id when nobody holds the machine, or when the
  holder has been silent for 60 s, or with `take = 1`; otherwise it answers
  `BUSY` with the holder. `hold_ms` defaults to 500. An attached slave
  refuses (`REFUSED`, prop `attached_to`).
- `cmd_vel` is accepted with the holder's session id, or with session 0 while
  nobody holds the machine. Another id while held: `BUSY`; a non-zero id
  while nobody holds it: `REFUSED`.
- `cmd` is accepted under the same rule. A `cmd` with a `tf` prop needs no
  session.
- `emit` and `actuate` need no session.
- With a session, silence longer than `hold_ms` zeroes the twist and 60 s of
  silence releases the session. A twist sent with session 0 has no timeout:
  it stays until replaced.
- `release` with the holder's id or 0 frees the machine and zeroes the twist.
  Removal, a scene clear and attaching drop sessions.

### 9.5 `/cmd` props

| Props | Effect |
|---|---|
| `tf=on` / `off` (or `1`, `true`) | Starts or stops `/tf`. |
| `tool=<slave id>` + anything | Forwarded to that attached slave (§8). |
| `tyre_pressure_scope=all` / `axle:<n>` / `wheel:<link>` + `value` | Sets target tyre pressure in gauge bar (TYRE_PRESSURE.md). |
| `request=<token>` + `value` | Slave request (§8). |
| `link=<link>`, `name=<Name>`, `value` | Sets a named value of a link (§5.4, §6). |
| `controller=<instance>` + keys | Service controller command (§5.4). |

The first matching row in this order handles the command. `value` is the
`value` prop, else the command's numeric `value` field. The `element` field
is not read.

### 9.6 State, odometry and derived streams

Published every frame for every machine with an agent.

- Source: the `cmd_vel` controller's state when it lists `pose` or `velocity`
  in `stateInterfaces` and has produced state (props then include
  `controller`); otherwise the body of `gearbox:machine:body` (else the base
  link), with horizontal speed and no encoders.
- `machine_state`: `odom` (pose and twist), `heading_rad`, `roll_rad`,
  `pitch_rad`, `session` (current holder's id), props.
- Pose: east-north-up metres, in the machine's own datum when it has one
  (§1), else the site frame. Heading 0 faces east, counter-clockwise positive;
  the orientation is the heading as a yaw quaternion. Roll is positive right
  side down, pitch positive nose down.
- Twist: `linear.x` = the controller's speed (unsigned for Ackermann, diff
  drive and the body fallback; signed forward speed for tracks),
  `angular.z` = yaw rate.
- Props: `machine_id`, `attached_to`, `tools` (comma-separated slaves),
  `link.<link>.<Name>` for every named value, `tool.<slave>.controller.…`
  (§8), `controller`, and `lat`, `lon`, `alt`, `ecef`, `datum` when the
  machine is placed on the globe.
- `imu`: yaw rate as angular velocity z, the change of speed per physics step
  as linear acceleration x, the yaw quaternion.
- `gnss`: WGS84 fix and compass bearing `(π/2 − heading) mod 2π`.
- `encoders`: when the controller reports any (Ackermann driven wheels,
  tracks).
- `turn_radius`: speed / yaw rate when |yaw rate| > 0.05 rad/s, else
  straight.

`links` answers one `link_record` per link, `base_link` first, then the links
of attached slaves: name, parent, role, prim, joint, body, static offset to
the parent (translation and quaternion w, x, y, z, asset Z-up axes; zero for
joint-connected links), coupling `side|type|name`, element kind, number and
designator, and authored values. `tf` streams each link's world pose
(simulator world axes, Y up) every frame while switched on. `gearbox machine
links` and `gearbox machine tf` print both; `GEARBOX_TF_OVERLAY` (`frames`,
`names`, `links`, `wheels`, `all`) turns the viewer's TF overlay on at start.

### 9.7 Sensor streams

Authoring is in M§13. Each sensor link streams on `sensors/<link name>` at
its own simulated-time rate.

- `imu`: `datapod.imu.v1`: specific force and angular velocity in the link
  frame, and the link's orientation in the same world frame as `odom`.
- `lidar`: `gearbox.lidar_scan.v1`: a row-major sweep of ranges and
  link-frame points (zenith from +Z, azimuth from +X towards +Y), `NaN`
  points and infinite ranges for misses. A sweep arrives as consecutive
  chunks of at most 900 pulses (`pulse_offset`, `pulse_count`) sharing one
  `sample`; reassemble by pulse index.
- `camera`: `gearbox.camera_frame.v1`: a pinhole camera looking along +X with
  +Z up, as row bands of packed RGBA8 colour and f32 depth (metres along the
  pixel ray, infinite for a miss). Every band of one channel of a frame
  shares its `sample`. A Bevy-rendered colour frame and the Molla depth of
  the same sample share `sample` and simulated time but arrive separately.
  Bevy cameras render only on frames where a sample is due, at most once per
  rendered frame.
- Every other kind: `gearbox.measurement.v1` with `kind`, `count` records of
  a fixed width in `values` (link axes), and props `name`:

| Kind | One record |
|---|---|
| `accelerometer` / `gyro` | `[x, y, z]` specific force (m/s²) / angular velocity (rad/s) |
| `inertial_unit` | `[roll, pitch, yaw, qx, qy, qz, qw]` in east-north-up; yaw 0 facing east, counter-clockwise |
| `compass` | `[nx, ny, nz, heading]`: north in link axes, clockwise angle from north to +X |
| `gps` | `[latitude°, longitude°, altitude m, east, north, up, speed, v_east, v_north, v_up]` (WGS84; east-north-up from the site origin) |
| `distance` | `[distance m (infinite for none), value]` |
| `light` | `[irradiance, direct, sky (W/m²), visible sources, value]` facing +X |
| `position` | `[position (rad or m), velocity]` of the joint between the link's body and its parent link's body |
| `radar` | per target `[distance, azimuth, elevation, radial speed, received power dBm]`, nearest first; dynamic bodies not of this machine are targets |
| `touch` | `[touching 0/1, contacts, force N, fx, fy, fz]` on the link's body |
| `receiver` | one packet `[channel, signal strength, dx, dy, dz]` (unit direction to the emitter); payload in `data`, emitter `<machine>/<link>` in props `from` |
| `recognition` (camera with `recognition`) | per object `[id, pixels, left, top, right, bottom, x, y, z, sx, sy, sz]` in camera axes (−Z forward, +Y up); labels in props `names`, one per line |

`emit` takes an `emit_request` with props `link` (an emitter link) and the
payload in `data`. Requests naming no emitter link of the machine are
dropped.

### 9.8 Actuator commands and readings

`actuate` takes an `actuator_command` whose props name the `device` and its
settings. The reply confirms receipt only; an unknown device or a refused
setting is logged.

| Device | Props |
|---|---|
| Motor or brake joint | `brake` (damping); `speed` (position-control speed, ≤ `maxVelocity`), `acceleration` (≤ 0 unlimited), `max_force` or `max_torque`, `pid` (`p,i,d`); then one command: `position`, else `velocity`, else `force` or `torque` |
| Propeller | `velocity` or `speed` (rad/s), required |
| Belt | `speed`, `acceleration`; then `position` (travel, m) or `velocity` |
| Connector | `lock` or `unlock` (`1`, `true`, `on`, `yes`) |
| Copter | `mode` `off`, `hover`, `velocity` or `attitude`; `forward`, `left`, `up`, `yaw_rate`; `roll`, `pitch`, `climb`. Without `mode`, any of `roll`, `pitch`, `climb` means attitude, else velocity. `hover` is a velocity command. Missing values are 0. |
| Drag | none (refused) |

Every registered device streams `gearbox.measurement.v1` on
`actuators/<device>` every frame, props `name` (and `peer` for connectors):

| Kind | Record |
|---|---|
| `motor` (motor or brake joint) | `[position, velocity, commanded velocity, force/torque, brake damping, brake force/torque, mode (0 position, 1 velocity, 2 force), target]` |
| `propeller` | `[rotor speed, thrust N, torque N·m, speed of advance m/s]` |
| `belt` | `[travel m, speed m/s]` |
| `connector` | `[presence, locked, linked, tensile N, shear N]`; props `peer` names the present or linked `<machine>/<device>` |
| `copter` | `[roll, pitch, v forward, v left, v up, yaw rate, thrust N, saturated, mode (0 off, 1 velocity, 2 attitude)]` |
| `drag` | `[airspeed x, y, z (drag frame), force x, y, z N (world), wind x, y, z m/s (world)]` |

## 10. Parsed but unused

| Attribute | Where |
|---|---|
| `gearbox:machine:interfaceVersion`, `:upAxis`, `:visuals`, `:colliders`, `:sensors`, `:idPolicy` (except for detection) | machine prim |
| `gearbox:controller:<n>:namespacePolicy`, `:updateRateHz`, `:frameConvention`, `:usesRoles`, `:driveWheels` | controllers |
| `gearbox:coupling:isoCategory`, `:capacityKg`, `:services`, `:lift`, `:excludes` | couplings |
| `gearbox:propeller:controlPID`, `:minPosition`, `:maxPosition` | propellers |
| `controller_command.element` | `/cmd` |
| `physics:collisionEnabled`, `drive:<dof>:physics:type`, `physics:breakForce/breakTorque`, `physics:principalAxes`, `physics:density`, `physics:simulationOwner`, `PhysicsCollisionGroup` | UsdPhysics (M§4–M§6) |

## 11. Known gaps and surprising behaviour

- **Rejected machines still move.** They stay in the inventory, and the
  Ackermann, diff and tracked controllers do not check for an agent: a
  rejected Ackermann machine is parked by zero commands, a rejected diff
  machine in `chassis` mode has its horizontal velocity zeroed every frame,
  its devices register and hold their joints, and its `external:process`
  children start.
- A granted `speed`/`steering` request is never cleared: after the slave
  detaches, the master keeps driving at the last requested twist whenever its
  own twist is zero.
- The master state sent to slaves reads hitch, PTO and valve settings from
  `/cmd controller=…` props only. Values set on the master's links with
  `set-value` move the master's own joints but do not reach its slaves.
- `builtin:brake` and drive controllers race on shared wheel joints (§5.6);
  `builtin:brake` has no effect on a joint with a motor device.
- Service controllers commanding a motor device use the device's own
  `maxForce` (default 10 N·m); their own force caps do not apply.
- A twist sent with session 0 never times out (§9.4).
- `GEARBOX_MACHINE_ID` of an `external:process` child is the machine id, not
  the agent name when a `namespace` is authored. An exiting child is
  respawned every frame.
- Two machines with one id: only one gets an agent, silently.
- The odometry speed is unsigned except for tracks.
- A closed kinematic loop of joints without `excludeFromArticulation` is a
  link-tree error, but physics conversion does not wait for validation: Molla
  refuses the joint (`runtime joint cycle`) and the adapter panics
  (`invalid Molla joint topology`) instead of the machine being rejected.
- One bad sensor link (no rigid-body ancestor, a position sensor without a
  joint) stops every sensor of the machine; one device that never registers
  stops every device of the machine (M§12.1).
- `tyre_axle` errors disable every tyre of the machine (M§8.4).
- In the legacy steering path a runtime steer servo, once written, is taken
  for an authored Force drive on the next frame, so its gains and torque cap
  stay at the first frame's values instead of following the tyre loads.
- `gearbox:link:parent` on a jointed link drops its joint: service
  controllers do not find that link as the one their joint moves.
- Colliders under different articulation roots may share a collision group
  and never touch (M§5).
- The derived link tree gives no `wheel` roles, so machines without
  controllers get no tyres unless marked (M§8.1).
- Name heuristics (M§17) still steer diff-drive wheel signs and legacy
  steering.
- Not implemented: hitch `float`/`draft` modes; validation of role targets
  against link roles; element number uniqueness; a solid-wheel model;
  authored joint softness; rolling resistance (only with
  `GEARBOX_ROLL_RESIST=1`, on the chassis).

## 12. Environment

| Variable | Effect |
|---|---|
| `GEARBOX_TRACTION_CONTROL=0` (or `false`, `off`) | `tractionControl` defaults to false. |
| `GEARBOX_ALLOW_USD_CONTROLLER_PROCESS`, `GEARBOX_CONTROLLER_ALLOWLIST` | `external:process` policy (§7). |
| `GEARBOX_ROLL_RESIST=1` | Tyre rolling resistance applied to the chassis. |
| `BEVY_OPENUSD_METERS_PER_UNIT` | Overrides every stage's `metersPerUnit`. |
| `GEARBOX_GROUND_FRICTION` | Friction of the generated grounds. |
| `GEARBOX_DRIVE_DEBUG`, `GEARBOX_STEERING_DEBUG` | Per-frame drive and steering logs. |
| `GEARBOX_JOINT_DUMP=<machine id>` | Every 5 s, the machine's joints and wheel colliders. |
| `GEARBOX_PHYSICS_LOG` | Body and joint conversion logs. |
| `GEARBOX_TF_DEBUG`, `GEARBOX_TF_OVERLAY` | TF alignment reports; TF overlay at start. |
