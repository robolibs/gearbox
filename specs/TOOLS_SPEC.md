# Gearbox attachments, trailers, and tools specification

How one machine attaches to another in Gearbox: a tractor towing a trailer,
a trailer towing a second trailer, an implement mounted on a hitch, a bucket
on a loader. This builds on `CONTROLLER_SPEC.md` (machines, controllers,
link tree) and is **entirely PROPOSED**: the current runtime knows one machine
per loaded USD and cannot join two of them. Assets SHOULD author it now.

Terms:

- **master** — the machine that carries or tows. Root of a composite.
- **slave** — a machine attached to a master. Its link tree hangs under the
  master's. Slaves can themselves be masters of further slaves.
- **composite** — master plus every slave reachable from it. One `base_link`,
  one command session, one tree.
- **coupling** — an authored attachment point. A **hitch** is the towing side,
  a **coupler** the towed side. One hitch holds one coupler.
- **carried** slave — has no controllers. It moves because it is attached.
- **controlled** slave — has controllers. Commands reach it through the
  master.

External conventions this spec binds to:

- ISO 11783-10 device descriptor object pool (DDOP): one `Device` element,
  a `Connector` element locating the coupling, `Function`, `Section`, `Unit`,
  `Bin`, `NavigationReference` elements in a parent/child tree, process data
  addressed by DDI number. AgIsoStack's `DeviceElementObject::Type` numbers
  them 1..7 in that order.
- ISO 11783-7 DDI 157 connector types, the enumeration §3.2 reuses.
- ISO 11783-9 tractor ECU classes: class 1 and 2 broadcast tractor state to
  the implement, class 3 also executes implement commands. AEF TIM is the
  cross-vendor form of class 3: the implement commands speed, steering, PTO,
  hitch, and remote valves, and the tractor confirms by status.
- SDF composition: a joint in an outer model may connect links of two
  included models by scoped name (`arm::mount`, `gripper::mount_point`).
- Gazebo `DetachableJoint`: fixed joint between links of two models, attach
  and detach by topic, no kinematic loops, no reattach while in contact.
- Isaac Sim Robot Assembler: an attach point prim on each asset, the attached
  asset is moved onto the base's attach point, a fixed joint is created, and
  the attached articulation root is removed so one articulation remains.

## 1. Machine roles

Roles are not authored. They follow from what a machine has:

| Has hitches | Has couplers | Can be |
|---|---|---|
| yes | no | master only (tractor, self-propelled sprayer) |
| no | yes | slave only (mounted implement, single-axle trailer) |
| yes | yes | either (tandem trailer, front loader carrying a bucket) |
| no | no | standalone, cannot join a composite |

A machine is a **slave** at runtime the moment one of its couplers is
attached. Until then it is an independent machine with its own namespace and
session exactly as `CONTROLLER_SPEC.md` describes.

## 2. Coupling points

A coupling is a link (§7 of `CONTROLLER_SPEC.md`) that also carries
`GearboxCouplingAPI`:

```usda
def Xform "rear_hitch" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxCouplingAPI"])
{
    token gearbox:link:role = "tool"
    token gearbox:coupling:side = "hitch"
    token gearbox:coupling:type = "drawbar"
    token gearbox:coupling:name = "rear_drawbar"
    double3 xformOp:translate = (-1.9, 0.0, 0.45)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
```

| Property | Type | Status | Default | Notes |
|---|---|---|---|---|
| `gearbox:coupling:side` | token | MUST | | `hitch` (towing side) or `coupler` (towed side). |
| `gearbox:coupling:type` | token | MUST | | One of §2.1. Hitch and coupler MUST have the same type to attach. |
| `gearbox:coupling:name` | token | SHOULD | prim name | Unique among this machine's couplings. Used in attach requests. |
| `gearbox:coupling:isoCategory` | int | optional | | ISO 730 category 0..4 for three-point types. Mismatch is a warning, not a refusal. |
| `gearbox:coupling:capacityKg` | float | optional | | Vertical load limit. Exceeding it is a warning. |
| `gearbox:coupling:services` | token[] | optional | `[]` | Service couplings offered or required here: `pto`, `hydraulic`, `electrical`, `isobus`. |
| `gearbox:coupling:lift` | rel | optional | | Hitch side, three-point types only: the master joint that raises the lower links. |

### 2.1 Coupling types and the joint they imply

Types follow ISO 11783-7 DDI 157 so a DDOP `ConnectorType` maps one-to-one.

