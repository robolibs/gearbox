# Gearbox attachments, trailers and tools

How one machine attaches to another: a tractor towing a trailer, a trailer
towing a second trailer, an implement on a three-point hitch, a bucket on a
loader. Machines, controllers and the link tree are defined in
[MACHINE_SPEC.md](MACHINE_SPEC.md) and [CONTROLLER_SPEC.md](CONTROLLER_SPEC.md).

| Code | Covers |
|---|---|
| `bin/gearbox/src/links.rs` | Coupling and element attributes, link-tree validation |
| `bin/gearbox/src/attach.rs` | Attach and detach, hitch joints, stands, composite `/links`, `tool` routing |
| `bin/gearbox/src/services.rs` | Service and work controllers, master ↔ slave exchange, link values |
| `bin/gearbox/src/controller.rs` | Static attachments (`discover_static_attachments_from_stage`) |

Anything marked **not implemented** is parsed or planned and has no runtime
effect.

Terms:

- **master**: the machine that carries or tows. Root of a composite.
- **slave**: a machine attached to a master. It can itself be the master of
  further slaves.
- **composite**: a master plus every slave reachable from it.
- **coupling**: an authored attachment point. A **hitch** is the towing side,
  a **coupler** the towed side. One hitch holds one coupler.
- **carried** slave: has no controllers. **controlled** slave: has
  controllers.

Conventions this spec follows:

- ISO 11783 working-part tree: device, connector, functions, sections,
  units, bins and a navigation reference, each with named values. Gearbox
  keeps that tree as its link tree (§7.3).
- ISO 11783-9 tractor ECU classes and AEF TIM: the tractor broadcasts its
  state to the implement (class 1 and 2) and executes granted implement
  requests for speed, steering, PTO, hitch and valves (class 3, §5.3).
- Gazebo `DetachableJoint`: attach and detach by topic, no kinematic loops.
- Isaac Sim Robot Assembler: the attached asset is moved onto the base's
  attach point, a joint is created, and both form one articulation.

## 1. Machine roles

Roles are not authored. They follow from the couplings a machine has:

| Has hitches | Has couplers | Can be |
|---|---|---|
| yes | no | master only (tractor, self-propelled sprayer) |
| no | yes | slave only (mounted implement, single-axle trailer) |
| yes | yes | either (tandem trailer, front loader carrying a bucket) |
| no | no | standalone |

A machine is a slave from the moment one of its couplers is attached. Until
then it is an independent machine with its own id and session.

## 2. Coupling points

A prim is a coupling when it applies `GearboxCouplingAPI` or authors
`gearbox:coupling:side`. In a strict link tree (any prim of the machine is
marked as a link) the coupling prim must also be a link, otherwise the machine
is rejected. In a derived tree it becomes a link automatically. An unmarked
coupling link gets role `tool`. The coupling rides on its own rigid body or on
the nearest ancestor link that has one; attach refuses a coupling with none.

```usda
def Xform "rear_hitch" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxCouplingAPI"])
{
    token gearbox:link:role = "tool"
    token gearbox:coupling:side = "hitch"
    token gearbox:coupling:type = "drawbar"
    token gearbox:coupling:name = "rear_drawbar"
    double3 xformOp:translate = (0.0, 1.9, 0.45)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
```

