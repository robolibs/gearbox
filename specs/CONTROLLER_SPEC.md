# Gearbox controller specification

What a USD asset must author, and what the runtime must do, for a machine to
be controllable in Gearbox. This documents the behaviour implemented in
`bin/gearbox/src/controller.rs` and `bin/gearbox/src/load.rs` as of version
0.0.6, plus the link-tree contract in §7 that the next runtime must
implement. Where design and code differ elsewhere, `SCHEMA.md` holds the
design direction and this file holds the contract the code enforces.

Terms:

- **MUST** — the runtime refuses to attach or drive the controller without it.
- **SHOULD** — omitting it works but degrades behaviour or breaks the scripts.
- **IGNORED** — parsed into the spec struct but never read by any system.
- **PROPOSED** — not implemented yet. Assets SHOULD author it now so they
  work unchanged when the runtime catches up. §7 is entirely PROPOSED.
- **REQUIRED CONTRACT** — required for full asset compatibility, but not yet
  enforced by the current loader. Unlike MUST above, this does not claim that
  the runtime already rejects an asset which omits it.

External conventions this spec binds to:

- REP-103 units and axes: SI, right-handed, body frames x forward / y left /
  z up, optical frames z forward / x right / y down.
- REP-105: `base_link` is the root link of a mobile platform, one parent per
  link. REP-145: the IMU link is `imu_link`.
- LinkForge's robot IR (`Link`, `Joint{parent,child,origin,axis}`,
  `Sensor{link_name,origin}`, single-root validation) as the authoring-side
  model that a USD machine should round-trip to.

## 1. Loading

A USD becomes a candidate machine only if it enters through a path that runs
controller discovery.

| Entry | Runs discovery |
|---|---|
| CLI argument (`gearbox path.usd`) | yes |
| Add USD button in the Selection pane | yes |
| `/gearbox/usd/load` with `category` = `machine` | yes |
| `/gearbox/usd/load` with any other category | **no** — spawned by the gearbox-api loader as a prop |

The load request MAY carry a `namespace` string. When present it replaces both
the machine id and every controller namespace on that asset (`load.rs`,
`apply_runtime_namespace`). This is how one asset is spawned several times
under `hunter_1`, `hunter_2`, and so on.

Discovery re-opens the USD file from disk independently of the Bevy asset,
after stripping prim metadata `openusd-rs` cannot parse. It scans only the
subtree under the layer's `defaultPrim`. The file MUST set `defaultPrim`.

Machines discovered on load are logged as
`gearbox-control: machine id=... kind=... controllers=...`. An asset with no
machines logs `no USD-authored machines discovered`.

## 2. Machine prim

A prim is a machine if **any one** of these is authored on it:

- `GearboxMachineAPI` in `apiSchemas`
- `token gearbox:machine:kind`
- `token gearbox:machine:idPolicy`
- `rel gearbox:machine:body`

Authoring the apiSchema entry is the SHOULD form. The attribute fallbacks exist
for files whose `apiSchemas` were stripped.

### Machine attributes

| Property | Type | Status | Notes |
|---|---|---|---|
| `gearbox:machine:id` | string | optional | Fixed id. Do not author in reusable assets. |
| `gearbox:machine:idPolicy` | token | optional | Only `prim_path` is implemented. Default `prim_path`. |
| `gearbox:machine:kind` | token | SHOULD | Free-form label, logged and shown in the UI. |
| `gearbox:machine:body` | rel | MUST unless every controller has its own `body` | Chassis prim. See §4. |
| `gearbox:machine:role:poweredWheelJoints` | rel[] | MUST for ackermann | Joints that receive drive torque. |
| `gearbox:machine:role:passiveWheelJoints` | rel[] | SHOULD for ackermann | Free-rolling joints, still used as tyre contacts. |
| `gearbox:machine:role:suspensionJoints` | rel[] | optional | Prismatic springs between the chassis and a wheel carrier; their USD drive (`stiffness`, `damping`, `maxForce`, `targetPosition` as preload) is the spring and the runtime never drives them. A non-prismatic target is a link tree warning. |
| `gearbox:machine:role:steeringJoints` | rel[] | SHOULD for ackermann | Joints that receive steer position targets. |
| `gearbox:machine:role:brakeJoints` | rel[] | IGNORED | |
| `gearbox:machine:role:toolJoints` | rel[] | IGNORED | |
| `gearbox:machine:visuals` / `colliders` / `sensors` | rel[] | IGNORED | |
| `gearbox:machine:interfaceVersion` | token | IGNORED | Author `"0.1"` for forward compatibility. |
| `gearbox:machine:upAxis` | token | IGNORED | The runtime assumes Z-up regardless. See §8. |
| `GearboxLinkAPI` on every rigid body under the machine | applied schema | PROPOSED, MUST | Link tree. See §7. |

### Machine id derivation

With no `gearbox:machine:id` and no runtime namespace, the id is the composed
prim path lower-cased with every non-alphanumeric run collapsed to `_`:

```
/robot               -> robot
/World/Tractor_01    -> world_tractor_01
```

Relationship targets are rebased onto the machine prim, so an asset authored at
`/robot` referenced into `/World/Tractor_01` resolves `</robot/chassis>` to
`/World/Tractor_01/chassis`.