| `type` | DDI 157 | Joint created between hitch link and coupler link |
|---|---|---|
| `drawbar` | 1, ISO 6489-3 | revolute about Z, pitch free ±20°, roll free ±10° |
| `three_point_semi_mounted` | 2, ISO 730 | revolute about Z at the lower links, lift via `lift` |
| `three_point_mounted` | 3, ISO 730 | fixed, lift via `lift` |
| `hitch_hook` | 4, ISO 6489-1 | spherical, cone limit 25° |
| `clevis` | 5, ISO 6489-2 | revolute about Z, pitch free ±20° |
| `piton` | 6, ISO 6489-4 | revolute about Z, pitch free ±15° |
| `cuna` | 7, ISO 6489-5 | spherical, cone limit 20° |
| `ball` | 8, ISO 24347 | spherical, cone limit 30° |
| `chassis_mounted` | 9 | fixed. Self-propelled machine carrying its own implement. |
| `pivot_wagon` | 10, ISO 5692-2 | revolute about Z |
| `fifth_wheel` | — | revolute about Z, pitch free ±15° |
| `loader_carriage` | — | fixed. Front-loader tool carrier, quick-attach frames. |

"Free" ranges are joint limits, not motors. Collision between the two coupled
links, and between the slave's coupler link and the master's hitch link only,
is disabled while attached. Everything else keeps colliding.

### 2.2 Coupling frame

The coupling prim's origin is the physical attachment point. Its axes follow
REP-103 relative to the machine it belongs to: +X toward that machine's
front, +Z up. Attaching places the coupler frame coincident with the hitch
frame and rotated 180° about Z, so the slave faces the same way as the
master.

For DDOP export the coupler's pose in the slave's `base_link` becomes the
`Connector` element's `DeviceElementOffsetX/Y/Z` (DDI 134, 135, 136) after
the frame conversion in §7.3.

## 3. Attaching

### 3.1 Static, in a world layer

A world USD that references both machines MAY author the attachment. Both
machines are still discovered separately by their own `GearboxMachineAPI`
prims, as discovery already supports several machines under one loaded
asset.

```usda
def Xform "Tractor_01" (prepend references = @tractor.usd@</robot>) {}
def Xform "Trailer_01" (prepend references = @trailer.usd@</robot>) {}

def Scope "Attachments"
{
    def "tractor_trailer" (prepend apiSchemas = ["GearboxAttachmentAPI"])
    {
        rel gearbox:attachment:hitch = </World/Tractor_01/rear_hitch>
        rel gearbox:attachment:coupler = </World/Trailer_01/drawbar_eye>
    }
}
```

The runtime creates the §2.1 joint at load, before physics activates, after
moving the slave so its coupler frame meets the hitch frame.

### 3.2 Runtime, over the machine agent

The master's agent hosts the attachment topics (`PLAN_COM.md` §10 for the
bus; `PLAN_TOOLS.md` for the implementation):

| Topic | Mode | Request → Response |
|---|---|---|
| `/machines/<master_ns>/tools/attach` | req/res | `gearbox.attach_request.v1` (`session`, `teleport`, props `slave`, `hitch`, `coupler`) → `gearbox.status.v1` |
| `/machines/<master_ns>/tools/detach` | req/res | `gearbox.detach_request.v1` (`session`, props `slave`) → `gearbox.status.v1` |
| `/machines/<master_ns>/tools` | que/ans | `gearbox.ping.v1` → `gearbox.attachment.v1` × N, depth-first (`controlled`, `depth`, props `master`, `slave`, `hitch`, `coupler`, `type`) |

`hitch` and `coupler` MAY be omitted when exactly one free pair of matching
type exists. Every change is also a `attached` / `detached` scene event on
`/gearbox/scene/events`.

Rules, taken from Gazebo's `DetachableJoint` and Isaac's assembler:

- The request MUST carry the master's session id (0 while nobody holds it).
- `hitch` and `coupler` MUST have the same `type` and be free.
- With `teleport = false` the coupler frame MUST lie within 0.25 m and 30° of
  the hitch frame or the request is refused. With `teleport = true` the slave
  is moved onto the hitch first. Teleport is refused while the slave's bodies
  are in contact with anything but the ground.
- Attaching MUST NOT create a loop: the slave's composite MUST NOT already
  contain the master.
- A machine cannot attach to itself, and a coupler attaches to one hitch.
- Detach removes the joint, restores collisions, and leaves the slave where
  it is with zero velocity commanded.