| Attribute | Type | Default | Effect |
|---|---|---|---|
| `gearbox:coupling:side` | token | required | `hitch` or `coupler`. Missing or any other value is a link-tree error; the machine is rejected. |
| `gearbox:coupling:type` | token | required | One of §2.1. Missing or unknown is a link-tree error. Hitch and coupler must have the same type to attach. |
| `gearbox:coupling:name` | token | prim leaf, lower-cased, `[a-z0-9_]` | Names the coupling in attach requests and records. Uniqueness is not validated; a duplicated name makes a request that names it fail with `USAGE`. |
| `gearbox:coupling:ptoJoint` | rel | none | Coupler side: the slave joint that follows the master's PTO (§6.3). |
| `gearbox:coupling:valveJoints` | rel[] | `[]` | Coupler side: slave joints that follow the master's hydraulic valves, in valve order (§6.3). |
| `gearbox:coupling:stand` | rel | none | Coupler side: the slave's parking-stand rigid body (§3.4). |
| `gearbox:coupling:topLink` | rel | none | `three_point_mounted` only. Hitch side: the free end of the master's top link. Coupler side: the implement's upper hitch point. With both, the implement hangs on the lower-link pins and the top link holds it (§2.1). The target rides on its own rigid body or the nearest ancestor's. |
| `gearbox:coupling:variantSet` | token | none | Hitch side: a variant set on the machine prim that this hitch selects by reach (§3.6). Needs `variantNear` and `variantFar`; one without the others is a link-tree error. |
| `gearbox:coupling:variantNear` | token | none | The selection while a coupler of the hitch's type is within reach, or attached. |
| `gearbox:coupling:variantFar` | token | none | The selection otherwise. |
| `gearbox:coupling:isoCategory` | int | none | Parsed (clamped 0–4). **Not implemented.** |
| `gearbox:coupling:capacityKg` | float | none | Parsed. **Not implemented.** |
| `gearbox:coupling:services` | token[] | `[]` | Parsed. **Not implemented.** |
| `gearbox:coupling:lift` | rel | none | Parsed. **Not implemented.** |
| `gearbox:coupling:excludes` | token[] | `[]` | Parsed. **Not implemented**: two hitches whose prims overlap can both be occupied. |

Relationship targets are rebased with the machine like every other machine
relationship.

### 2.1 Coupling types and the hitch joint

Attaching creates a joint from the hitch's rigid body to the coupler's rigid
body (`attach.rs` `joint_for`). It is a tree joint: the slave's articulation
joins the master's. All three linear axes are locked. Rotation axes are the
hitch body's axes: pitch about X (lateral), roll about Y (longitudinal), yaw
about Z (up). Limits are symmetric per-axis joint limits, not cone limits and
not motors. A free axis without a limit is unlimited.

| `type` | Standard | Locked rotation | Free rotation |
|---|---|---|---|
| `three_point_mounted` with both `topLink` pins | ISO 730 | roll, yaw | pitch, held by the top link |
| `three_point_mounted` | ISO 730 | pitch, roll, yaw | none |
| `chassis_mounted` | | pitch, roll, yaw | none |
| `loader_carriage` | | pitch, roll, yaw | none |
| `drawbar` | ISO 6489-3 | roll | pitch, yaw |
| `clevis` | ISO 6489-2 | roll | pitch ±20°, yaw |
| `piton` | ISO 6489-4 | roll | pitch ±15°, yaw |
| `fifth_wheel` | | roll | pitch ±15°, yaw |
| `pivot_wagon` | ISO 5692-2 | pitch, roll | yaw |
| `three_point_semi_mounted` | ISO 730 | pitch, roll | yaw |
| `hitch_hook` | ISO 6489-1 | none | pitch ±25°, roll ±25°, yaw |
| `cuna` | ISO 6489-5 | none | pitch ±20°, roll ±20°, yaw |
| `ball` | ISO 24347 | none | pitch ±30°, roll ±30°, yaw |

A carried implement (the first four rows) hangs rigidly once captured (§3.3):
a revolute pin about the hitch body's X axis, or a fixed joint. The towed
types stay a compliant D6: 8 Hz while the joint is captured, rising to 30 Hz,
damping ratio 1.

**Top link.** A `three_point_mounted` hitch and coupler that both author
`gearbox:coupling:topLink` close the three-point linkage: besides the pin at
the lower links, a loop joint ties the master's top-link end to the
implement's upper point, all linear axes locked, rotations free. The
implement then pitches as the lower links, the top link and its own mast
let it, the way a real top link of fixed length does. A loop joint cannot be
rigid; the top link is held at 120 Hz, which keeps it within millimetres
under an implement's weight. Without both pins the implement is fixed to
the lower link and tilts with it.

The lift should drive a joint on the path that carries the implement: the
lower link the hitch coupling rides on. A lift driven through loop joints
(a rockshaft pulling lift rods) carries the load through compliant joints
and stretches under it.

While attached, every rigid body of the master is filtered against every
rigid body of the slave (all link bodies of both machines). Detach restores
those pairs. The two bodies joined by the hitch joint never collide. In a
nested composite only directly attached pairs are filtered: a tractor still
collides with the second trailer.

