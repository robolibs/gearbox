# Gearbox machine asset specification

What a USD file must author for Gearbox to load it as a machine, and what the
runtime does with each authored value. Controllers, their behaviour and the
bus API are in [CONTROLLER_SPEC.md](CONTROLLER_SPEC.md). Physics is Molla.

Terms:

- **rejected**: the machine gets no agent (no topics, no commands from the
  bus, no tyres, no sensors, no service controllers). A `machine_rejected`
  scene event carries the reasons. See §16.
- `/machines/<id>/…`: the machine agent's topics. The agent name is the
  machine id unless a controller `namespace` says otherwise
  (CONTROLLER_SPEC §1).
- **scene units**: the stage's own length unit, converted with
  `metersPerUnit` (§1).
- Chassis frame: the machine body's local axes. Forward is **−Y**, left is
  **+X**, up is **+Z** (§18).

Source files: `controller.rs` (machine and controller discovery), `links.rs`
(link tree, sensors, couplings, elements), `devices.rs` (actuator devices),
`physics/attach.rs`, `physics/bodies.rs`, `physics/colliders.rs`,
`physics/joints.rs`, `physics/molla/` (UsdPhysics conversion),
`controller/wheel_forces.rs` (tyres), `controller/tracked.rs` (tracks).

## 1. Stage

Read from the layer metadata (`physics/attach.rs`).

| Key | Default | Effect |
|---|---|---|
| `metersPerUnit` | **0.01** when unauthored; env `BEVY_OPENUSD_METERS_PER_UNIT` overrides | Scales positions, joint frames, prismatic limits, linear drive gains and targets, velocities, centre of mass and inertia. An asset authored in metres without this key loads at 1/100 scale. Author `1`. |
| `kilogramsPerUnit` | 1 | Scales mass, density and inertia. |
| `upAxis` | any value other than `"Z"` is Y-up | `"Z"` rotates the stage −90° about X into the Y-up world. The drive controllers read the chassis frame of §18, which only lies flat in a Z-up stage. Author `"Z"`. |
| `defaultPrim` | none | When set, only its subtree is scanned for machines and static attachments; otherwise the whole stage is scanned. |

A `PhysicsScene` prim sets world gravity: the first one loaded wins for the
whole simulation, `physics:gravityMagnitude` defaults to 9.81 and is not
unit-scaled. Machine assets should not author one.

## 2. Machine prim

### 2.1 Detection

Every prim in the scanned subtree is a machine when **any** of these is
authored on it:

- `GearboxMachineAPI` in `apiSchemas`
- `gearbox:machine:kind`
- `gearbox:machine:idPolicy`
- a target on `rel gearbox:machine:body`

Author the API schema. Several machines may sit side by side in one file (a
yard with a tractor and a trailer). Machine prims must not nest: every scan
(link tree, devices) covers all prims under a machine prim, including those
of a machine inside it.

A prim with `PhysicsArticulationRootAPI` that is not itself a Gearbox machine
prim and has UsdPhysics joints (with both bodies) below it can become an
Isaac Sim compatibility machine (§2.4).

### 2.2 Attributes

All on the machine prim. Relationships are read in full (`rel[]`) or by their
first target (`rel`). Targets starting with `/robot/` are rebased onto the
machine prim when the machine prim is not `/robot`.

| Attribute | Type | Default | Effect |
|---|---|---|---|
| `gearbox:machine:id` | string or token | derived from the prim path (below) | Machine id. A `machine_id` load prop replaces it (CONTROLLER_SPEC §1). Do not author it in reusable assets. |
| `gearbox:machine:idPolicy` | token | `prim_path` | Detection only. |
| `gearbox:machine:kind` | token | none | Free label: logs, `MachineInfo`, scene list. |
| `gearbox:machine:body` | rel | none | The chassis body. Required in practice, see §2.3. |
| `gearbox:machine:role:*` | rel[] | [] | Joint roles, §7. |
| `gearbox:machine:grants` | token[] | [] | Slave requests this machine accepts when it is a master: `speed`, `steering`, `hitch:<instance>`, `pto:<instance>`, `aux_valve:<n>` (n counts the `builtin:hydraulic_valve` controllers from 0 in instance-name order). CONTROLLER_SPEC §8. |
| `gearbox:machine:tracks` | string (JSON) | none | Tracked drive definition, §11. |
| `gearbox:machine:interfaceVersion` | token | none | Parsed, unused. |
| `gearbox:machine:upAxis` | token | none | Parsed, unused; the stage `upAxis` applies. |
| `gearbox:machine:visuals`, `:colliders`, `:sensors` | rel[] | [] | Parsed, unused. |
| `gearbox:controller:<n>:*` | | | Controller instances, CONTROLLER_SPEC §2. |

Derived id: the composed prim path, lower-cased, every run of
non-alphanumeric characters replaced by one `_`, leading and trailing `_`
trimmed; `machine` if nothing is left. `/robot` → `robot`,
`/World/Tractor_01` → `world_tractor_01`. Machine ids must be unique in the
scene: two machines with one id share one agent name and only one of them
gets an agent. Load the same asset twice with distinct `machine_id` props.

### 2.3 The machine body

`gearbox:machine:body` names the chassis rigid body. These use it and never
fall back to a controller's `body`:

- tyre preparation and registration (§8), including trailers;
- the chassis inertia guard (§4);
- the derived link tree's base (§3.2);
- `builtin:trailer_steer` heading and the fallback `/state` pose (both fall
  back to the base link's body);
- wheel-track heading for the terrain;
- the drive controllers, unless the controller authors its own `body`.

A machine without it gets no tyres: its wheels are plain rigid colliders.

### 2.4 Isaac Sim compatibility

For an articulation root (including one inside a Gearbox machine, such as a
chassis carrying `PhysicsArticulationRootAPI`) with UsdPhysics joints below
it, some of them identified as wheel or steering joints (below), the runtime
synthesises a machine with `kind = "isaac_articulation"`, `interfaceVersion =
"isaac_compat:v0"`, an empty link tree (never rejected) and one enabled
`drive` controller of type `builtin:ackermann_cmd_vel` (`commandInterface =
cmd_vel`, `stateInterfaces = [pose, velocity, joint_state]`,
`steeringGeometry = ackermann`, `maxSteerDeg = 45`).

- Joint roles come from any property whose name contains `jointNames`,
  `joint_names`, `dofNames` or `dof_names`, anywhere in the stage. When the
  prim path, type, property names or string values contain `steer` or
  `position`, those names are steering joints; `wheel`, `velocity` or `drive`
  make them driven wheel joints.
- Without such arrays: joints whose path contains `wheel` are driven; paths
  containing `steer`, `knuckle` or `upright` are steering joints.
- The body is the shallowest rigid body under the root, avoiding paths that
  look like wheels or steering.

This is best effort; mislabels are expected on unfamiliar exports.

## 3. Link tree

Every machine has a tree of links rooted at `base_link`. Links are what
`/links`, `/tf`, sensors, named values, couplings and elements refer to. The
tree is answered on `/machines/<id>/links` (CONTROLLER_SPEC §9.6).

### 3.1 Marking

A prim under the machine prim (or the machine prim itself) is **marked** when
it has `GearboxLinkAPI` in `apiSchemas`, or an authored `gearbox:link:name`
or `gearbox:link:role`.

| Attribute | Type | Default | Effect |
|---|---|---|---|
| `gearbox:link:name` | token | `base_link` for the base; otherwise the prim name lower-cased, non-alphanumeric runs → `_`, trailing `_` trimmed (`link` if empty) | Must match `[a-z0-9_]+` and be unique in the machine. |
| `gearbox:link:role` | token | `base` for the derived base (§3.2); `tool` for couplings and elements; else `link` | One of `base`, `link`, `wheel`, `steer`, `sensor`, `tool`. Anything else is an error. |
| `gearbox:link:parent` | rel | derived (§3.3) | Replaces the derived parent. Must target another link of this machine. |

What the roles do at runtime:

| Role | Runtime use |
|---|---|
| `base` | Root of the tree. Its body is the fallback when `gearbox:machine:body` is missing (trailer steer heading, `/state`). |
| `wheel` | The body becomes a tyre (§8) even when no controller lists its joint. Wheel links get `slip`, `normal_force`, `slip_angle` and `tyre_*` values, press the terrain, and are lifted clear of the ground on a teleport attach. |
| `sensor` | `gearbox:sensor:*` is read and the link is simulated (§13). |
| `steer` | Descriptive: TF overlay colouring only. |
| `link`, `tool` | Descriptive. |

### 3.2 Strict and derived trees

- **Strict**: as soon as one prim under the machine is marked. The links are
  the marked prims plus every coupling (§14) and element (§15).
  - Every `PhysicsRigidBodyAPI` prim under the machine must be marked.
  - Every coupling must be marked.
  - Exactly one link must have `role = "base"`. Its name defaults to
    `base_link`, and an authored name must be `base_link`. The base is never
    derived in a strict tree: author the role.
- **Derived**: when nothing is marked. The links are every rigid body plus
  couplings and elements. The base is the machine body if it is a link, else
  the only rigid body that is never a joint's `physics:body1`; with neither,
  the machine is rejected ("no link has role = base"). A warning is logged
  and `MachineInfo` reports `links_derived = true`. No link has role
  `wheel`, so a derived machine gets tyres only for joints its controllers
  list (§8.1).

**The base link must be the chassis body itself**: author
`GearboxLinkAPI` and `gearbox:link:role = "base"` (optionally
`gearbox:link:name = "base_link"`) on the prim `gearbox:machine:body`
targets.
A `base_link` Xform below the chassis body does not work: the chassis then
has no joint parent and no ancestor link and the machine is rejected
("not connected to base_link"). Nothing checks that the base is the machine
body.

### 3.3 Parents

In this order:

1. `gearbox:link:parent`, when authored. The link then has a static offset
   and no joint, even if a joint names it as `body1`.
2. A joint: for every prim of a `Physics*Joint` type under the machine with
   both bodies authored, `physics:body1` is a child of `physics:body0`. The
   first such joint in prim order wins, including one marked
   `physics:excludeFromArticulation` and joint types physics skips (§6.1).
   Author `body0` on the parent side.
3. The nearest ancestor prim that is a link, with a static offset composed
   from the `xformOp`s in between (`translate`, `orient`, `rotateXYZ`,
   `rotateX/Y/Z`, `transform`, `!invert!` ops; scale and pivots ignored).

The base link has no parent.

### 3.4 Validation

Each failure is an error and rejects the machine:

- an unknown `gearbox:link:role`;
- (strict) a rigid body without `GearboxLinkAPI`, or a coupling that is not
  marked;
- a link name that is not `[a-z0-9_]+`, or a name used twice;
- no base link, more than one, or a base not named `base_link`;
- (strict) a joint whose `physics:body0` or `physics:body1` is not a link of
  this machine (including a body outside the machine);
- a link that is `body1` of two or more joints that are not
  `excludeFromArticulation` (a kinematic loop);
- a `gearbox:link:parent` that is not a link of this machine;
- a link with no joint parent and no ancestor link;
- a link not reachable from `base_link` (a parent cycle).

Not validated: that role relationships (§7) point at `wheel` or `steer`
links, joint axes of wheel or steer links, element number uniqueness, and
that the base is the machine body.

## 4. Bodies and mass

A body is a prim with `PhysicsRigidBodyAPI` applied.

| Attribute | Effect |
|---|---|
| `physics:rigidBodyEnabled = false` | Fixed body. |
| `physics:kinematicEnabled = true` | Kinematic body. |
| `physics:startsAsleep` | Honoured. |
| `physics:velocity` | Initial velocity, scaled by `metersPerUnit`. |
| `physics:angularVelocity` | Initial angular velocity, deg/s → rad/s (not rotated into the world basis). |
| `physics:simulationOwner` | Ignored. |

Dynamic bodies get fixed damping: linear 0.1, angular 0.5.

`PhysicsMassAPI` on the body prim (on collider prims it is ignored):

| Attribute | Default | Effect |
|---|---|---|
| `physics:mass` | none: a dynamic body gets 0.001 kg | Body mass, × `kilogramsPerUnit` for kg. |
| `physics:centerOfMass` | (0, 0, 0) | Body frame, scene units. |
| `physics:diagonalInertia` | `0.4 · m · 0.01` on every axis | Principal inertia. The default is far too small; always author it. |
| `physics:principalAxes` | | Ignored (rigid bodies). |
| `physics:density` | | Ignored, as is material density. |

Every collider also adds its volume × **1 kg/m³** to its body, whatever is
authored. This is negligible except for very large hulls.

Nested rigid bodies (a body prim under another body prim) are supported.

**Chassis inertia guard.** Once the machine has an agent, if the machine
body weighs at least 100 kg and any diagonal component of its inertia is
below 25 % of a box estimate from its collider bounds
(`m/12 · (b² + c²)`), that component is replaced by the estimate and a
warning names the machine. Fix the asset; the guard only keeps it usable.

## 5. Colliders and materials

A collider is a prim with `PhysicsCollisionAPI`. It belongs to the nearest
body at or above it.

| Prim type | Shape |
|---|---|
| `Cube` | Box, `size` (default 2) × the prim's scale relative to its body. |
| `Sphere` | Ball, `radius` × the largest scale component. |
| `Capsule` | Capsule; prim scale ignored. |
| `Cylinder`, `Cone` | Cylinder along `axis` (a cone is a cylinder). |
| `Plane` | A 100 m × 100 m slab. |
| `Mesh` | Per `physics:approximation`, below. |
| other | A descendant mesh if there is one, else a unit cube. |

| `physics:approximation` | Dynamic body | Static body |
|---|---|---|
| `none`, `meshSimplification` | convex hull (warning) | triangle mesh |
| `convexHull`, `boundingSphere`, `boundingCube` | convex hull | convex hull |
| `convexDecomposition` | convex decomposition (needs an indexed mesh) | convex decomposition |

- `physics:collisionEnabled = false` is **ignored**: every collider is built.
- Defaults: friction 0.5, restitution 0, friction combine Average.
- Physics material: `material:binding:physics`, else `material:binding`.
  Friction is `physics:dynamicFriction`, else `physics:staticFriction`, else
  0.5; `physics:restitution` is honoured; `physics:density` is ignored.
- `PhysicsFilteredPairsAPI` (`physics:filteredPairs`) filters contacts
  between the two bodies.
- Colliders under a `PhysicsArticulationRootAPI` prim share a collision group
  derived from the root's entity id and never touch each other. Two roots can
  hash to the same group; their machines then never collide with each other.
- `PhysicsCollisionGroup` prims are parsed and not used.
- Jointed bodies never collide with each other. Once a machine has an agent,
  every pair of its own bodies is filtered, so it touches only the ground and
  other machines. While attached, every master × slave body pair is filtered.

Spawn alignment moves a loaded machine up or down so its lowest collider
whose name contains `tire`, `tyre` or `wheel` sits 0.03 m above the terrain
(all colliders when none match). Name tyre colliders accordingly.

## 6. Joints

### 6.1 Types

| Prim type | Runtime |
|---|---|
| `PhysicsRevoluteJoint` | Hinge about `physics:axis`. Limits and drives (§6.3, §6.4). |
| `PhysicsPrismaticJoint` | Slider along `physics:axis`. Limits and drives. |
| `PhysicsFixedJoint` | Keeps the frame **translations only**: `localRot0/1` are ignored and the child is locked in the parent body's orientation. |
| `PhysicsSphericalJoint` | Ball joint with both frames; cone limits unused. |
| `PhysicsDistanceJoint`, `PhysicsJoint` (D6) | **Skipped** with a warning: no constraint is built. They still define link-tree parents. |

### 6.2 Common attributes

| Attribute | Effect |
|---|---|
| `physics:body0`, `physics:body1` | body0 is the parent. With only one body authored, that body is made Fixed (anchored to the world). A body that never resolves leaves the joint pending. |
| `physics:localPos0/1`, `physics:localRot0/1` | Joint frames in each body; positions scaled by `metersPerUnit`. |
| `physics:axis` | `"X"` (default), `"Y"`, anything else is Z. |
| `physics:jointEnabled = false` | Joint skipped. |
| `physics:excludeFromArticulation = true` | Loop-closure joint: a soft constraint (30 Hz, damping ratio 1) outside the reduced-coordinate tree. Motor and brake devices refuse it, tyres need a tree spin joint (§8), and the parking hold falls back to a velocity brake on it. A body that is `body1` of two joints without this flag is a link-tree error (§3.4); a closed loop of such joints also makes Molla refuse the joint and the simulator panics (CONTROLLER_SPEC §11). |
| `physics:collisionEnabled` | Ignored; jointed bodies never collide. |
| `physics:breakForce`, `physics:breakTorque` | Ignored by rigid joints (read only by FEM validation). |
| `physics:coneAngle0Limit/1Limit`, `physics:minDistance/maxDistance` | Parsed; not used by rigid joints. |
| `PhysicsLimitAPI:<dof>` | Only meaningful on generic joints, which are skipped. |

### 6.3 Limits

`physics:lowerLimit` and `physics:upperLimit` apply to revolute (degrees) and
prismatic (scene units) joints, and **both must be authored**; one alone is
ignored.

### 6.4 Drives

`PhysicsDriveAPI:<dof>` must be applied in `apiSchemas`; attributes without
the applied schema are not found.

- The first applied drive whose dof fits the joint is used: `angular`,
  `rotX`, `rotY`, `rotZ` for revolute; `linear`, `transX`, `transY`,
  `transZ` for prismatic.
- `drive:<dof>:physics:targetPosition` authored: a position drive with
  `stiffness` and `damping`. Otherwise `targetVelocity` authored: a velocity
  drive with `damping`. **Neither authored: no drive mode is set**, only
  `maxForce`. Author `targetPosition = 0` for a spring at rest.
- Units: rotational stiffness and damping are per degree in USD and × 180/π
  here; linear ones are ÷ `metersPerUnit`. Targets: degrees → radians, or ×
  `metersPerUnit`. `maxForce` is not scaled; unauthored means unlimited.
- `drive:<dof>:physics:type` (`force`/`acceleration`) is **ignored**. The
  drive is a **Force**-model drive only when `localRot0` equals `localRot1`
  (within 1e-4); otherwise it is an **Acceleration**-model drive whose gains
  act per unit of joint inertia. Keep both frame rotations equal on every
  driven joint.
- A `gearbox:motor:*` device on the joint replaces its drive every physics
  step (§12.3).

## 7. Joint roles

Relationships on the machine prim. Each target is a joint prim.

| Role | Read by |
|---|---|
| `gearbox:machine:role:poweredWheelJoints` | Driven wheels of `builtin:ackermann_cmd_vel` (unless the controller authors `driveWheelJoints`) and of diff drive in `wheels` mode; tyre pairs (§8.1); visual spin in diff `chassis` mode; `builtin:brake` fallback; the motor ownership rule (§12.4). |
| `gearbox:machine:role:passiveWheelJoints` | Tyre pairs; idle-rolled while an Ackermann machine drives and held while it is parked; visual spin in diff `chassis` mode; `builtin:brake` fallback; the ownership rule. |
| `gearbox:machine:role:steeringJoints` | Steering solver and legacy steering; steered-knuckle detection (derived wheelbase, steer torque cap); `builtin:trailer_steer` fallback; the ownership rule. |
| `gearbox:machine:role:brakeJoints` | `builtin:brake` when the controller has no `wheelJoints`. |
| `gearbox:machine:role:suspensionJoints` | A warning when a target is not a `PhysicsPrismaticJoint` or `PhysicsRevoluteJoint`; the ownership rule (no motor device may sit on them). Nothing drives them. §10. |
| `gearbox:machine:role:toolJoints` | The **first** entry is the joint of any service controller (hitch, pto, hydraulic_valve, joint_position, joint_velocity) that has no `target`. |

## 8. Wheels and tyres

Every registered wheel is a Molla pressure tyre. There is no solid-wheel
model. The pressure model, controls and telemetry are in
[TYRE_PRESSURE.md](TYRE_PRESSURE.md); drive torque limits in
[DRIVETRAIN.md](DRIVETRAIN.md).

### 8.1 Which bodies are wheels

Once the machine has an agent and `gearbox:machine:body` resolves:

- the body of every link with `role = "wheel"`;
- for every controller of the machine (any type), the wheel side of every
  joint in: the machine's powered and passive roles, the controller's
  `driveWheelJoints`, `passiveWheelJoints`, `wheelJoints` and the four
  `front/rear Left/Right WheelJoint` relationships. The wheel side is the
  body that is not the chassis; for a knuckle ↔ wheel joint it is the body
  with the larger collider. A pair already represented by a `wheel` link is
  not added twice.

**A machine without controllers (a trailer) must mark its wheel links
`role = "wheel"`**; otherwise its wheels are plain rigid colliders with no
tyre model.

Machines with `gearbox:machine:tracks` skip everything below (§11).

### 8.2 Preparation

Once per machine, every wheel body gets CCD, and each of its colliders gets
friction raised to at least 1.1 with combine rule Min and restitution 0.
`Cylinder` colliders get a rounded edge (5 % of the radius). These settings
apply to contacts with colliders that are not registered ground (props,
other machines, the machine's own obstacles).

### 8.3 Registration

Every frame, per wheel:

- The wheel body needs at least one collider. The **largest** collider (by
  local bounding box) gives the tyre geometry: its centre is the hub; a
  `Cylinder` gives the axle (its own axis), the width (its height) and the
  radius; any other shape gives the thinnest axis of its local box as axle
  and width, and its largest half-extent as radius.
- A joint touching the wheel body must leave exactly the rotation about its
  axis free (a revolute joint). It must be a tree joint: with
  `excludeFromArticulation` the tyre produces no force.
- The rolling direction (that joint's axis in its parent frame × world up)
  must align with the chassis forward direction by at least 0.1 (cosine),
  else registration is refused with a warning. Its sign sets the wheel's
  drive sign, so either axis direction drives forward.
- Radius: measured once from the visual meshes under the wheel body whose
  prim or entity name contains `tyre`, `tire`, `tread`, `mould_line` or
  `bkt_fl630` and no ancestor between it and the body contains `rim`, `hub`,
  `collision` or `collider`: the largest distance of a vertex from the axle
  through the hub. Without such meshes: the largest collider's radius.
- Supported mass: `max(machine mass / wheel count, wheel mass, 1 kg)`.
- Tyre properties from the wheel link's values (§8.4).

Only colliders registered as wheel ground carry the tyre model: generated
terrain, the flat ground and USD terrain meshes. There, wheel ↔ ground
penalty contacts are replaced by tyre forces, and friction comes from the
ground collider (or its friction grid), not from the tyre collider.

Drive controllers use the tyre's loaded radius for wheel speeds; before a
tyre reports one they use the largest collider's radius, and below 0.05 m
the controller's `wheelRadius`.

### 8.4 Tyre values

`gearbox:value:*` on the wheel link (§15). All are optional.

| Name | Default | Meaning |
|---|---|---|
| `tyre_pressure_bar` | 1.8 | Initial gauge pressure. |
| `tyre_min_pressure_bar` | 0.5 | Lowest allowed pressure. |
| `tyre_max_pressure_bar` | 4.0 | Highest allowed pressure. |
| `tyre_pressure_rate_bar_s` | 1.0 | Inflation and deflation rate. |
| `tyre_width_m` | the largest collider's width (§8.3) | Tyre width. |
| `tyre_carcass_stiffness_pa_m` | 400 000 | Carcass stiffness. |
| `tyre_tread_stiffness_n_m3` | 2 600 000 | Tread stiffness. |
| `tyre_damping_ratio` | 0.7 | Vertical damping ratio. |
| `tyre_hysteresis_fraction` | 0.1 | Rolling hysteresis. |
| `tyre_axle` | inferred | Axle id, a whole number 1–65535. Wheels within 0.25 m along chassis forward share an inferred axle; ids count from the front. **An invalid value disables tyre registration for the whole machine** (warning). Partial ids in one row are an error with the same effect. |
| `tyre_target_pressure_bar` | runtime | Live target; restored from a saved configuration. |

The runtime writes `tyre_*` telemetry, `slip`, `normal_force` and
`slip_angle` onto every wheel link (TYRE_PRESSURE.md).

## 9. Steering joints

The steering solver is described in [STEERING.md](STEERING.md); the
controller side in CONTROLLER_SPEC §4.2. The asset rules:

- A steered wheel is two links in series: `chassis → knuckle` (revolute
  about up, limited) and `knuckle → wheel` (revolute about the axle,
  unlimited). A wheel that does not steer joins the chassis (or a bogie)
  directly.
- Every steering joint listed (role and controller relationships) must
  resolve to a joint between two bodies. **If any one does not resolve, the
  solver is off for the whole controller** and the legacy mapping runs.
- The steer axis must be within 60° of chassis up (`|axis · up| ≥ 0.5`);
  either sign works in the solver. The legacy path assumes positive rotation
  about chassis +Z turns left.
- The joint limits, clamped to ±`maxSteerDeg` (at most 85°), must contain 0.
  A missing limit pair means ±`maxSteerDeg`.
- The steer joint's `body0` anchor is taken as the wheel's position: put the
  steer joint (`localPos0`) at the wheel centre in plan.
- Author `localRot0 == localRot1` if the joint's own `PhysicsDriveAPI`
  gains should steer it; otherwise the runtime's load-scaled servo does
  (CONTROLLER_SPEC §4.2.6).
- A pivoting axle is one more revolute between the chassis and the steer or
  wheel joints. A tandem of rigid axles should hang from a walking beam
  (`bogie` body on a revolute of a few degrees, the axle or wheel joints'
  `body0` on the bogie).

## 10. Suspension

Suspension joints are springs and pivots the runtime leaves alone. List them
in `gearbox:machine:role:suspensionJoints`.

- **Strut spring**: a `PhysicsPrismaticJoint`, `body0` the chassis (or axle
  or bogie), `body1` the wheel carrier or knuckle, `physics:axis` along up,
  both `physics:lowerLimit` and `physics:upperLimit` authored for travel.
- **Pivot**: a `PhysicsRevoluteJoint` for a pivoting axle or bogie, free or
  sprung.
- **Spring**: the joint's `PhysicsDriveAPI:linear` (or `:angular` for a
  pivot), applied, with `stiffness`, `damping`, and an authored
  `targetPosition` as rest position (0 is fine; without a target no spring
  exists). `maxForce` optional. Keep `localRot0 == localRot1`, or the gains
  act per unit of joint inertia (§6.4). Joint softness cannot be authored.
- Keep it a tree joint (no `excludeFromArticulation`).
- The wheel spin joint sits below the carrier; the carrier must not have a
  collider larger than the tyre (§8.1).
- In a strict tree the carrier body needs `GearboxLinkAPI`.
- A `gearbox:motor:*` device on a suspension joint rejects the machine
  (§12.4). `gearbox:brake:damping` is allowed and adds viscous damping.
- Nothing stops a service controller `target` or a `toolJoints` entry from
  naming a suspension joint; do not.
- Other joint types in the role only produce a warning.

The tyre's own vertical stiffness acts in series with the spring.

## 11. Tracks

`gearbox:machine:tracks` holds a JSON array of exactly two track
definitions (one left, one right) consumed by `builtin:tracked_cmd_vel`. The
format, telemetry and FEM options are in
[TRACKED_DRIVE.md](TRACKED_DRIVE.md) and [FEM_CONTACTS.md](FEM_CONTACTS.md).
Invalid JSON or geometry, a target outside the machine, or a body owned by
two tracks rejects the machine. Tracked machines get no tyres.

Track values on the carrier link (the definition's `link`):
`track_speed_gain` (120), `track_slip_damping` (8000),
`track_friction_long` (0.85), `track_friction_lateral` (0.65). Links whose
parent is the carrier link, with a body joined to the carrier and a
`rolling_radius_m` value > 0, become follower rollers.

## 12. Actuator devices

Molla's device layer runs the actuators of the Webots actuator set inside
each physics step: motors and brakes on joints, propellers, belts,
connectors, plus copters and aerodynamic drag.

### 12.1 Discovery

- Every prim **strictly under** the machine prim with any `gearbox:`
  property is examined. Its kind is the first namespace present, in this
  order: `gearbox:motor:` or `gearbox:brake:` (one joint device, either or
  both) > `gearbox:propeller:` > `gearbox:belt:` > `gearbox:connector:` >
  `gearbox:copter:` > `gearbox:drag:`. Other namespaces on the same prim are
  ignored. Prims with none of these are not devices.
- Name: `gearbox:device:name` (string or token), else the prim name. Names
  must be unique per machine.
- Errors reject the machine: a duplicate name, a motor or brake on a prim
  whose type is not `Physics…Joint`, a propeller without
  `thrustConstants`, a connector `type` other than symmetric, active or
  passive, a drag without a valid `area`, and the ownership rule (§12.4).
- Registration waits until every device's physics objects exist, then
  registers the machine's devices together. A device the backend refuses (a
  motor on a fixed, spherical or loop joint) is skipped with a warning; the
  others run. A device whose physics object never appears holds back **every
  device of the machine**: a motor or brake on a joint physics does not build
  (distance, D6, `jointEnabled = false`), a propeller, connector, copter or
  drag with no body at or above it, a belt with no collider.

Vector attributes accept `float2/3`, `double2/3`, `float[]` or `double[]`.

### 12.2 Authoring

| Device | Prim | Attributes (default) |
|---|---|---|
| Motor | a revolute or prismatic tree joint | `gearbox:motor:maxForce` N·m or N (10), `maxVelocity` rad/s or m/s (10), `acceleration` (unlimited; ≤ 0 means unlimited), `controlPID` 3 values (10, 0, 0), `minPosition`/`maxPosition` in rad or m (none; used only when both are authored and min < max) |
| Brake | a revolute or prismatic tree joint | `gearbox:brake:damping` N·m·s/rad or N·s/m (0; negative → 0) |
| Propeller | a prim at or below a body; +X is the shaft, the origin the centre of thrust | `gearbox:propeller:thrustConstants` 2 values (**required**), `torqueConstants` 2 values (0, 0), `maxVelocity` rad/s (100), `acceleration` (unlimited), `maxTorque` N·m (unlimited). `controlPID`, `minPosition`, `maxPosition` are parsed and have no effect. |
| Belt (conveyor, tank track) | a collider, or a prim with colliders below it | `gearbox:belt:direction` 3 values in the prim frame (1, 0, 0), `maxVelocity` m/s (10), `acceleration`, `controlPID`, `minPosition`/`maxPosition` |
| Connector | a prim at or below a body; +X points out of the docking face | `gearbox:connector:model` (""), `type` `symmetric`/`active`/`passive` (symmetric), `isLocked` (false; latches at registration), `autoLock` (false), `unilateralLock`/`unilateralUnlock` (true), `distanceTolerance` m (0.01), `axisTolerance`/`rotationTolerance` rad (0.2), `numberOfRotations` (4), `snap` (true), `tensileStrength`/`shearStrength` N (unbreakable; negative → unbreakable), `stiffness` Hz (30; ≤ 0 → 30) |
| Copter (multirotor) | a prim at or below the body the propellers push; +X forward, +Z up | `gearbox:copter:maxTiltDeg` (28.6), `maxSpeed` m/s (10), `maxClimb` m/s (3), `maxYawRate` rad/s (1.5), `velocityGain` 1/s (1.5), `disturbanceTime` s (0.5), `attitudeFrequency` rad/s (6), `yawGain` 1/s (4). A value ≤ 0 keeps the default. |
| Drag | a prim at or below a body; its origin is the centre of pressure | `gearbox:drag:area` `CdA` m² along the prim's X, Y, Z, all ≥ 0 (**required**), `density` kg/m³ (1.225; ≤ 0 → 1.225) |

Booleans must be `bool`; numbers may be float, double, int or uint. A device
frame is the prim's pose relative to its body when registered.

### 12.3 Behaviour

- **Motors**
  - A new motor holds the joint position it finds.
  - Position control: a PID on the position error gives a speed, capped by
    the speed setting (default `maxVelocity`) and ramped by
    `acceleration`; position targets are clipped to
    `minPosition`/`maxPosition`.
  - Velocity control: runs at the commanded speed, clipped to
    ±`maxVelocity`.
  - Force control applies an effort clipped to ±`maxForce`.
  - The drive is a Force-model velocity drive with effort up to
    `maxForce`. It **replaces the joint's whole motor at the start of every
    physics step**: the joint's USD drive and any runtime write to its motor
    (drive, steering, parking, service servo, track roller) are lost.
- **Brakes** add viscous damping `−c·ω` on the joint through a channel
  separate from the motor; it stays until changed with an `/actuate`
  `brake` prop. `builtin:brake` does not use it.
- **Propellers**: the rotor speed follows its command. The body takes the
  thrust `t1·|ω|·ω − t2·|ω|·V` along +X at the centre of thrust and the
  reaction torque `−Q`, `Q = q1·|ω|·ω − q2·|ω|·V`, with `V` the speed of
  advance from the body's airspeed.
- **Belts** drag whatever touches their surface along their direction by
  friction.
- **Connectors**
  - A connector links to a compatible peer lined up within its tolerances:
    same model, symmetric–symmetric or active–passive.
  - It links when latched, right away or, with `autoLock`, as the peer
    arrives. Unlatching unlinks it.
  - The link breaks when the load pulling the faces apart or sliding them
    exceeds the summed strength of the latched sides.
- **Copters**
  - A copter sets the speed of every propeller of its machine each step. It
    holds a velocity (forward, left, up, yaw rate in the heading frame) or an
    attitude (roll right side down, pitch nose down, yaw rate, climb).
  - A disturbance observer learns the steady push of wind, drag or payload
    and leans into it.
  - Rotors spin opposite ways by the sign of their thrust constant.
  - It advertises a `builtin:copter` controller on `cmd_vel`
    (CONTROLLER_SPEC §4.5).
- **Drag and wind**
  - Each axis of a drag frame takes `−½·ρ·CdA·|a|·a`, with `a` the frame
    origin's airspeed: body velocity minus wind.
  - The wind is the weather's (heading, speed, gustiness). At a drag body it
    gusts as the vegetation shows it: the same gust map, scrolled downwind,
    sampled at the body. Between gusts it drops to `1 − gustiness` of its
    speed.

Commands and readings: CONTROLLER_SPEC §9.8.

### 12.4 Ownership rule

A `gearbox:motor:*` device replaces its joint's motor every step, so it may
not sit on a joint something else drives. Discovery rejects the machine
when a motor device's joint is:

- listed in `gearbox:machine:role:suspensionJoints`, or
- owned by an **enabled** drive controller (`builtin:ackermann_cmd_vel`,
  `builtin:diff_drive_cmd_vel`, `builtin:tracked_cmd_vel`): the machine's
  powered, passive and steering roles, and that controller's
  `driveWheelJoints`, `passiveWheelJoints`, `wheelJoints`, `steerJoints`,
  `steerLeftJoint`, `steerRightJoint` and the four corner wheel joints.

Brake-only devices are allowed on any joint. Not covered by the rule: track
sprocket and roller joints, `builtin:brake` and `builtin:trailer_steer`
joints.

### 12.5 Devices and service controllers

A service controller whose joint has a motor device commands the device
(`Position` or `Velocity`) instead of writing its own servo: the device's
PID, speed, acceleration, limits and `maxForce` apply and the controller's
force caps do not. Author `maxForce` for the load; the default 10 N·m moves
almost nothing. Details in CONTROLLER_SPEC §5.2.

A motor holds its commanded speed against everything the step puts on its
joint (gravity, what hangs on it through soft and loop joints, the other
motors) up to `maxForce`; beyond that it stalls at `maxForce`. Rate it like
the machine's own actuator: a hitch lift at the tractor's rated lift
capacity, on the joint that carries the implement (TOOLS_SPEC §2.1).

## 13. Sensor links

A link with `role = "sensor"` can carry a simulated sensor. Molla computes
every reading from the physics scene; Gearbox supplies the environment (site
frame, planet position, sun and sky, contact forces, radio packets). Each
sensor streams on `/machines/<id>/sensors/<link name>` at its own
simulated-time rate (payloads in CONTROLLER_SPEC §9.7).

- Readings are in the link's own frame. Author sensor links with REP-103
  axes: +X forward, +Y left, +Z up. Under a chassis that faces −Y, rotate the
  link −90° about Z.
- The sensor rides the body of the nearest link at or above it that has a
  rigid body. A sensor link with no such ancestor, or a `position` sensor
  without a joint between its body and its parent link's body, stops **every**
  sensor of the machine (runtime error, not a rejection).
- Sensors need a GPU render device.
- `gearbox:sensor:*` on a link whose role is not `sensor` is ignored.

### 13.1 Kind

- `gearbox:sensor:kind` (token): `imu`, `lidar`, `camera`, `range_finder`,
  `accelerometer`, `gyro`, `inertial_unit`, `compass`, `gps`, `distance`,
  `light`, `position`, `radar`, `touch`, `receiver`, `emitter`. Any other
  token is an error (rejects the machine). `range_finder` is a camera whose
  default render is `geometry` with depth only.
- Without it, the link name's prefix decides: `lidar` or `laser` → lidar;
  `cam_` or `range_finder` → camera (`range_finder` as above); `gnss` → gps;
  otherwise any kind name above (so `camera_link` is a camera, `imu_link` an
  IMU). A sensor link whose name implies nothing is a warning and is not
  simulated.
- A camera's optical frame (`<name>_optical`) must not be a sensor link: its
  name would make it a second camera. Author it with role `link`, or leave
  it out; cameras look along their link's +X.
- `gearbox:sensor:type` is only the Webots variant of distance, touch,
  receiver and emitter links. Any other value, or any value on another kind,
  is an error.

| Kind | `gearbox:sensor:type` values (default first) |
|---|---|
| `distance` | `laser`, `infra_red` (or `infrared`), `sonar` |
| `touch` | `bumper`, `force`, `force3d` (or `force_3d`, `force-3d`) |
| `receiver`, `emitter` | `radio`, `infrared` (or `infra_red`), `serial` |

### 13.2 Attributes

Out-of-range values are errors that reject the machine; they are not clamped.
Numeric attributes may be any number type; "whole" means no fractional part.

| Attribute | Kinds | Default | Range and meaning |
|---|---|---|---|
| `gearbox:sensor:rate_hz` | all | 100 (imu, accelerometer, gyro, inertial_unit, position, touch); 50 (receiver, emitter); 20 (compass, distance, radar); 10 (lidar, gps, light); 5 (camera) | Samples per simulated second, `(0, 10000]`. Receivers publish each packet as it arrives. |
| `gearbox:sensor:columns` | lidar | 360 | Azimuth columns, whole `1..=8192`. |
| `gearbox:sensor:rows` | lidar | 16 | Zenith rows, whole `1..=512`; `columns × rows ≤ 1048576`. |
| `gearbox:sensor:hfov_deg` | lidar | 360 | Horizontal field of view centred on +X, `(0, 360]`. A full circle does not repeat its first column. |
| `gearbox:sensor:hfov_deg` | radar | 45 | `(0, 360]`. |
| `gearbox:sensor:vfov_deg` | lidar | 30 | Centred on the XY plane, `[0, 180)`; 0 only with one row. |
| `gearbox:sensor:vfov_deg` | camera | 60 | `(0, 180)`. |
| `gearbox:sensor:vfov_deg` | radar | 10 | `(0, 360]`. |
| `gearbox:sensor:range_m` | lidar, camera | 100 | `> 0`, finite. Misses report infinite range or depth. |
| `gearbox:sensor:range_m` | distance, radar | 10, 50 | `> 0`, finite. |
| `gearbox:sensor:range_m` | receiver, emitter | unlimited | `> 0`; a negative value means unlimited. |
| `gearbox:sensor:width`, `height` | camera | 128, 96 | Pixels, whole `1..=4096`. |
| `gearbox:sensor:render` | camera | `optimized` (`geometry` for a range finder) | `optimized` (or `optimised`): Bevy render of the visual scene without grass, clouds, bloom or multisampling. `full`: Bevy render with grass, bloom and multisampling. `geometry`: Molla trace of the collision shapes with a flat colour per shape. |
| `gearbox:sensor:channels` | camera | `color_depth` (`depth` for a range finder) | `color` (or `colour`), `depth`, `color_depth` (or `colour_depth`). Depth always comes from the Molla trace of the collision shapes. |
| `gearbox:sensor:recognition` | camera | false | Bool. Also stream the objects in view. |
| `gearbox:sensor:rays` | distance | 1 | Whole `1..=64`: the axis, then a ring at half the aperture. |
| `gearbox:sensor:aperture_deg` | distance, receiver, emitter | 0 (distance), 90 (radio links) | Full cone, `[0, 360]`. |
| `gearbox:sensor:min_range_m` | radar | 1 | `[0, range_m)`. |
| `gearbox:sensor:channel` | receiver, emitter | 0 | Whole `≥ -1`; −1 hears and reaches every channel. |

Radio reaches receivers on its channel within range; infra-red also needs
each end inside the other's aperture and a clear line of sight; serial
reaches every serial receiver on its channel.

```usda
def Xform "lidar_link" (prepend apiSchemas = ["GearboxLinkAPI"])
{
    token gearbox:link:role = "sensor"
    token gearbox:sensor:kind = "lidar"
    float gearbox:sensor:rate_hz = 10
    int gearbox:sensor:columns = 360
    int gearbox:sensor:rows = 16
    float gearbox:sensor:vfov_deg = 30
    float gearbox:sensor:range_m = 60
    double3 xformOp:translate = (0, -0.6, 2.4)
    float3 xformOp:rotateXYZ = (0, 0, -90)
    uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]
}
```

## 14. Couplings and attachments

Hitches (master side) and couplers (slave side) are links with coupling
attributes. The attach model, joint per coupling type, command routing and
the master/slave exchange are in [TOOLS_SPEC.md](TOOLS_SPEC.md).

| Attribute | Prim | Type | Effect |
|---|---|---|---|
| `GearboxCouplingAPI` or `gearbox:coupling:side` | link prim | token | `hitch` or `coupler`, required; anything else is an error. A coupling is always a link (role `tool` by default) and must be marked in a strict tree. |
| `gearbox:coupling:type` | same | token | Required; one of `drawbar`, `three_point_semi_mounted`, `three_point_mounted`, `hitch_hook`, `clevis`, `piton`, `cuna`, `ball`, `chassis_mounted`, `pivot_wagon`, `fifth_wheel`, `loader_carriage`. Hitch and coupler must match to attach. |
| `gearbox:coupling:name` | same | token | Default: the sanitised prim name. Used in attach requests. |
| `gearbox:coupling:ptoJoint` | coupler | rel | Slave joint spun by the master's first engaged PTO while attached. |
| `gearbox:coupling:valveJoints` | coupler | rel[] | Slave joint n follows the master's hydraulic valve n. |
| `gearbox:coupling:stand` | coupler | rel | Parking stand body: its colliders are off while hitched, and the slave machine prim's variant set `coupling` is set to `hitched` (`parked` on detach). |
| `gearbox:coupling:topLink` | both | rel | Three-point hitches: the top link's free end on the master, the upper hitch point on the implement. With both, the implement pitches on the lower-link pins and the top link holds it. |
| `gearbox:coupling:variantSet`, `:variantNear`, `:variantFar` | hitch | token | The machine prim's variant set this hitch selects: `near` while a coupler of its type is within 3 m or attached, `far` otherwise. All three or none; a partial set is an error. |
| `gearbox:coupling:isoCategory`, `:capacityKg`, `:services`, `:lift`, `:excludes` | | | Parsed, unused. |

Only the positions of coupling prims are used; their rotations are ignored.

A static attachment in a loaded file: any prim with `GearboxAttachmentAPI`
or a `rel gearbox:attachment:hitch`, plus `rel gearbox:attachment:coupler`,
joins the two coupling prims once both machines have agents (attached like a
teleport, retried for up to 1200 frames). Both relationships are required.

## 15. Elements and named values

### 15.1 Elements

Working parts (sections, bins, functions) are links marked as elements.

| Attribute | Type | Effect |
|---|---|---|
| `GearboxElementAPI` or `gearbox:element:type` | token | `device`, `function`, `bin`, `section`, `unit`, `connector`, `navigation`; anything else is an error. An element is always a link (role `tool` by default). |
| `gearbox:element:number` | int or uint | Reported on `/links`. Uniqueness is not checked. |
| `gearbox:element:designator` | string or token | Default: the prim name. Reported on `/links`. |

`builtin:section_control` works on `function` and `section` links,
`builtin:rate_control` on a `bin` link (CONTROLLER_SPEC §6).

### 15.2 Named values

Every `gearbox:value:<Name>` attribute (float, double, int, uint or bool) on
a **link prim** is a named value of that link. Authored values are copied into
the live values once, when the machine gets its agent; `/cmd` link commands
and controllers change them afterwards, and `/state` publishes them as
`link.<link>.<Name>`. Values on prims that are not links are not read.

Values the runtime consumes:

| Name | Link | Default | Used by |
|---|---|---|---|
| `tyre_*` inputs | wheel | §8.4 | Tyre registration. |
| `track_speed_gain`, `track_slip_damping`, `track_friction_long`, `track_friction_lateral` | track carrier | 120, 8000, 0.85, 0.65 | Track motors (§11). |
| `rolling_radius_m` | children of a track carrier | none (not a roller) | Roller follower motor. |
| `position`, `value` | link moved by a `builtin:hitch` or `builtin:joint_position` joint | 0 | Target 0..1 of the range. |
| `range` | same | 1 | Target at `position = 1`, rad or m. |
| `velocity` | link moved by a `builtin:joint_velocity` joint | 0 | rad/s or m/s. |
| `rpm`, `engaged` | link moved by a `builtin:pto` joint | 540, 0 | Shaft speed (clamped 0–1200) and on/off. |
| `flow` (or `value`), `rate` | link moved by a `builtin:hydraulic_valve` joint | 0, 0.5 | Joint speed `flow × rate`, flow clamped −1..1. |
| `level` (or `value`) | link moved by a `builtin:brake` joint | 1 unattached, 0 attached | Brake level 0..1. |
| `angle_rad` | link moved by a `builtin:trailer_steer` joint | automatic | Steer angle. |
| `SectionControlState` | function | 1 | Section control on when > 0.5. |
| `SetpointWorkState`, `ActualWorkingWidth` | section | 0, 0 | Section requested on; its width in m. |
| `SetpointVolumePerAreaApplicationRate`, `ActualVolumeContent` | bin | 0, 0 | Rate setpoint (l/ha) and tank content (l). |

"The link moved by a joint" is the link whose parent joint is that joint.
A service controller reads that link's values on top of its own `/cmd` props,
so authoring `gearbox:value:range` on the link parameterises the controller.

Values written by the runtime: `slip`, `normal_force`, `slip_angle` and
`tyre_*` telemetry on wheel links; `track_*` telemetry on carrier links;
`ActualWorkState`, `TotalArea`, `EffectiveTotalDistance`,
`ActualVolumePerAreaApplicationRate`, `ActualVolumeContent` by the work
controllers.

## 16. Rejection rules

The machine is rejected (no agent, `machine_rejected` event with
`reason.<n>` props) when discovery records any error:

- link tree errors (§3.4);
- coupling errors (§14) and element type errors (§15.1);
- sensor errors: unknown kind, a `type` that does not apply, any out-of-range
  or unknown attribute value (§13);
- device errors, including the ownership rule (§12.1, §12.4);
- an invalid `gearbox:machine:tracks` definition (§11).

Warnings do not reject: a derived tree, a suspension joint of another type,
a sensor link that implies no kind.

A machine-category load is also rejected when its USD fails to load or a
saved tyre configuration does not fit (CONTROLLER_SPEC §1).

A rejected machine stays in the controller inventory (CONTROLLER_SPEC §11).

## 17. Name heuristics

The runtime still reads meaning from names in these places:

| Heuristic | Where | Effect |
|---|---|---|
| Side from the full joint path (case-insensitive): `left` or `_l` → left, else `right` or `_r` → right | diff drive `wheels` mode and visual spin; legacy wheel offsets | Forces the lateral sign of a wheel. `_r` matches `_rear`; a path such as `/World/krampe_ladewagen/...` contains `_l` and reads as left everywhere. |
| Axle from the full joint path: `front`/`fwd`, else `middle`/`mid`, else `rear`/`back` | legacy steering | Per-axle steer multipliers and differential degrees. |
| First steering-role path containing `left` / `right` | legacy Ackermann steering | Left and right steer joints. |
| Collider names containing `tire`, `tyre`, `wheel` | spawn alignment | Which colliders set the ground clearance. |
| Mesh names (§8.3) | tyre radius, rubber rendering | Which meshes are rubber. |
| Link name prefix | sensors | Sensor kind (§13.1). |
| Isaac joint names (§2.4) | Isaac compatibility | Roles. |

Spell out `left` and `right` in joint names and keep `_l`/`_r` out of prim
paths of machines that use diff drive.

## 18. Fixed conventions

- Chassis frame: forward −Y, left +X, up +Z, in a Z-up stage. The drive
  controllers, heading, roll, pitch and wheel geometry all assume it, whatever
  `base_link` or its axes say.
- Wheel joints rotate about the wheel's axle. The drive sign of a registered
  tyre comes from its joint axis (§8.3); an unregistered wheel is driven
  with sign +1.
- Sensor links: +X forward, +Y left, +Z up. Cameras look along +X with +Z
  up.
- SI units once `metersPerUnit = 1` and `kilogramsPerUnit = 1`.

## 19. Minimal machine

A cart with its chassis as `base_link`, two driven rear wheels, two free
front wheels, an IMU link and a boom on a motor device, lifted by a
`builtin:joint_position` controller through the tool joint role. It
satisfies every rule above.

```usda
#usda 1.0
(
    defaultPrim = "robot"
    upAxis = "Z"
    metersPerUnit = 1
    kilogramsPerUnit = 1
)

def Xform "robot" (
    prepend apiSchemas = ["GearboxMachineAPI", "GearboxControllerAPI:drive", "GearboxControllerAPI:boom"]
)
{
    token gearbox:machine:kind = "cart"
    rel gearbox:machine:body = </robot/chassis>
    rel gearbox:machine:role:poweredWheelJoints = [
        </robot/Joints/rear_left_spin>, </robot/Joints/rear_right_spin>
    ]
    rel gearbox:machine:role:passiveWheelJoints = [
        </robot/Joints/front_left_spin>, </robot/Joints/front_right_spin>
    ]
    rel gearbox:machine:role:toolJoints = [</robot/Joints/boom_lift>]

    token gearbox:controller:drive:type = "builtin:diff_drive_cmd_vel"
    token gearbox:controller:drive:driveMode = "wheels"
    token gearbox:controller:drive:commandInterface = "cmd_vel"
    token[] gearbox:controller:drive:stateInterfaces = ["pose", "velocity"]
    float gearbox:controller:drive:wheelRadius = 0.3
    float gearbox:controller:drive:maxWheelTorqueNm = 300
    float gearbox:controller:drive:maxPowerKw = 5

    token gearbox:controller:boom:type = "builtin:joint_position"

    def Xform "chassis" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]
    )
    {
        token gearbox:link:name = "base_link"
        token gearbox:link:role = "base"
        float physics:mass = 400
        float3 physics:diagonalInertia = (95, 42, 120)
        double3 xformOp:translate = (0, 0, 0.5)
        uniform token[] xformOpOrder = ["xformOp:translate"]

        def Cube "body_collider" (prepend apiSchemas = ["PhysicsCollisionAPI"])
        {
            double size = 1
            double3 xformOp:translate = (0, 0, 0.1)
            float3 xformOp:scale = (1, 1.6, 0.5)
            uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:scale"]
        }

        def Xform "imu_link" (prepend apiSchemas = ["GearboxLinkAPI"])
        {
            token gearbox:link:role = "sensor"
            token gearbox:sensor:kind = "imu"
            double3 xformOp:translate = (0, 0, 0.3)
            float3 xformOp:rotateXYZ = (0, 0, -90)
            uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]
        }
    }

    def Xform "wheel_front_left" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]
    )
    {
        token gearbox:link:role = "wheel"
        float physics:mass = 20
        float3 physics:diagonalInertia = (0.9, 0.52, 0.52)
        double3 xformOp:translate = (0.65, -0.6, 0.3)
        uniform token[] xformOpOrder = ["xformOp:translate"]

        def Cylinder "tyre_collider" (prepend apiSchemas = ["PhysicsCollisionAPI"])
        {
            uniform token axis = "X"
            double radius = 0.3
            double height = 0.2
        }
    }

    def Xform "wheel_front_right" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]
    )
    {
        token gearbox:link:role = "wheel"
        float physics:mass = 20
        float3 physics:diagonalInertia = (0.9, 0.52, 0.52)
        double3 xformOp:translate = (-0.65, -0.6, 0.3)
        uniform token[] xformOpOrder = ["xformOp:translate"]

        def Cylinder "tyre_collider" (prepend apiSchemas = ["PhysicsCollisionAPI"])
        {
            uniform token axis = "X"
            double radius = 0.3
            double height = 0.2
        }
    }

    def Xform "wheel_rear_left" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]
    )
    {
        token gearbox:link:role = "wheel"
        float physics:mass = 20
        float3 physics:diagonalInertia = (0.9, 0.52, 0.52)
        double3 xformOp:translate = (0.65, 0.6, 0.3)
        uniform token[] xformOpOrder = ["xformOp:translate"]

        def Cylinder "tyre_collider" (prepend apiSchemas = ["PhysicsCollisionAPI"])
        {
            uniform token axis = "X"
            double radius = 0.3
            double height = 0.2
        }
    }

    def Xform "wheel_rear_right" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]
    )
    {
        token gearbox:link:role = "wheel"
        float physics:mass = 20
        float3 physics:diagonalInertia = (0.9, 0.52, 0.52)
        double3 xformOp:translate = (-0.65, 0.6, 0.3)
        uniform token[] xformOpOrder = ["xformOp:translate"]

        def Cylinder "tyre_collider" (prepend apiSchemas = ["PhysicsCollisionAPI"])
        {
            uniform token axis = "X"
            double radius = 0.3
            double height = 0.2
        }
    }

    def Xform "boom" (
        prepend apiSchemas = ["PhysicsRigidBodyAPI", "PhysicsMassAPI", "GearboxLinkAPI"]
    )
    {
        float gearbox:value:range = 1.2
        float physics:mass = 30
        float3 physics:diagonalInertia = (2.05, 0.05, 2.05)
        double3 xformOp:translate = (0, 1.3, 0.6)
        uniform token[] xformOpOrder = ["xformOp:translate"]

        def Cube "boom_collider" (prepend apiSchemas = ["PhysicsCollisionAPI"])
        {
            double size = 1
            float3 xformOp:scale = (0.1, 0.9, 0.1)
            uniform token[] xformOpOrder = ["xformOp:scale"]
        }
    }

    def Scope "Joints"
    {
        def PhysicsRevoluteJoint "front_left_spin"
        {
            rel physics:body0 = </robot/chassis>
            rel physics:body1 = </robot/wheel_front_left>
            uniform token physics:axis = "X"
            point3f physics:localPos0 = (0.65, -0.6, -0.2)
            point3f physics:localPos1 = (0, 0, 0)
            quatf physics:localRot0 = (1, 0, 0, 0)
            quatf physics:localRot1 = (1, 0, 0, 0)
        }

        def PhysicsRevoluteJoint "front_right_spin"
        {
            rel physics:body0 = </robot/chassis>
            rel physics:body1 = </robot/wheel_front_right>
            uniform token physics:axis = "X"
            point3f physics:localPos0 = (-0.65, -0.6, -0.2)
            point3f physics:localPos1 = (0, 0, 0)
            quatf physics:localRot0 = (1, 0, 0, 0)
            quatf physics:localRot1 = (1, 0, 0, 0)
        }

        def PhysicsRevoluteJoint "rear_left_spin"
        {
            rel physics:body0 = </robot/chassis>
            rel physics:body1 = </robot/wheel_rear_left>
            uniform token physics:axis = "X"
            point3f physics:localPos0 = (0.65, 0.6, -0.2)
            point3f physics:localPos1 = (0, 0, 0)
            quatf physics:localRot0 = (1, 0, 0, 0)
            quatf physics:localRot1 = (1, 0, 0, 0)
        }

        def PhysicsRevoluteJoint "rear_right_spin"
        {
            rel physics:body0 = </robot/chassis>
            rel physics:body1 = </robot/wheel_rear_right>
            uniform token physics:axis = "X"
            point3f physics:localPos0 = (-0.65, 0.6, -0.2)
            point3f physics:localPos1 = (0, 0, 0)
            quatf physics:localRot0 = (1, 0, 0, 0)
            quatf physics:localRot1 = (1, 0, 0, 0)
        }

        def PhysicsRevoluteJoint "boom_lift"
        {
            rel physics:body0 = </robot/chassis>
            rel physics:body1 = </robot/boom>
            uniform token physics:axis = "X"
            point3f physics:localPos0 = (0, 0.8, 0.1)
            point3f physics:localPos1 = (0, -0.5, 0)
            quatf physics:localRot0 = (1, 0, 0, 0)
            quatf physics:localRot1 = (1, 0, 0, 0)
            float physics:lowerLimit = -10
            float physics:upperLimit = 80
            float gearbox:motor:maxForce = 2000
            float gearbox:motor:maxVelocity = 0.8
            float gearbox:motor:minPosition = -0.17
            float gearbox:motor:maxPosition = 1.39
        }
    }
}
```

What it produces:

- Links: `base_link` (the chassis), `imu_link` (static child of
  `base_link`), four `wheel_*` links and `boom`, all parented by their
  joints.
- Tyres on all four wheels (role `wheel`), the rear pair driven by the diff
  drive in `wheels` mode, the front pair free.
- One device, `boom_lift` (a motor). The `boom` controller has no `target`,
  so it takes the first tool joint and commands the motor to
  `position × range` = 0 rad until told otherwise.
- An IMU stream on `/machines/robot/sensors/imu_link`.

Drive it and move the boom:

```bash
gearbox machine move robot --forward 1 --turn 0.3 --for 3s
gearbox machine set-value boom position 0.5 --machine robot   # 0.6 rad
gearbox machine cmd boom position=0.25 --machine robot
```