- `/tools` answers late joiners; every change is also an `attached` or
  `detached` scene event.

### 3.3 Tolerances and safety

The master's body mass, wheel forces, and brake constants already scale with
chassis mass (`CONTROLLER_SPEC.md` §4). Attached slaves add their mass to
the towed load, not to the chassis, so the raycast vehicle's engine force
MUST also scale with total composite mass or a loaded trailer stalls the
tractor.

## 4. Effect on the link tree

The slave keeps the whole link tree it authored. Only its root changes
parent:

```
tractor_1/base_link
└── tractor_1/chassis
    └── tractor_1/rear_hitch                (hitch link)
        └── [drawbar joint]
            └── tractor_1/trailer_1/base_link   (slave root, re-parented)
                ├── tractor_1/trailer_1/drawbar_eye
                ├── tractor_1/trailer_1/axle_front
                └── tractor_1/trailer_1/rear_hitch
                    └── [drawbar joint]
                        └── tractor_1/trailer_1/trailer_2/base_link
```

- The slave's link names are prefixed with the master prefix plus the
  slave's runtime namespace, so two identical trailers never collide.
- The composite has exactly one `base_link`, the master's. The slave's
  `base_link` is still a link in the tree. It is no longer a root.
- Tree validation (`CONTROLLER_SPEC.md` §7.3) runs on the composite after
  every attach and detach.
- On detach the slave's tree is restored under its own namespace.

## 5. Command routing

### 5.1 Session

A composite has one command session, the master's. While attached:

- the slave's own `/machines/<slave_ns>/claim` and `cmd_vel` answer
  `REFUSED` with an `attached_to` prop naming the master. Its `/info`,
  `/state`, `/links` and `/tf` keep working, since watchers still want them.
- a slave controller is commanded through the master's `/cmd` with a `tool`
  prop naming the slave (`tool = "trailer_1"`, nested `"trailer_1/trailer_2"`);
  the sim routes the command to that slave's controller.
- the master's `/state` props gain `tools = <slave>,<slave>…`, and its
  `/links` answers the composite tree of §4.

This is the ISOBUS shape: the implement is on the tractor's bus, and the task
controller talks to it through that bus, not around it.

### 5.2 Master to slave (ISO 11783-9 class 1 and 2)

The master publishes its tractor state to every attached slave controller
each step, in the slave's `base_link` frame. It mirrors the ISO 11783-7
broadcasts an implement relies on:

| Field | ISO source |
|---|---|
| `ground_speed_mps`, `distance_m`, `direction` | PGN 65097 ground speed and distance |
| `hitch.<name>.position` (0..1), `.in_work` | PGN 65093 rear hitch status |
| `pto.<name>.rpm`, `.engaged` | PGN 65091 rear PTO status |
| `aux_valve.<n>.flow` (−1..1) | auxiliary valve status |
| `heading_rad`, `roll_rad`, `pitch_rad` | tractor pose |

A slave controller reads it from its `ControllerInputs`. Section control,
rate control, and bin accounting need speed and work state and nothing else.

### 5.3 Slave to master (ISO 11783-9 class 3, AEF TIM)

A controlled slave MAY request master functions. It declares what it needs:

```usda
token[] gearbox:controller:baler:requests = ["speed", "hitch:rear_lift", "pto:rear"]
```

The master declares what it grants:

```usda
token[] gearbox:machine:grants = ["speed", "hitch:rear_lift", "pto:rear", "aux_valve:1"]
```

Attach succeeds regardless; a request that is not granted is reported in
`attachments` as `"denied": [...]` and ignored, matching TIM where a tractor
without a valid certificate simply does not execute. Granted requests are
written as commands onto the master's own controllers each step:

| Request | Master controller written |
|---|---|
| `speed` | the drive controller's `cmd_vel.linear` |
| `steering` | the drive controller's `cmd_vel.angular` |
| `hitch:<name>` | the `builtin:hitch` instance of that name (§6.2) |
| `pto:<name>` | the `builtin:pto` instance of that name |
| `aux_valve:<n>` | the `builtin:hydraulic_valve` instance `n` |

The external session still owns the composite. A slave request never
overrides a command the session sent in the same step.

Over the bus a slave (or a script acting for it) sends a request as
`/machines/<slave_ns>/cmd` with props `request = speed | steering` and a
`value`; the runtime honours it only while the slave is attached and the
master grants that request, and the attachment record lists the rest under
`denied`. Only `speed` and `steering` act on the master today.