### 2.2 Coupling frame

Only the coupling prims' **positions** are used; their rotations are
ignored. The joint frames sit at the hitch and coupler prim positions with
the orientation of the hitch body and the coupler body. Attaching aligns the
coupler body with the hitch body's orientation, so the slave faces the same
way as the master only when both bodies use the chassis frame of
[MACHINE_SPEC.md](MACHINE_SPEC.md) (forward −Y, left +X, up +Z). A rear
hitch therefore sits at +Y on the tractor and a drawbar eye at −Y on the
trailer.

## 3. Attaching

### 3.1 Static, in a world layer

A USD that references several machines may author attachments:

```usda
def Xform "World"
{
    def Xform "Tractor_01" (prepend references = @tractor.usd@</robot>) {}
    def Xform "Trailer_01" (prepend references = @trailer.usd@</robot>) {}

    def Scope "Attachments"
    {
        def "tractor_trailer" (prepend apiSchemas = ["GearboxAttachmentAPI"])
        {
            rel gearbox:attachment:hitch = </World/Tractor_01/chassis/rear_hitch>
            rel gearbox:attachment:coupler = </World/Trailer_01/frame/drawbar_eye>
        }
    }
}
```

| Attribute | Type | Default | Effect |
|---|---|---|---|
| `gearbox:attachment:hitch` | rel | required | The master's hitch coupling prim. |
| `gearbox:attachment:coupler` | rel | required | The slave's coupler coupling prim. |

- A prim is an attachment when it applies `GearboxAttachmentAPI` or authors
  `gearbox:attachment:hitch`. One missing relationship is a warning; the
  attachment is skipped.
- Only the `defaultPrim` subtree of the loaded USD is scanned (the whole stage
  when there is no default prim). Both machines must come from the same
  loaded USD.
- Targets are stage paths and are not rebased.
- Once both machines have agents, the runtime issues a teleport attach with
  session 0 (§3.2). Targets that are not couplings give a warning. An
  attachment whose two machines do not both have agents within 1200 frames
  is dropped with a warning.

### 3.2 Runtime, over the master's agent

| Topic | Mode | Request → response |
|---|---|---|
| `/machines/<master>/tools/attach` | req/res | `gearbox.attach_request.v1` (`session`, `teleport`, props `slave`, `hitch`, `coupler`) → `gearbox.status.v1` (prop `slave` on success) |
| `/machines/<master>/tools/detach` | req/res | `gearbox.detach_request.v1` (`session`, prop `slave`) → `gearbox.status.v1` |
| `/machines/<master>/tools` | que/ans | `gearbox.ping.v1` → one `gearbox.attachment.v1` per attachment below the master, depth-first (`controlled`, `depth`, props `master`, `slave`, `hitch`, `coupler`, `type`, `denied`) |

Every attach and detach is also a `gearbox.scene_event.v1` of kind `attached`
or `detached` on `/gearbox/scene/events` with props `master`, `slave`,
`hitch`, `coupler`; `attached` adds `type` and `denied`.

Choosing the couplings:

- `hitch` names a hitch of the master. Without it, the only free hitch
  matching the type of the named coupler is used (any type when no coupler
  is named).
- `coupler` names a coupler of the slave. Without it, the only coupler with
  the hitch's type is used.
- No candidate answers `UNSUPPORTED`, several answer `USAGE`, an unknown
  name answers `NOT_FOUND` with the available couplings.

Attach checks, in order:

| Check | Refusal |
|---|---|
| `slave` is given | `USAGE` |
| The slave is not the master | `REFUSED` |
| `session` equals the master's session id (0 while nobody holds it) | `REFUSED` |
| The slave is not already attached | `REFUSED` |
| The master does not already hang below the slave (no loop) | `REFUSED` |
| The hitch is free | `BUSY` |
| Hitch and coupler types match | `REFUSED` |
| Both couplings ride on rigid bodies that exist in physics | `REFUSED` |
| `teleport = 0`: the coupler prim is within 0.5 m of the hitch prim and the coupler body within 30° of the hitch body's orientation | `REFUSED` |