## 3. Controller instance

A controller instance named `<n>` exists if **either**:

- `GearboxControllerAPI:<n>` is in the machine prim's `apiSchemas`, or
- `<n>` is one of `drive`, `arm`, `implement` and
  `gearbox:controller:<n>:type` is authored.

Any other instance name without the apiSchema entry is not discovered. All
properties below live on the machine prim with the prefix
`gearbox:controller:<n>:`.

### Controller attributes

| Property | Type | Status | Default | Notes |
|---|---|---|---|---|
| `type` | token | MUST | `builtin:unknown` | One of the three types in §3.1. Anything else is silently inert. |
| `enabled` | bool | optional | `true` | Disabled controllers get no topics and no motion. |
| `commandInterface` | token | MUST for cmd_vel | none | Must be exactly `cmd_vel`, otherwise no machine agent is created. |
| `stateInterfaces` | token[] | SHOULD | `[]` | Must contain `pose` or `velocity` for the `.../state` topic to publish. Scripts wait on this topic. |
| `body` | rel | MUST unless `gearbox:machine:body` is set | machine body | Controller-level override of the chassis. |
| `namespace` | string | optional | machine id | Topic namespace for this instance. |
| `namespacePolicy` | token | optional | `machine_id` | Only `machine_id` is implemented. |
| `wheelBase` | float, m | SHOULD | `2.37` | Steering angle from yaw rate. |
| `wheelRadius` | float, m | SHOULD | `0.45` (ackermann), `0.1` (diff drive) | Visual spin and fallback when a wheel has no collider. |
| `trackWidth` | float, m | SHOULD | `1.5675` | Ackermann inner/outer split. |
| `frontTrackWidth` / `rearTrackWidth` | float, m | optional | `trackWidth` | Per-axle override. |
| `maxSteerDeg` | float | SHOULD | `45` | Steer clamp. |
| `steeringGeometry` | token | optional | `ackermann` | See §3.2. |
| `frontSteerMultiplier` / `middleSteerMultiplier` / `rearSteerMultiplier` | float | optional | geometry default | Legacy per-axle angle multiplier; front/rear ratios locate the virtual pivot when all Ackermann axles steer. Ignored with a resolved fixed axle. |
| `frontLeftSteerDifferentialDeg` etc. (four corners) | float | optional | `0` | Legacy per-corner offset, not used by resolved geometric Ackermann. |
| `driveWheelJoints`, `passiveWheelJoints`, `wheelJoints` | rel[] | optional | `[]` | Controller-level joint overrides, merged with machine roles. |
| `frontLeftWheelJoint` … `rearRightWheelJoint` | rel | optional | none | Explicit corner joints. The rear pair counts as powered. |
| `steerLeftJoint` / `steerRightJoint` | rel | optional | none | Explicit steering pair, merged with roles by the geometric solver; takes precedence in legacy fallback resolution. |
| `steerJoints` | rel[] | optional | `[]` | Extra steering joints, merged with `role:steeringJoints`. Last-resort fallback receives the centre angle. |

| `maxWheelTorqueNm` | float | optional | none | Per-wheel drive torque limit, applied on top of the grip cap. |
| `maxPowerKw` | float | optional | none | Total drive power: grip/torque-limited wheel budgets are scaled together so `Σ τ·max(abs(ω),0.5) ≤ P`; wheel speed is relative to its parent body. Parking brakes are not engine-power-limited. |
| `tractionControl` | bool | optional | `true` | Keeps each driven wheel within 8 % + 0.15 m/s of its ground speed. `GEARBOX_TRACTION_CONTROL=0` sets the default off. |
| `driveMode` | token | optional | `chassis` | Differential drive only. `chassis` writes the twist onto the body with slippery tyres. `wheels` drives each powered wheel's velocity motor at `(v − ω·x)/r` under `maxWheelTorqueNm` and `maxPowerKw`, with normal tyre grip, bounded yaw feedback for skid scrub, and a full-torque hold at rest. |
| `zeroCommand` | token | optional | `coast` | Ackermann only. `coast` free-wheels a rolling machine on a zero command until it is below 0.25 m/s, then parks. `brake` drives the wheels to rest at the command brake rate (1.6 m/s²) first, as a geared servo drive does. |
| `driveWheels`, `target` | rel | IGNORED | | |
| `updateRateHz` | float | IGNORED | `60` | Controllers run every Update frame. |
| `frameConvention` | token | IGNORED | | Author `usd_z_up`; the runtime assumes it anyway. |
| `usesRoles` | token[] | IGNORED at runtime | | Only checked by unit tests. Author it for documentation. |
| `executable`, `args`, `transport` | string, string[], token | MUST for `external:process` | | See §3.1. |

### 3.1 Controller types