## 6. What a master must author

A tractor that wants to tow or carry MUST author, in addition to
`CONTROLLER_SPEC.md`:

### 6.1 Couplings

At least one `hitch` coupling (§2). Typical set for a tractor:

| Name | Type | Notes |
|---|---|---|
| `rear_three_point` | `three_point_mounted` | `lift` targets the lower-link lift joint. |
| `rear_drawbar` | `drawbar` | Same neighbourhood, different prim. |
| `front_three_point` | `three_point_mounted` | optional |
| `pickup_hitch` | `hitch_hook` | optional |

Only one of `rear_three_point` and `rear_drawbar` can be occupied at a time
if their prims overlap; author `gearbox:coupling:excludes = [<other>]` to say
so.

### 6.2 Service controllers

Service couplings need a controller on the master that does the work:

| Controller type | Drives | Command payload |
|---|---|---|
| `builtin:hitch` | the `lift` joint: position 0..1, `float` mode, `draft` target | `{ "position": 0.3 }` or `{ "float": true }` |
| `builtin:pto` | a revolute joint at the PTO stub, velocity in rpm, engaged flag | `{ "rpm": 540, "engaged": true }` |
| `builtin:hydraulic_valve` | a named slave joint, flow −1..1 mapped to joint velocity | `{ "flow": 0.5 }` |

When a slave attaches, the master's PTO joint is coupled to the slave joint
named in the slave's `pto` service coupling with a fixed-ratio motor, and
each hydraulic valve is bound to the slave joint the slave names for that
valve. These are runtime joints like the coupling joint and go away on
detach.

The slave names the joints on its `coupler` coupling:
`rel gearbox:coupling:ptoJoint` is the shaft the master's PTO spins and
`rel gearbox:coupling:valveJoints` lists, in valve order, the joints the
master's `builtin:hydraulic_valve` controllers move. The runtime sets the
slave joint's motor to the master's PTO speed or valve flow each step; a
`builtin:joint_velocity` on the same joint follows the PTO too unless it was
commanded a velocity directly.

## 7. What a slave must author

### 7.1 Every slave

- A full `CONTROLLER_SPEC.md` machine: `GearboxMachineAPI`, body, link tree
  with its own `base_link`. A trailer is a machine even when carried.
- At least one `coupler` coupling (§2). Its frame is the DDOP connector.
- `gearbox:machine:kind` SHOULD name the implement class: `trailer`,
  `sprayer`, `seeder`, `baler`, `mower`, `spreader`, `loader_tool`.

### 7.2 Trailers

Trailers are wheeled, so they author wheel joints as passive under the
machine roles. A trailer with a steered axle, brakes, or a tipping body is a
controlled slave and authors controllers for those:

| Controller type | Purpose |
|---|---|
| `builtin:trailer_steer` | steered axle: follows the hitch angle, or takes `angle_rad` when commanded |
| `builtin:brake` | `{ "level": 0..1 }` on the wheel joints |
| `builtin:joint_position` | tipping body, tailgate, unloading auger: `{ "position": 0..1 }` |

A tandem trailer authors a `hitch` coupling on its rear as well. Nesting
depth is not limited by this spec; the loop rule in §3.2 is the only guard.

### 7.3 Implements as a DDOP tree

A controlled implement MUST describe its working parts as a device element
tree so a DDOP can be generated from it and a DDOP can be turned into it.
Elements are prims under the slave's machine prim carrying
`GearboxElementAPI`. The parent element is the nearest ancestor element
prim, exactly as ISO 11783-10 stores a parent object id per element.

```usda
def Xform "boom" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxElementAPI"])
{
    token gearbox:element:type = "function"
    int gearbox:element:number = 2
    string gearbox:element:designator = "Boom"
    float gearbox:pd:ActualWorkingWidth = 24.0

    def Xform "section_0" (prepend apiSchemas = ["GearboxElementAPI"])
    {
        token gearbox:element:type = "section"
        int gearbox:element:number = 4
        float gearbox:pd:ActualWorkingWidth = 3.0
        double3 xformOp:translate = (-2.5, 10.5, 0.0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
}
def Xform "tank" (prepend apiSchemas = ["GearboxElementAPI"])
{
    token gearbox:element:type = "bin"
    int gearbox:element:number = 3
    float gearbox:pd:MaximumVolumeContent = 4000.0
    float gearbox:pd:ActualVolumeContent = 4000.0
}
```