With `teleport ≠ 0` every slave body is moved so the coupler lands on the
hitch with the hitch body's orientation. The slave is then pitched about the
hitch's lateral axis, up to three passes, until no `role = "wheel"` body more
than 0.5 m from the hitch sits more than 5 mm below the terrain.

Detach requires the master's session id (`REFUSED`) and an attachment of that
slave to that master (`NOT_FOUND`). It removes the hitch joint, restores
collisions between the two machines and sets the stand back to parked. The
slave stays where it is. Removing a machine from the scene removes its
attachments the same way.

```sh
gearbox machine tools list tractor_1
gearbox machine tools couplings tractor_1
gearbox machine tools attach trailer_1 --machine tractor_1 --hitch rear_drawbar --coupler drawbar_eye --teleport
gearbox machine tools detach trailer_1 --machine tractor_1
```

The viewer's machine context offers **Connect** when a free hitch and a
coupler of the same type on another machine are within reach (3 m), and
**Disconnect** for an existing attachment. Both need both machines unheld
(session 0) and nearly still (below 0.3 m/s and 0.2 rad/s). Connect does not
teleport.

### 3.3 Hitch capture

The hitch joint is created with its coupler frame where the coupler currently
is, so nothing jumps. The frame then moves to the authored coupler position
along a smoothstep over `max(0.5 s, 1.5 s × max(gap / 0.2 m, angle / 0.2 rad))`
while the joint's stiffness rises from 8 Hz. When the frames meet, a carried
implement's joint turns rigid and a towed one holds at 30 Hz.

The top link is captured the same way: caught where the master's top link
hangs, drawn onto the implement's upper point, stiffening from 8 Hz to
120 Hz. The top link swings up to meet it.

### 3.4 Parking stand

When the attached coupler authors `gearbox:coupling:stand`:

| Event | Stand body colliders | Slave machine prim variant set `coupling` |
|---|---|---|
| attach | disabled | `hitched` |
| detach | enabled | `parked` |

Without `stand`, no collider or variant is touched. The `coupling` variant set
is optional; author it with `hitched` and `parked` to swap stand geometry.

### 3.5 Loads

The master's wheel torque cap comes from its own tyres' solved loads
([DRIVETRAIN.md](DRIVETRAIN.md)), which include the load the hitch transfers.
An attached slave rolls on its own tyres and acts on the master only through
the hitch joint. Nothing adds the slave's mass to the master's force law.

A carried implement hangs its whole weight behind the rear axle. A master
too light at the front rears up instead of lifting it, and a lift too weak
for the implement's moment stalls part way, as a real tractor does; the
asset's masses, front ballast and lift rating decide which.

### 3.6 Hitch variants by reach

A hitch that authors `gearbox:coupling:variantSet`, `variantNear` and
`variantFar` (§2) shows its `near` selection of that variant set on the
master's machine prim while

- it holds an attached coupler, or
- a coupler of the hitch's type on another machine, not attached anywhere,
  is within reach: 3 m, the same reach the viewer offers **Connect** at;

and its `far` selection otherwise. A hitch showing `near` returns to `far`
only once every such coupler is beyond 3.5 m, so it does not flicker at the
edge. The switch is an ordinary variant swap: the stage recomposes, prims the
selection activates join physics where the machine stands, and the machine's
controllers and links are read again.

A tractor whose `hitchRear` set hides the top link in `bare` and shows it in
`linked` authors on its rear three-point hitch:

```usda
over "rear_three_point"
{
    custom token gearbox:coupling:variantSet = "hitchRear"
    custom token gearbox:coupling:variantNear = "linked"
    custom token gearbox:coupling:variantFar = "bare"
    custom rel gearbox:coupling:topLink = </robot/hitch_top_link/HITCH_3POINT_upper>
}
```

The top-link end that `topLink` names may exist only in the `near`
selection. Attaching before the swap has landed finds no pin and fixes the
implement to the lower link instead.

## 4. Composite link tree

The master's `/links` answers its own links followed by the links of every
slave below it, depth-first:

- slave link names are prefixed with the slave id path:
  `trailer_1/base_link`, `trailer_1/trailer_2/base_link`;