**`builtin:ackermann_cmd_vel`** — wheeled vehicles with steered axles. The
solid tyre colliders carry the machine: the wheel joints are velocity
motors capped at the tyre's grip
(`0.9 · μ · N · r`, `N` the machine's weight over all its wheels), passive
wheels idle at ground speed, an integral trim holds the commanded ground
speed, and steer joints keep their authored USD drive gains (else
50 000 N·m/rad) and only take new targets. Requires body (§4) and at
least one resolvable wheel joint pair (§5). Without wheel pairs it logs
`no wheel joint pairs found ...` once and stops. Every wheel link publishes
`link.<wheel>.slip` (tyre surface speed over ground speed, minus one).

**`builtin:diff_drive_cmd_vel`** — skid-steer and differential machines. The
commanded twist is written directly onto the chassis body as horizontal
velocity and yaw rate. Tyre friction is forced to 0.05 and wheels are carried
along kinematically, spinning visually from ground speed. Requires body (§4).
Wheel joints are optional but SHOULD be authored so wheels spin.

**`external:process`** — spawns an executable that speaks the same agentio
topics. Deny by default. The runtime MUST be launched with
`GEARBOX_ALLOW_USD_CONTROLLER_PROCESS=1` and
`GEARBOX_CONTROLLER_ALLOWLIST=/abs/dir[:/abs/dir]`, and the canonicalised
`executable` MUST lie under one of those directories. The child receives
`GEARBOX_MACHINE_ID`, `GEARBOX_CONTROLLER`, `GEARBOX_NAMESPACE`, and
`GEARBOX_TRANSPORT` (default `agentio`) in its environment. Blocked reasons are
recorded per controller and shown in the Machine controllers pane.

**Service controllers** drive one joint each with a rapier motor and are
commanded through `/machines/<ns>/cmd` (`controller = <instance>` plus the
keys below; `gearbox machine cmd <instance> key=val`). The joint is
`gearbox:controller:<n>:target`, or the machine's first `toolJoints` entry.

| Type | Keys | Drives |
|---|---|---|
| `builtin:joint_position` | `position` 0..1, `range` (rad or m, default 1) | motor position `position × range` on the target joint |
| `builtin:hitch` | `position` 0..1 | the lift joint; reported to attached slaves as `hitch.<instance>.position` |
| `builtin:pto` | `rpm` (default 540), `engaged` | motor velocity on the PTO stub; reported as `pto.<instance>` |
| `builtin:hydraulic_valve` | `flow` −1..1, `rate` (default 0.5) | motor velocity `flow × rate` on the named joint |
| `builtin:brake` | `level` 0..1 | a velocity motor braking every wheel joint (`wheelJoints`, else `brakeJoints`, else all wheels) |
| `builtin:trailer_steer` | `angle_rad`, else automatic | steering joints follow the master's heading when attached, clamped to `maxSteerDeg` (default 35) |
| `builtin:joint_velocity` | `velocity` rad/s, else bound PTO | motor velocity on the target joint; a joint named by the coupler's `gearbox:coupling:ptoJoint` turns with the master's PTO |

Every link of the tree carries **named values** (`TOOLS_SPEC.md` §7.3),
seeded from `gearbox:value:<Name>` attributes and set at runtime with `/cmd`
carrying `link` and `name` props (`gearbox machine set-value LINK NAME
VALUE`). A service controller overlays the values of the link its joint moves
onto its own command props, so `set-value boom position 0.8` moves the boom
without naming a controller. Values ride `/links` as `value.<Name>` props and
`/state` as `link.<link>.<Name>`.

**Work controllers** act on links by element kind rather than on a joint:

| Type | Acts on | Behaviour |
|---|---|---|
| `builtin:section_control` | the `function` link it targets and its `section` children | `SetpointWorkState` becomes `ActualWorkState` while the master moves and `SectionControlState` is on; `TotalArea` and `EffectiveTotalDistance` accumulate from speed × active width |
| `builtin:rate_control` | the `bin` link it targets | `SetpointVolumePerAreaApplicationRate` becomes the actual rate while sections work; `ActualVolumeContent` drains from speed × active width × rate |

A controller MAY author `gearbox:controller:<n>:requests` (`speed`,
`steering`, `hitch:<name>`, `pto:<name>`) and a machine MAY author
`gearbox:machine:grants`; see `TOOLS_SPEC.md` §5.3.

### 3.2 Steering geometries

| Token | Front | Middle | Rear |
|---|---|---|---|
| `ackermann` (default) | 1.0, true Ackermann inner/outer split | 1.0 | 1.0 |
| `parallel` | 1.0, both wheels same angle | 1.0 | 1.0 |
| `crab` | 1.0 | 1.0 | −1.0 |
| `six_wheel_ackermann`, `oxbo`, `rear_counter_half`, `counter_steer_half` | 1.0 | 0.0 | −0.5 |

For Ackermann and counter-steering tokens, resolved joint geometry takes precedence
over this legacy multiplier table. Steering roles and explicit joint relationships
are merged and deduplicated. Fixed wheels locate the turn-centre axle line; steering
pivots determine each wheel angle. When all axles steer, front/rear multiplier ratios
locate the virtual pivot. With a fixed axle, multipliers and differential-degree
offsets are not used. `parallel` and `crab` retain their legacy mappings.

All Ackermann wheel targets use one curvature, reduced together to satisfy every
steering joint's USD limits and `maxSteerDeg`. Inner wheels steer more sharply;
front and rear wheels counter-steer about a fixed middle axle when present.
Wheel speeds use the complete local rolling vector and each tyre's radius,
including both lateral and longitudinal offsets from the turn centre.

Bus `cmd_vel` uses signed `angular / linear` curvature; positive yaw in reverse
requires opposite steering. At standstill it cannot request a finite turn.
The gamepad instead supplies normalized steering separately, independent of speed,
including while parked. Its dead zone is rescaled continuously to avoid an input jump.

## 4. Body

The controller's `body` (or the machine `body`) MUST point at a prim that:

- carries `PhysicsRigidBodyAPI`, so `usd_bevy` creates a rapier body for it;
- has a collider somewhere in the machine, so terrain alignment can finish
  (`load.rs` waits for both bodies and colliders and warns after 120 frames);
- SHOULD carry `PhysicsMassAPI` with `physics:mass` and
  `physics:diagonalInertia`. The machine's total mass sets the wheel
  torque cap.
- `physics:diagonalInertia` MUST be plausible for the mass. The runtime
  compares each component with a box estimate from the chassis collider
  bounds (`m/12 · (b² + c²)`); when any component is below 25 % of that
  estimate it substitutes the estimate on that axis and logs
  `chassis inertia (...) is implausible`. A 3.8 t tractor authored at
  452 kg·m² on every axis rolled on its suspension and crept at rest until
  this guard existed; the fix belongs in the asset, the guard only keeps
  the machine usable.

If the body relationship is missing, the prim is not found under the loaded
scene root, or it has no rapier body, the controller silently does nothing
every frame. No log line is emitted.

### 4.1 Self-collision

`PhysicsFilteredPairsAPI` on a body is honoured: every contact between that
body and each listed body is dropped by a rapier pair filter. Linkages that
run inside the chassis hull (three-point hitch rods, cylinders, stabilisers)
MUST list the chassis and each other, or their hulls fight the chassis every
step. Contacts between two bodies joined by a joint are always off. Once a
machine is up the runtime also drops every contact between two of its own
bodies, so a machine touches only the ground and other machines; coupled
machines stop colliding with each other until they detach.

## 5. Wheel joints

Each wheel joint path MUST resolve to a prim that `usd_bevy` turned into a
`UsdPhysicsJoint` with both `physics:body0` and `physics:body1` mapping to
rapier bodies. In USD terms:

- a `PhysicsRevoluteJoint` (or another `UsdPhysics` joint type) prim;
- `rel physics:body0` = chassis or steering knuckle, `rel physics:body1` = wheel,
  both prims carrying `PhysicsRigidBodyAPI`;
- the wheel body has a collider. Its largest half-extent is taken as the tyre
  radius if above 0.05 m, otherwise `wheelRadius` is used.
- the tyres of every `wheel` link carry the machine. The
  runtime rounds a `Cylinder` tyre's edge (5 % of the radius), raises its
  friction to at least 1.1 with a `min` combine rule, so the ground's
  material decides the grip, zeroes restitution, and turns on CCD. Name the
  collider `tire_collider` so spawn alignment finds it. Generated grounds
  take `GEARBOX_GROUND_FRICTION` when set.
- Wheel-speed traction control measures motion along the tyre's contact plane using
  impulse-weighted contact normals, falling back to the chassis up axis without support.
  Its slip clamp does not reverse the requested drive direction during rollback or turn
  a zero-speed parking command into a downhill wheel-speed target.
- steer joints without an authored drive get a force-based motor capped at
  the standstill scrub torque `μ · N · w / 2` of their tyre.

Joint sources, all merged and deduplicated:
`role:poweredWheelJoints`, `role:passiveWheelJoints`, controller
`driveWheelJoints`, `passiveWheelJoints`, `wheelJoints`, and the four corner
relationships. A joint is **driven** if it is in `role:poweredWheelJoints`,
`driveWheelJoints`, `rearLeftWheelJoint`, or `rearRightWheelJoint`.

For an all-wheel-drive asset, list every driven rolling joint in `poweredWheelJoints`
and remove those joints from `passiveWheelJoints`. Torque remains bounded by tyre grip,
`maxWheelTorqueNm` and `maxPowerKw`; multiplying the torque ceiling alone cannot bypass grip.

Drive grip comes from the last solved tyre contact impulses divided by the physics timestep,
using each solver contact's combined friction. Unsupported powered wheels receive only a
shaft-spin budget (`mass × radius² × 4 rad/s²`), not ground traction. Supported wheels retain
their grip-derived budget; all powered wheels share the total power ceiling. Slip limiting
requires solved support. Parking torque is independent of contact load, bounded by authored
wheel torque (6,000 Nm fallback), so lifted wheels can brake too.
Axle load transfer and trailer tongue load therefore affect grip through the physical joints
and contacts, rather than an equal share of the tractor's own mass. This is a wheel-torque /
power-envelope model, not an engine-RPM, clutch or discrete-gear simulation. Assets intended
for towing should author both limits; omitting power keeps the legacy grip-only power budget.
The Machines drive panel reports authored limits, powered/loaded wheel counts, the current
motor torque budget and whether the power ceiling is active. The budget is not measured torque.

Steering joints follow the same rule: each path in `role:steeringJoints`,
`steerJoints`, `steerLeftJoint`, or `steerRightJoint` MUST resolve to a
`UsdPhysicsJoint` between two rapier bodies, typically chassis and a steering
knuckle body. The knuckle then carries the wheel joint as `physics:body0`;
§7.5 fixes that chain.
The geometric Ackermann solver merges explicit and role steering joints. Assets
without resolvable geometry retain legacy target resolution as a fallback.
Steering joints must define straight-ahead at zero and rotate about chassis up
(reversed axes are supported). Multiple fixed axles at different longitudinal
positions cannot all roll without scrub in a turn; their mean line is an approximation.

Rigid bodies MUST be siblings in the prim tree, not nested inside one another.
Nested bodies hit transform write-back ordering and drift off the machine each
frame. Both `tractor.usd` and `hunter.usd` follow this.

Collision shapes SHOULD be primitives. The collider adapter bakes prim `scale`
into `Cube` and `Sphere` sizes.

### 5.1 Wheel and tyre compatibility — REQUIRED CONTRACT

Every compatible machine must explicitly classify its ground-contact wheels:
pneumatic tyre, solid wheel, or another supported contact model. Every
pneumatic wheel must satisfy the [tyre asset contract](TYRE_PRESSURE.md#required-machine-asset-contract),
whether powered, passive, steered, a caster, or attached to a trailer or
implement. Machine brand and mesh names must not decide support.

That contract requires explicit wheel/joint and rubber/rigid-part bindings,
reference geometry, axle identity, pressure bounds and material properties.
One tyre state must supply support, grip, visible deformation and readout.
Solid wheels and tracks must declare their applicable model; machines without
wheels are not required to invent tyres or pressure controls.

**Enforcement is incomplete.** The isolated Molla adapter currently recognises
rubber by names and derives missing dimensions/default properties. Those are
legacy import fallbacks, not evidence of full compatibility. Explicit contact-
type/mesh-binding schema and loader validation still need implementation.
Rapier currently provides rigid wheel behaviour, not pressure-tyre capability.

## 6. Tool interface (agentio over peerbus)

The simulator hosts one agentio agent for the scene and one per discovered
machine. Every topic carries a `DatapodMsg` envelope whose payload is a
datapod named `gearbox.<type>.v1`; the Rust definitions are in
`crates/gearbox-api/src/wire`, the Python mirrors in
`examples/python/gearbox_client.py`.

A running host writes `$XDG_RUNTIME_DIR/gearbox/<name>.json` with its
`did:key`, its endpoint address, and pid. Machines are listed by the host.

| Topic (host agent) | Mode | Request → Response |
|---|---|---|
| `/gearbox/info` | req/res | `Ping` → `HostInfo` |
| `/gearbox/scene/clock` | req/res | `ClockCommand` → `ClockState` |
| `/gearbox/scene/clear` | req/res | `ClearRequest` → `Status` |
| `/gearbox/scene/list` | que/ans | `ListQuery` → `SceneObject`… |
| `/gearbox/scene/events` | pub/sub | `SceneEvent`: loaded, pose, harvested, removed, machine_ready |
| `/gearbox/usd/load`, `/gearbox/usd/delete` | req/res | `UsdLoad` / `UsdRef` → `Status` |
| `/gearbox/marker/set`, `/gearbox/marker/delete` | req/res | `MarkerSet` / `MarkerRef` → `Status` |
| `/gearbox/select` | req/res | `Selection` → `Selection` |
| `/gearbox/machines/list` | que/ans | `Ping` → `MachineRef`… (namespace, did, addr) |

| Topic (machine agent) | Mode | Payload |
|---|---|---|
| `/machines/<ns>/info` | req/res | `Ping` → `MachineInfo` (controllers and their interfaces) |
| `/machines/<ns>/claim` | req/res | `ClaimRequest{take, hold_ms}` → `ClaimResponse{session, code}` |
| `/machines/<ns>/cmd_vel` | req/res | `TwistCmd{session, twist}` → `Status` |
| `/machines/<ns>/release` | req/res | `SessionRef` → `Status` |
| `/machines/<ns>/session` | req/res | `Ping` → `SessionInfo` |
| `/machines/<ns>/cmd` | req/res | `ControllerCommand` → `Status` |
| `/machines/<ns>/state` | pub/sub | `MachineState`: `Odom`, heading, roll, pitch, session |
| `/machines/<ns>/odom` | pub/sub | `datapod::robot::Odom` |

`cmd_vel` follows ROS `base_link` convention: `linear.vx` forward in m/s,
`angular.vz` yaw in rad/s, `angular.vy` accepted as a fallback. Commands
are dead-banded at 0.03 m/s and 0.02 rad/s, clamped to ±16 m/s and
±4.8 rad/s, and slewed at 80 m/s² and 6 rad/s².

### Sessions

- `claim` grants a session id when the machine is free, or refuses with
  `BUSY` and the holder. `take = 1` steals it.
- `cmd_vel` is accepted with the holder's session id, or with session 0
  while nobody holds the machine.
- Silence longer than the claim's `hold_ms` (default 500) zeroes the twist.
  A minute of silence releases the machine (the holder is presumed gone).
- `release` frees it; so does despawn and scene clear.

### State payload

Published every frame from `PostUpdate` while the controller runs and
`stateInterfaces` contains `pose` or `velocity`. Position is the chassis
body translation in the Y-up world frame. `heading_rad` is
`atan2(forward.x, forward.z)`: 0 faces +Z, +π/2 faces +X. `roll_rad` and
`pitch_rad` follow REP-103.

## 7. Link tree

Every machine MUST declare its own link tree: which prims are links, which
link is the root, and how they connect. Anything outside the machine's root
link is not part of the machine and is not described here.

The runtime answers the tree on `/machines/<ns>/links` (que/ans,
`gearbox.link_record.v1` per link, `base_link` first) and streams world link
poses on `/machines/<ns>/tf` (`gearbox.link_pose.v1`) while a client has
switched them on with `/cmd` `tf = on`. `gearbox machine links` and
`gearbox machine tf` show both.

TF poses and the viewer's TF axes/names use the same propagated world
transforms as the rendered links. Frame order is physics step → parent-first
body writeback → transform propagation → TF publication and overlays.
Nested rigid bodies factor out the current parent pose, including intervening
non-physical Xforms and scene-unit scale, not the previous frame's parent pose.
Wheel frames include both steering and axle spin; steering-knuckle frames
include steering only.

The viewer's **Wheels only** TF filter shows wheel links without coincident
steering-knuckle frames. Wheel axes are larger than knuckle axes; display
lengths are in world metres, independent of authored scene-unit scale.
`GEARBOX_TF_DEBUG=1` reports body/physics pose error and wheel-mesh rotation
relative to its wheel frame. `GEARBOX_TF_OVERLAY=frames,names,wheels` opens a
labelled wheel-only view.

**Transition rule.** An asset that authors no `GearboxLinkAPI` at all is
loaded with a *derived* tree: every rigid body is a link named after its
prim, `physics:body1` is the child of `physics:body0`, the machine body is
`base_link`. The runtime warns on every load and `MachineInfo` reports
`links_derived = true`. The moment one prim carries `GearboxLinkAPI`, §7.3
applies in full and a failing machine gets no agent (a `machine_rejected`
scene event carries the reasons).

### 7.1 Marking links

A link is a prim under the machine prim carrying `GearboxLinkAPI`:

```usda
def Xform "chassis" (
    prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]
)
{
    token gearbox:link:name = "base_link"
    token gearbox:link:role = "base"
}
```

| Property | Type | Status | Default | Notes |
|---|---|---|---|---|
| `gearbox:link:name` | token | MUST | prim name | Link name. Lowercase `[a-z0-9_]`, unique within the machine. |
| `gearbox:link:role` | token | SHOULD | `link` | `base`, `link`, `wheel`, `steer`, `sensor`, `tool`. |
| `gearbox:link:parent` | rel | optional | derived | Override the derived parent. MUST target another link in the same machine. |

Rules:

- **Every `PhysicsRigidBodyAPI` prim under the machine MUST carry
  `GearboxLinkAPI`.** An unmarked rigid body is a validation error, not a
  silently ignored body.
- **Exactly one link MUST have `role = "base"`** and its name MUST be
  `base_link`. It is the root of the tree. It MUST be either the machine body
  (`gearbox:machine:body`) itself, or a static `Xform` directly under it. In
  the second form the machine body is the child of `base_link`, not its
  parent, with the inverse of that `Xform` as the static transform between
  them. This is the one place §7.2's parent derivation is inverted.
- **`base_link` axes MUST follow REP-103**: +X forward, +Y left, +Z up in the
  asset's Z-up stage frame. If the chassis geometry is authored on a different
  basis, use the second form above: the `Xform` carries the correcting
  rotation and the chassis body stays a plain `link`.
- **Links without a rigid body are allowed** for `sensor` and `tool` roles.
  They are static `Xform`s under a rigid-body link and move with it.
  Sensor link names SHOULD follow ROS practice: `imu_link`, `gps_link`,
  `camera_link`, `lidar_link`. A camera SHOULD also author
  `<name>_optical` as a child link with the REP-103 optical rotation.
- Wheel bodies SHOULD use `role = "wheel"` and steering knuckles
  `role = "steer"`, chained as §7.5 describes. The controller role relationships in §2 keep pointing at
  joints; the link role is descriptive and lets the validator check that a
  `poweredWheelJoints` target actually drives a wheel link.

### 7.2 Connections

Links connect through the `UsdPhysics` joints already required by §5. A joint
with `physics:body0 = A` and `physics:body1 = B` makes `B` a child of `A`.
No separate parent attribute is needed for rigid-body links.

For links without a rigid body, the parent is the nearest ancestor prim that
is a link. The transform to it is the composed local xform between the two
prims, and it is static.

`gearbox:link:parent` overrides either derivation for that one link.

### 7.3 Validation

The runtime MUST reject the machine, with the reason logged and shown in the
Machine controllers pane, when any of these fail. The set mirrors LinkForge's
`TreeStructureCheck` and `JointReferenceCheck`.

- a rigid body under the machine has no `GearboxLinkAPI`;
- no link, or more than one link, has `role = "base"`;
- a link name is duplicated within the machine;
- a joint's `body0` or `body1` is not a link of this machine;
- a link is `body1` of two or more joints (a kinematic loop), unless all but
  one of those joints set `physics:excludeFromArticulation`;
- a link is not reachable from `base_link` (disconnected);
- `gearbox:link:parent` targets a prim outside the machine or creates a cycle;
- a controller role relationship (§2) targets a joint whose child link is not
  `role = "wheel"` or `role = "steer"` as appropriate.

### 7.4 LinkForge round-trip

Link names and joint endpoints are chosen so a machine authored in LinkForge
or URDF compiles to this layout without renaming:

| LinkForge / URDF | USD |
|---|---|
| `Link.name` | `gearbox:link:name` on the rigid-body prim |
| root link (never a child) | the `role = "base"` link, named `base_link` |
| `Joint.parent` / `Joint.child` | `physics:body0` / `physics:body1` |
| `Joint.origin` | joint `physics:localPos0` / `localRot0` |
| `Joint.axis` | `physics:axis` |
| `Sensor.link_name` + `Sensor.origin` | static `Xform` under that link with `GearboxLinkAPI`, `role = "sensor"` |
| `Ros2ControlJoint.command_interfaces` | `gearbox:controller:<n>:commandInterface` |
| `Ros2ControlJoint.state_interfaces` | `gearbox:controller:<n>:stateInterfaces` |

### 7.5 Steered wheels: one chain, two joints

A wheel that both spins and steers is two links in series, never one link
with two joints and never two joints side by side. This is the URDF rule (a
link has exactly one parent joint) and the shape ros2_control's steering
controllers expect: `steering_joints_names` take a position in radians,
`traction_joints_names` take a velocity in rad/s, as two separate joint lists.

```
chassis                              role = link (or base)
└─ steer_<pos>      revolute about the up axis, limited to ±maxSteerDeg
   └─ steer_<pos>   role = steer      the knuckle
      └─ wheel_<pos>   revolute about the axle axis, unlimited
         └─ wheel_<pos>   role = wheel    the tyre
```

- The **steer link is the parent of the wheel link**. Steering turns the
  knuckle; the tyre follows because it is the knuckle's child. Spin is one
  level below, about the axle.
- The steer joint's axis is the machine up axis; the wheel joint's axis is
  the axle (X in a Z-up asset whose front is −Y). Positive steer MUST turn
  the machine toward its up axis (REP-103), which the runtime's
  `steeringGeometry` sign convention assumes.
- The knuckle's origin SHOULD sit at the wheel centre, so the steer axis
  passes through the tyre and the wheel joint's `physics:localPos0` is zero.
  Kingpin offset, if wanted, is authored as the wheel joint's local position
  inside the knuckle, not as another link.
- A wheel that does not steer skips the knuckle: `chassis → wheel_<pos>`.
- A pivoting axle is one more revolute level between the chassis and the
  steer links (`chassis → axle → steer → wheel`). It is optional and swings
  freely within its limits, carried by its tyres; a small USD drive on it
  damps the swing.
- A tandem of two rigid axles rocks onto one of them as soon as a hitch pins
  the nose. Hang both axles from a walking beam instead: a `bogie` link on a
  revolute (axis X, a few degrees each way) between the chassis and a point
  midway between the axles, with the axle or wheel joints' `body0` on the
  bogie. The Claas and Krampe trailers do this.
- Names carry the axle and side hints the runtime matches on (§8):
  `steer_front_left`, `wheel_front_left`, `roll_front_left` all pair up;
  `wheel_back_left` pairs with nothing and is not steered.

In the tf tree this is what `gearbox machine links` prints for a tractor:

```
base_link  [base]
  chassis  [link]
    steer_front_left  [steer]    via steer_front_left
      wheel_front_left  [wheel]    via roll_front_left
    wheel_back_left  [wheel]    via roll_back_left
```

Validation (§7.3) warns when a `wheel` link's parent is a `steer` link but
the joint between them is not a revolute, or when a `steer` link's joint
axis is not the up axis; it rejects a `wheel` link that has more than one
joint to its parent.

## 8. Fixed conventions the runtime assumes

These are not configurable today. An asset that violates them drives wrong
with no diagnostic. §7.2 describes how an authored `base_link` replaces the
first one.

- **Chassis local basis**: forward is −Y, left is +X, up is +Z. The world is
  Y-up and the loader rotates Z-up USD into it. Author the machine so it drives
  toward local −Y.
- **Axle from joint path**: case-insensitive substring. `front` or `fwd` →
  front; `middle` or `mid` → middle; `rear` or `back` → rear.
- **Side from joint path**: `left` or `_l` → left; `right` or `_r` → right.
  Note `_r` also matches `_rear`, so prefer `left`/`right` spelled out.
- **Steering joint to wheel matching**: a steering joint steers the wheels that
  share both its axle hint and its side hint.
- **Tractor preset radii**: joints whose paths contain `front`/`rear`/`back`
  plus `left`/`right` may pick up hard-coded tractor tyre radii for visual
  spin, but only if the measured collider radius is within 25 % of the preset.

## 9. Isaac Sim compatibility

A prim with `PhysicsArticulationRootAPI`, physics joints beneath it, and no
gearbox metadata is synthesised into a machine with
`kind = "isaac_articulation"`, `interfaceVersion = "isaac_compat:v0"`, and a
`drive` controller of type `builtin:ackermann_cmd_vel` with
`steeringGeometry = "ackermann"`. Joint roles come from OmniGraph `jointNames`
arrays, classified by substring: `steer` or `position` → steering, `wheel`,
`velocity`, or `drive` → powered. This is best effort and mislabels are
expected on unfamiliar exports.

## 10. Minimal conforming asset

```usda
#usda 1.0
(
    defaultPrim = "robot"
    upAxis = "Z"
    metersPerUnit = 1
)

def Xform "robot" (
    prepend apiSchemas = ["GearboxMachineAPI", "GearboxControllerAPI:drive"]
)
{
    token gearbox:machine:interfaceVersion = "0.1"
    token gearbox:machine:kind = "cart"
    token gearbox:machine:idPolicy = "prim_path"
    rel gearbox:machine:body = </robot/chassis>
    rel gearbox:machine:role:poweredWheelJoints = [
        </robot/Joints/rev_rear_left>, </robot/Joints/rev_rear_right>
    ]
    rel gearbox:machine:role:passiveWheelJoints = [
        </robot/Joints/rev_front_left>, </robot/Joints/rev_front_right>
    ]
    rel gearbox:machine:role:steeringJoints = [
        </robot/Joints/steer_front_left>, </robot/Joints/steer_front_right>
    ]

    token gearbox:controller:drive:type = "builtin:ackermann_cmd_vel"
    bool gearbox:controller:drive:enabled = true
    token gearbox:controller:drive:commandInterface = "cmd_vel"
    token[] gearbox:controller:drive:stateInterfaces = ["pose", "velocity"]
    rel gearbox:controller:drive:body = </robot/chassis>
    float gearbox:controller:drive:wheelBase = 1.2
    float gearbox:controller:drive:trackWidth = 0.9
    float gearbox:controller:drive:wheelRadius = 0.3
    float gearbox:controller:drive:maxSteerDeg = 35
    token gearbox:controller:drive:steeringGeometry = "ackermann"

    def Xform "chassis" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]
    )
    {
        token gearbox:link:name = "chassis"
        # base_link (§7): turns the asset's -Y-forward chassis into REP-103
        # +X-forward. Sensor links are static mounts under it.
        def Xform "base_link" (prepend apiSchemas = ["GearboxLinkAPI"])
        {
            token gearbox:link:name = "base_link"
            token gearbox:link:role = "base"
            float3 xformOp:rotateXYZ = (0, 0, -90)
            uniform token[] xformOpOrder = ["xformOp:rotateXYZ"]

            def Xform "imu_link" (prepend apiSchemas = ["GearboxLinkAPI"])
            {
                token gearbox:link:role = "sensor"
                double3 xformOp:translate = (0.1, 0.0, 0.2)
                uniform token[] xformOpOrder = ["xformOp:translate"]
            }
            def Xform "camera_link" (prepend apiSchemas = ["GearboxLinkAPI"])
            {
                token gearbox:link:role = "sensor"
                double3 xformOp:translate = (0.6, 0.0, 0.5)
                uniform token[] xformOpOrder = ["xformOp:translate"]
            }
        }
        # ... visuals, colliders, mass ...
    }
    def Xform "wheel_rear_left" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"])
    {
        token gearbox:link:role = "wheel"
        # ...
    }
    # ... remaining wheel and knuckle bodies as siblings of chassis ...
    def Scope "Joints"
    {
        def PhysicsRevoluteJoint "rev_rear_left"
        {
            rel physics:body0 = </robot/chassis>
            rel physics:body1 = </robot/wheel_rear_left>
            uniform token physics:axis = "X"
        }
        # ...
    }
}
```

Drive it:

```bash
python - <<'EOF'
import sys; sys.path.insert(0, "scripts")
from gearbox_client import Gearbox
gb = Gearbox()                       # first running host in the registry
m = gb.machine("robot")             # resolves the machine's own agent
session = m.claim()                  # refused with BUSY while someone holds it
m.cmd_vel(1.0, 0.3)                  # forward 1 m/s, yaw 0.3 rad/s
print(m.state(wait=1.0).position)
m.release()
EOF
```

## 11. Known gaps

Recorded here so authors know what to expect; none are enforced or fixed yet.

- No link tree exists. Rigid bodies are found by scanning for physics
  schemas, nothing names them, and nothing checks that the joints form a tree
  rooted at the chassis. §7 is the target.

- Missing `type`, `commandInterface`, or an unresolvable `body` fail silently.
  Only missing wheel joints produce a warning. SCHEMA.md's validator does not
  exist.
- `pause_clock` on `gearbox/sim/clear` is ignored; physics keeps running on an
  empty scene when triggered over the bus.
- Session ownership survives a scene clear.
- `updateRateHz`, `frameConvention`, `upAxis`, `usesRoles`, `target`, brake
  and tool roles, and the visuals/colliders/sensors relationships are parsed
  and unused.
- Discovery writes a stripped copy of every text USD to the system temp
  directory and never removes it.