| Property | Type | Status | Notes |
|---|---|---|---|
| `gearbox:element:type` | token | MUST | `device`, `function`, `bin`, `section`, `unit`, `connector`, `navigation`. DDOP types 1..7. |
| `gearbox:element:number` | int | MUST | Device element number, unique in the machine. Process data is addressed by it. |
| `gearbox:element:designator` | string | SHOULD | prim name | Human label. |
| `gearbox:pd:<DdiName>` | float or int | optional | One attribute per DDI the element exposes, named by the AgIsoStack `DataDescriptionIndex` enum entry. Settable ones become commands. |

Derived, never authored:

- The `device` element is the machine prim itself. Its designator is
  `gearbox:machine:kind` unless a `GearboxElementAPI` is applied there.
- The `connector` element is every `coupler` coupling.
- The `navigation` element is the link named `gps_link` or `gnss_link` if
  one exists.
- `DeviceElementOffsetX/Y/Z` of every element, connectors included, is its
  prim origin in `base_link`, converted to the ISO 11783-10 device frame.

ISO 11783-10 measures offsets from the **device reference point** (DRP) in
a frame with x forward, **y right, z down**. `base_link` is x forward, y
left, z up, so the export is `(x, y, z)_iso = (x, −y, −z)_base_link` in
millimetres. The standard fixes the DRP at the rear-axle centre for a
tractor and the front-axle centre for a wheeled implement, free otherwise.
`base_link` SHOULD be placed there. When it is not, author
`double3 gearbox:machine:drpOffset` in `base_link` metres and the export
subtracts it.

Element tree constraints, from ISO 11783-10:

- exactly one `device`, and it is the root;
- `connector` and `navigation` are direct children of `device`;
- `section` and `unit` hang under `function` or `device`;
- `bin` hangs under `device` or `function`;
- an element prim is not required to be a link, but if it is a link the link
  tree and the element tree MUST agree on the parent.

### 7.4 Process data controllers

Tool controllers speak process data, not twists. A controller with
`commandInterface = "process_data"` accepts:

```json
{ "element": 4, "ddi": "SetpointWorkState", "value": 1, "session_id": "..." }
```

and publishes `{ "element", "ddi", "value" }` on its state topic for every
DDI that changes. This is one bridge away from a task controller: the DDI
numbers and element numbers are the DDOP's own.

| Controller type | Acts on | Behaviour |
|---|---|---|
| `builtin:section_control` | `section` elements | `SetpointWorkState` per section sets `ActualWorkState`; `SectionControlState` on the parent function; totals `TotalArea` from speed × active width. |
| `builtin:rate_control` | `bin` element | `SetpointVolumePerAreaApplicationRate` → `ActualVolumePerAreaApplicationRate`; drains `ActualVolumeContent` from speed × active width × rate. |
| `builtin:joint_position` | a joint | folding, lifting, unloading. |
| `builtin:joint_velocity` | a joint | augers, rotors, conveyors. Coupled to the master's PTO when a `pto` service is bound. |

Work state and rate need ground speed. They read it from §5.2, so an
implement that is not attached, or is attached to a master that publishes no
speed, reports `ActualWorkState = 0`.

### 7.5 DDOP round-trip

| DDOP | USD |
|---|---|
| `Device` designator, structure label | machine prim, `gearbox:machine:kind`, `gearbox:machine:interfaceVersion` |
| `DeviceElement` type, number, designator, parent | `gearbox:element:*` on a prim, parent from prim hierarchy |
| `Connector` element + `ConnectorType` (DDI 157) | `coupler` coupling, `gearbox:coupling:type` |
| `DeviceElementOffsetX/Y/Z` (DDI 134..136) | prim origin in `base_link`, minus `drpOffset`, y and z negated, metres to millimetres |
| `DeviceProcessData` DDI, settable, trigger | `gearbox:pd:<Ddi>`; settable when a controller acts on it |
| `DeviceProperty` DDI + value | `gearbox:pd:<Ddi>` on an element with no controller |

Importing a DDOP creates one `Xform` per element, positioned by its offsets,
under the parent element's prim, with a placeholder box visual sized by
`ActualWorkingWidth` where present. Exporting walks the prims and emits
AgIsoStack `add_device_element` / `add_device_process_data` /
`add_device_property` calls in tree order. `AgIsoDDOPGenerator` can then load
the binary to inspect it.