- each slave's root link is re-parented under the link carrying the hitch
  coupling and reported with role `link`, so the composite has one `base`;
- the slave's own `/links` does not change;
- the composite is not validated.

```
base_link                                  tractor_1
└── rear_hitch                             hitch link
    └── trailer_1/base_link                slave root, role link
        ├── trailer_1/drawbar_eye
        ├── trailer_1/axle_front
        └── trailer_1/rear_hitch
            └── trailer_1/trailer_2/base_link
```

## 5. Command routing

### 5.1 Session

While a slave is attached:

- its session is cleared, and its `claim` and `cmd_vel` answer `REFUSED` with
  props `attached_to` and `message`. Its `info`, `state`, `links`, `tf` and
  `cmd` keep working; `cmd` accepts session 0.
- a command on the master's `/cmd` with a `tool` prop is moved to the slave
  named by the last segment of `tool` (`trailer_1`, `trailer_1/trailer_2`),
  with `tool` removed. A `tool` naming no attached slave is dropped with a
  warning.
- the master's `info` and `state` carry `tools = <slave>,…` (the whole
  composite); the slave's carry `attached_to`.
- the master's `state` carries `tool.<slave>.controller.<instance>.<key>`:
  the latest command props of each directly attached slave's service
  controllers.

```sh
gearbox machine cmd tip position=1 --machine tractor_1 --tool trailer_1
```

### 5.2 Master to slave

Each frame, every attached slave gets an internal `MasterState` from its
master. It is not published.

| Field | Source on the master |
|---|---|
| ground speed, heading, roll, pitch | the state of the master's `cmd_vel` controller |
| hitch position per `builtin:hitch` instance | its last commanded `position` (or `value`), default 0 |
| PTO rpm and engaged per `builtin:pto` instance | its commanded `rpm` (540, clamped 0–1200) and `engaged` (false) |
| valve flows | each `builtin:hydraulic_valve`'s commanded `flow` (or `value`), in controller order |

Only `/cmd` props and granted requests feed the hitch, PTO and valve fields;
link values set on the master do not. A nested slave inherits its master's
inherited PTO set when its master has no PTO, and its ground speed when its
master reports zero.

Readers: `builtin:trailer_steer` automatic steering, the bound PTO and valve
joints (§6.3), `builtin:brake`'s default level (§6.2), and the work
controllers' ground speed (§7.4).

### 5.3 Slave to master

| Attribute | Prim | Type | Default | Effect |
|---|---|---|---|---|
| `gearbox:controller:<n>:requests` | slave machine | token[] | `[]` | What the slave will ask for. At attach, entries the master does not grant are reported as `denied` in the attachment record and event, and logged. Attach succeeds regardless. |
| `gearbox:machine:grants` | master machine | token[] | `[]` | Requests the master executes. |

```usda
token[] gearbox:controller:baler:requests = ["speed", "hitch:hitch_rear", "pto:pto_rear"]
token[] gearbox:machine:grants = ["speed", "hitch:hitch_rear", "pto:pto_rear", "aux_valve:0"]
```