## 8. Validation

In addition to `CONTROLLER_SPEC.md` §7.3, the runtime MUST refuse:

- a coupling prim that is not a link;
- a `hitch` and `coupler` attach request with different `type`;
- a three-point `hitch` whose `lift` is not a joint of the same machine;
- an attach that would create a loop or a second parent for a slave root;
- an element tree with no `device`, two `device`s, a `connector` not under
  `device`, or an element prim whose link parent differs from its element
  parent;
- a `gearbox:pd:<Ddi>` name that is not a `DataDescriptionIndex` entry;
- duplicate `gearbox:element:number` in one machine.

Warnings, not refusals: ISO category mismatch, capacity exceeded, a slave
requesting a function the master does not grant, an implement attached to a
master with no `speed` in its state.

## 9. Examples

### 9.1 Tractor rear end

```usda
def Xform "chassis" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"])
{
    def Xform "lower_links" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"])
    {
        token gearbox:link:role = "tool"
        def Xform "rear_three_point" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxCouplingAPI"])
        {
            token gearbox:coupling:side = "hitch"
            token gearbox:coupling:type = "three_point_mounted"
            int gearbox:coupling:isoCategory = 2
            token[] gearbox:coupling:services = ["pto", "hydraulic"]
            rel gearbox:coupling:lift = </robot/Joints/rear_lift>
        }
    }
    def Xform "rear_drawbar" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxCouplingAPI"])
    {
        token gearbox:coupling:side = "hitch"
        token gearbox:coupling:type = "drawbar"
        token[] gearbox:coupling:excludes = ["rear_three_point"]
    }
    def Xform "pto_stub" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"]) {}
}
# On the machine prim:
#   apiSchemas += GearboxControllerAPI:hitch_rear, GearboxControllerAPI:pto_rear
#   token gearbox:controller:hitch_rear:type = "builtin:hitch"
#   rel   gearbox:controller:hitch_rear:joint = </robot/Joints/rear_lift>
#   token gearbox:controller:pto_rear:type = "builtin:pto"
#   rel   gearbox:controller:pto_rear:joint = </robot/Joints/pto>
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
    rel gearbox:controller:tip:joint = </robot/Joints/tip>
    token gearbox:controller:tip:commandInterface = "process_data"

    def Xform "frame" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"])
    {
        token gearbox:link:name = "base_link"
        token gearbox:link:role = "base"
        def Xform "drawbar_eye" (prepend apiSchemas = ["GearboxLinkAPI", "GearboxCouplingAPI"])
        {
            token gearbox:coupling:side = "coupler"
            token gearbox:coupling:type = "drawbar"
            double3 xformOp:translate = (2.6, 0.0, 0.45)
            uniform token[] xformOpOrder = ["xformOp:translate"]
        }
    }
    def Xform "body" (prepend apiSchemas = ["PhysicsRigidBodyAPI", "GearboxLinkAPI"]) {}
    # wheels, joints ...
}
```

Attach over zenoh after loading both:

```python
s.put("gearbox/machines/tractor_1/attach", cbor2.dumps({
    "slave": "trailer_1", "hitch": "rear_drawbar", "coupler": "drawbar_eye",
    "teleport": True, "session_id": "demo"}))
s.put("gearbox/machines/tractor_1/tools/trailer_1/tip/cmd", cbor2.dumps({
    "element": 0, "ddi": "SetpointWorkState", "value": 1, "session_id": "demo"}))
```

### 9.3 Mounted sprayer

Coupler of type `three_point_mounted` on `base_link`; `boom` function with
sections as in §7.3; `tank` bin; controllers `sections`
(`builtin:section_control`) and `rate` (`builtin:rate_control`);
`requests = ["speed", "hitch:hitch_rear"]` so it can lift itself at the
headland. Attached, it is commanded at
`gearbox/machines/tractor_1/tools/sprayer_1/sections/cmd`.

## 10. Not covered

- Physical hitching by driving into the coupler. Attach is a request, not a
  contact event.
- Load transfer through the three-point linkage (draft sensing) beyond the
  `float` and `draft` modes of `builtin:hitch`.
- ISOBUS transport itself. The payloads are DDOP-shaped so a bridge to
  AgIsoStack is mechanical, but Gearbox does not speak CAN.
- Detaching by breaking force. `physics:breakForce` on the coupling joint
  is honoured if authored on the hitch, and reported as a detach.