A request is a `/cmd` on the **slave** with props `request=<token>` and
`value` (or the command's `value` field). It acts only while the slave is
attached and the master's `grants` contain that exact token. The slave's own
`requests` list is not consulted. Anything else is dropped with a one-time
warning.

| Request | Effect on the master |
|---|---|
| `speed` | Linear speed (m/s) of the master's drive command while its session twist is zero. |
| `steering` | Yaw rate (rad/s), same condition. |
| `hitch:<instance>` | `position` of the master's `builtin:hitch` controller `<instance>`. |
| `pto:<instance>` | `rpm = abs(value)` and `engaged = value > 0` on `builtin:pto` controller `<instance>`. |
| `aux_valve:<n>` | `flow` of the n-th `builtin:hydraulic_valve` of the master, 0-based, in controller order. |

A nonzero session twist always overrides `speed` and `steering`. Requested
values stay in effect until another request replaces them.

## 6. What a master authors

### 6.1 Couplings

At least one `hitch` coupling (§2). A typical tractor:

| Name | Type |
|---|---|
| `rear_three_point` | `three_point_mounted` |
| `rear_drawbar` | `drawbar` |
| `front_three_point` | `three_point_mounted` |
| `pickup_hitch` | `hitch_hook` |

### 6.2 Service controllers

Service controllers are `gearbox:controller:<n>:*` instances (see
[CONTROLLER_SPEC.md](CONTROLLER_SPEC.md)) of these types. They run on masters
and slaves alike.

| Type | Joint(s) driven |
|---|---|
| `builtin:joint_position`, `builtin:hitch`, `builtin:joint_velocity`, `builtin:pto`, `builtin:hydraulic_valve` | `:target`, else the machine's first `role:toolJoints` entry |
| `builtin:brake` | `:wheelJoints`, else `role:brakeJoints`, else `role:poweredWheelJoints` + `role:passiveWheelJoints` |
| `builtin:trailer_steer` | `:steerJoints`, else `role:steeringJoints` |

A controller with no joint logs a warning once. Positions are in rad
(revolute) or m (prismatic), velocities in rad/s or m/s.

**Actuation** (`services.rs` `actuate`):

- A joint with a `gearbox:motor:*` device takes position and velocity
  commands through that device; the device's own limits apply and the caps
  below do not.
- Otherwise the runtime writes an Acceleration-model servo on the joint each
  frame (gains per unit of joint inertia): position stiffness 4000, damping
  400, cap 50 000; velocity damping 200, cap per type below.
- `builtin:brake` always writes the runtime servo: velocity 0, damping
  `400 × level`, cap `50 000 × level`. A motor device on the same joint
  replaces it every step.

| Type | Props (default) | Command |
|---|---|---|
| `builtin:joint_position` | `position` or `value` (0), clamped 0–1; `range` (1) | position `position × range` |
| `builtin:hitch` | same as `builtin:joint_position` | position `position × range` |
| `builtin:joint_velocity` | `velocity` | velocity `velocity`; without it, the master's first engaged PTO speed if this joint is the slave's bound `ptoJoint`, else 0. Cap 2000. |
| `builtin:pto` | `rpm` (540), clamped 0–1200; `engaged` (false) | velocity `rpm × 2π / 60` when engaged, else 0. Cap 150. |
| `builtin:hydraulic_valve` | `flow` or `value` (0), clamped −1–1; `rate` (0.5) | velocity `flow × rate`. Cap 50 000. |
| `builtin:brake` | `level` or `value`, clamped 0–1; default 1 while the machine is not attached as a slave, 0 while attached | brake servo |
| `builtin:trailer_steer` | `angle_rad`; `:maxSteerDeg` (35) | position `angle_rad`; without it, `−(master heading − trailer heading)` wrapped to ±π while attached, else 0. Clamped to ±`maxSteerDeg`. |

Commands are `/cmd` with `controller=<instance>` and props. Props merge over
time: the last value per key stays. Numeric props (and `true`/`on`/`yes` as
1, `false`/`off`/`no` as 0) are also written to the live values of the link
the joint moves. Each frame the live values of that link override the props,
so `set-value <link> position 0.8` moves the joint too (§7.4). Flags read
`1`, `true`, `on` and `yes` as true.

`builtin:hitch` has no `float` or `draft` mode; that is **not implemented**.

### 6.3 PTO and valve binding

A slave names the joints the master's services drive on its coupler
coupling: `gearbox:coupling:ptoJoint` and `gearbox:coupling:valveJoints`.
While the slave is attached, each frame:

| Slave joint | Command | Cap |
|---|---|---|
| `ptoJoint` | velocity = the master's first engaged PTO speed (rad/s), else 0 | 150 |
| `valveJoints[n]` | velocity = master valve n's flow × 0.5 | 50 000 |

A joint named by one of the slave's own controllers is skipped; a
`builtin:joint_velocity` on the PTO joint follows the PTO itself unless it is
commanded a `velocity`. Commands go through the joint's motor device when it
has one (§6.2). No joint is created between the machines. The binding is read
from the slave's first coupler coupling.

## 7. What a slave authors

### 7.1 Every slave

- A complete machine ([MACHINE_SPEC.md](MACHINE_SPEC.md)). A carried trailer
  is a machine too.
- At least one `coupler` coupling (§2).
- `gearbox:machine:kind` should name the implement class: `trailer`,
  `sprayer`, `seeder`, `baler`, `mower`, `spreader`, `loader_tool`. It is
  informational.

### 7.2 Trailers

A machine without controllers gets pressure tyres only on links with
`gearbox:link:role = "wheel"` ([TYRE_PRESSURE.md](TYRE_PRESSURE.md)), so mark
every trailer wheel link `wheel`. Controllers for a steered axle, brakes or a
tipping body:

| Type | Purpose |
|---|---|
| `builtin:trailer_steer` | steered axle: follows the articulation, or `angle_rad` when commanded |
| `builtin:brake` | wheel brake, `level` 0–1; fully applied while the trailer is not attached |
| `builtin:joint_position` | tipping body, tailgate, unloading auger |

A tandem trailer also authors a `hitch` coupling on its rear. Nesting depth is
not limited; the loop check in §3.2 is the only guard.

### 7.3 Working parts as links

A working part is a link that also applies `GearboxElementAPI` or authors
`gearbox:element:type`. Its parent part is its parent link. A prim with an
element type but no link marker is still made a link, with role `tool`, so it
has a frame in `/tf`.

```usda
def Xform "boom" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxElementAPI"])
{
    token gearbox:link:role = "tool"
    token gearbox:element:type = "function"
    string gearbox:element:designator = "Boom"
    float gearbox:value:ActualWorkingWidth = 24.0
    float gearbox:value:position = 0.0

    def Xform "section_0" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxElementAPI"])
    {
        token gearbox:element:type = "section"
        float gearbox:value:ActualWorkingWidth = 3.0
        double3 xformOp:translate = (-2.5, 10.5, 0.0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
}
def Xform "tank" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxElementAPI"])
{
    token gearbox:element:type = "bin"
    float gearbox:value:MaximumVolumeContent = 4000.0
    float gearbox:value:ActualVolumeContent = 4000.0
}
```

| Attribute | Type | Default | Effect |
|---|---|---|---|
| `gearbox:element:type` | token | none | `device`, `function`, `bin`, `section`, `unit`, `connector` or `navigation`. Any other value is a link-tree error. |
| `gearbox:element:number` | int ≥ 0 or uint | none | Published on `/links` as `number`. Uniqueness is **not validated**. |
| `gearbox:element:designator` | string or token | prim leaf | Label, published on `/links` as `designator`. |
| `gearbox:value:<Name>` | float, double, int, uint or bool | none | A named value of the link (§7.4). |

### 7.4 Values on links, and the controllers that read them

- Every `gearbox:value:<Name>` on a link prim becomes a live value once, when
  the machine gets its agent.
- `/cmd` with props `link`, `name` and `value` sets a live value on a link of
  that machine. Address a slave's link on the slave's own `/cmd`, or on the
  master's with `tool`. An unknown link is dropped with a warning.
- Live values are published on `/state` as `link.<link>.<Name>`. `/links`
  carries the authored values as `value.<Name>`.

```sh
gearbox machine set-value boom position 0.8 --machine sprayer_1
```

A service controller (§6.2) reads the live values of the link its joint
moves. A work controller acts on links by element type. Work controllers use
the master's ground speed (§5.2); a slave that is not attached has speed 0.

| Type | Acts on | Each frame |
|---|---|---|
| `builtin:section_control` | the `function` link named by `:target`, else the first `function` link, and its direct `section` children | Section `ActualWorkState` = function `SectionControlState` (1) > 0.5 and section `SetpointWorkState` (0) > 0.5 and speed > 0.05 m/s. Function `ActualWorkState` = any section working. Function `TotalArea` (ha) += speed × working width × dt / 10 000; `EffectiveTotalDistance` (m) += speed × dt while any section works. Working width is the sum of the working sections' `ActualWorkingWidth`. |
| `builtin:rate_control` | the `bin` link named by `:target`, else the first `bin` link | Applies while the working width of all working sections > 0, `ActualVolumeContent` > 0 and speed > 0.05 m/s. `ActualVolumePerAreaApplicationRate` = `SetpointVolumePerAreaApplicationRate` while applying, else 0. `ActualVolumeContent` (l) −= rate (l/ha) × speed × width × dt / 10 000, floored at 0. |

## 8. Validation

Link-tree errors reject the machine:

- a coupling prim that is not a link in a strict tree;
- a missing or unknown `gearbox:coupling:side` or `gearbox:coupling:type`;
- a `gearbox:element:type` outside the seven kinds.

Attach refusals are listed in §3.2. Warnings: slave requests the master does
not grant, a `request` from a machine that is not attached, a granted request
the master has no controller for.

**Not implemented:** the three-point `lift` check, element-number uniqueness,
coupling-name uniqueness, ISO category and capacity warnings, and a warning
for an implement attached to a master that reports no speed.

## 9. Examples

### 9.1 Tractor rear end

```usda
def Xform "chassis" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"])
{
    token gearbox:link:name = "base_link"
    token gearbox:link:role = "base"
    def Xform "lower_links" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"])
    {
        token gearbox:link:role = "tool"
        def Xform "rear_three_point" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxCouplingAPI"])
        {
            token gearbox:coupling:side = "hitch"
            token gearbox:coupling:type = "three_point_mounted"
        }
    }
    def Xform "rear_drawbar" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxCouplingAPI"])
    {
        token gearbox:coupling:side = "hitch"
        token gearbox:coupling:type = "drawbar"
    }
    def Xform "pto_stub" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"]) {}
}
# On the machine prim:
#   apiSchemas += GearboxControllerAPI:hitch_rear, GearboxControllerAPI:pto_rear
#   token gearbox:controller:hitch_rear:type = "builtin:hitch"
#   rel   gearbox:controller:hitch_rear:target = </robot/Joints/rear_lift>
#   token gearbox:controller:pto_rear:type = "builtin:pto"
#   rel   gearbox:controller:pto_rear:target = </robot/Joints/pto>
#   token[] gearbox:machine:grants = ["speed", "hitch:hitch_rear", "pto:pto_rear"]
```

### 9.2 Tipping trailer

```usda
def Xform "robot" (prepend apiSchemas = ["GearboxMachineAPI", "GearboxControllerAPI:tip"])
{
    token gearbox:machine:kind = "trailer"
    rel gearbox:machine:body = </robot/frame>
    rel gearbox:machine:role:passiveWheelJoints = [</robot/Joints/rev_left>, </robot/Joints/rev_right>]
    token gearbox:controller:tip:type = "builtin:joint_position"
    rel gearbox:controller:tip:target = </robot/Joints/tip>

    def Xform "frame" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"])
    {
        token gearbox:link:name = "base_link"
        token gearbox:link:role = "base"
        def Xform "drawbar_eye" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxCouplingAPI"])
        {
            token gearbox:coupling:side = "coupler"
            token gearbox:coupling:type = "drawbar"
            double3 xformOp:translate = (0.0, -2.6, 0.45)
            uniform token[] xformOpOrder = ["xformOp:translate"]
        }
    }
    def Xform "body" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"]) {}
    # wheel links with gearbox:link:role = "wheel", joints ...
}
```

Attach and tip after loading both:

```sh
gearbox machine tools attach trailer_1 --machine tractor_1 --hitch rear_drawbar --coupler drawbar_eye --teleport
gearbox machine cmd tip position=1 --machine tractor_1 --tool trailer_1
```

### 9.3 Mounted sprayer

A `three_point_mounted` coupler on `base_link`; a `boom` function link with
`section` children as in §7.3; a `tank` bin link; controllers `sections`
(`builtin:section_control`) and `rate` (`builtin:rate_control`);
`requests = ["speed", "hitch:hitch_rear"]` so it can lift itself at the
headland. Attached, its sections are switched with
`gearbox machine set-value section_0 SetpointWorkState 1 --machine sprayer_1`.

## 10. Not implemented

- Hitching by driving into the coupler. Attach is always a request.
- `builtin:hitch` `float` and `draft` modes, and draft sensing.
- `gearbox:coupling:lift`, `excludes`, `services`, `isoCategory` and
  `capacityKg`.
- Detaching by breaking force: `physics:breakForce` is not honoured on hitch
  joints.
- Validation of the composite link tree.
- ISOBUS or CAN transport, and device-description export or import. The link
  tree is the only description of a machine.
