# PLAN — link trees and attachments

Implement `specs/CONTROLLER_SPEC.md` §7 (every machine publishes its link
tree) and `specs/TOOLS_SPEC.md` (masters tow or carry slaves through typed
couplings, with one command session for the composite). The link tree comes
first: attachments re-parent one tree under another, and the coupling frame
is a link.

Companion documents: the two specs above (what a USD must author),
`PLAN_COM.md` §10 (the agentio bus as built: one agent per machine,
claim / cmd_vel / release, `/machines/<ns>/…` topics), `PLAN_CLI.md` §10
(`machine links` and `machine tools` exit 5 today and are reserved for this).

## 0. Decisions

| Decision | Why |
|---|---|
| **Links are discovered at load, in `discover_machines_from_usd`, next to controllers.** | Discovery already reopens the stage and walks the machine prim; links and couplings are two more schemas on the same walk. No second parser. |
| **Unmarked assets get a derived tree with a warning, marked assets are validated strictly.** | The spec says every rigid body MUST carry `GearboxLinkAPI`. Every asset in the repo today has none. Rejecting them all on day one blocks every demo. Derivation: rigid bodies are links named after their prim, joints give parents, the machine body is `base_link`. The moment one `GearboxLinkAPI` appears in a machine, §7.3 applies in full. |
| **The tree goes over the bus as a que/ans stream of `LinkRecord`s, not as props on `MachineInfo`.** | A tractor has ~10 links, a composite with two trailers ~30. A flat `Map` of `link.<n>.*` keys works but is unreadable; que/ans is what `scene list` already uses for lists. |
| **Dynamic link poses are a separate pub/sub topic, `/machines/<ns>/tf`, opt-in per machine, off by default.** | The tree is static structure; wheel and steer angles change every frame. Scripts that need them (a viewer, a recorder) subscribe; nobody else pays for 30 poses at 60 Hz. |
| **Attachment topics live on the master's agent under `/machines/<master>/tools/…`, and the slave agent stays alive.** | `TOOLS_SPEC.md` §5.1 was written for one zenoh session with path routing. With one agent per machine the slave already owns its topics; closing them means refusing, not deleting. The slave keeps publishing its own `/state` (a viewer wants it), refuses `cmd_vel` and `claim` with `REFUSED "attached to <master>"`, and answers `/info` with `attached_to`. |
| **Slave controllers are commanded through the master's `/cmd` with `tool = <slave_ns>` in the props, not through nested `/tools/<slave>/<controller>/cmd` topics.** | Exact-match topics on the machine agent would need one server per slave controller, created and destroyed on every attach. One `/cmd` with a routing prop keeps the directory stable. Nested slaves: `tool = "trailer_1/trailer_2"`. |
| **Coupling joints are rapier `ImpulseJoint`s built by coupling type, with limits, never motors.** | Same joint family `usd_rapier` already builds from USD joints. The slave's own wheels stay whatever they were (multibody or impulse); only the coupling is added at runtime. |
| **Collision between coupled links is suppressed with collision groups, restored on detach.** | rapier has no per-pair filter without hooks; groups on the two link colliders (and only those) give the §2.1 rule exactly. |
| **Engine force scales with composite mass.** | §3.3. Without it the tractor stalls with a loaded trailer. Composite mass is the sum of every attached machine's rigid bodies; recomputed on attach and detach. |
| **Static attachments in a world layer (`GearboxAttachmentAPI`) reuse the runtime path.** | Discovery collects them; after both machines are ready the loader issues the same attach the CLI would, with `teleport = true`. One code path. |
| **Service controllers (`builtin:hitch`, `builtin:pto`, `builtin:hydraulic_valve`) and slave requests (TIM) are the last phase.** | A drawbar trailer that follows a tractor is the demo that proves the plumbing. PTO and hydraulics need new controller types and a slave that consumes them. |

## 1. Link tree

### 1.1 Data model (sim side, `bin/gearbox/src/controller.rs`)

```rust
pub struct LinkSpec {
    pub name: String,              // gearbox:link:name or prim name
    pub prim_path: String,
    pub role: LinkRole,            // Base, Link, Wheel, Steer, Sensor, Tool
    pub parent: Option<String>,    // link name; None only for base_link
    pub joint_prim: Option<String>,// the UsdPhysics joint that connects it, if any
    pub static_offset: Option<Transform>, // for rigid-body-less links
    pub body_prim: Option<String>, // the PhysicsRigidBodyAPI prim, if any
    pub coupling: Option<CouplingSpec>,   // §2
}

pub struct LinkTree {
    pub links: Vec<LinkSpec>,      // base_link first, then breadth-first
    pub derived: bool,             // true when no GearboxLinkAPI was authored
    pub warnings: Vec<String>,
}
```

`MachineInstanceSpec` gains `pub links: LinkTree`.

Discovery order inside the machine prim:

1. Collect every prim with `PhysicsRigidBodyAPI` and every prim with
   `GearboxLinkAPI` (marked links may be rigid bodies or plain `Xform`s).
2. Collect every `UsdPhysics*Joint` with `physics:body0` / `physics:body1`
   inside the machine.
3. If no prim carries `GearboxLinkAPI`: derived mode. Every rigid body is a
   link named after its prim; `gearbox:machine:body` is `base_link`; joints
   give parents (`body1` child of `body0`). Warn once per machine:
   `machine <id>: link tree derived, author GearboxLinkAPI to make it explicit`.
4. Otherwise strict mode: §7.3 rules, each failure a reason string. A
   machine with any failure is **not** made a machine agent; the reasons go
   to the log, to the Machine controllers pane, and out on the scene events
   stream as `machine_rejected` with `reason` props.
5. `gearbox:link:parent` overrides; rigid-body-less links take the nearest
   ancestor link and the composed local xform as `static_offset`.
6. `base_link` axes: when `base_link` is an `Xform` above the body, the body's
   parent is `base_link` with the inverse xform (§7.1 second form). The
   existing chassis basis (forward −Y, left +X, up +Z in Bevy) stays the
   runtime convention; `base_link` is REP-103 in the asset frame, and the
   static rotation between them is recorded on the body link, so `tf`
   consumers get REP-103 without changing the controller math.

### 1.2 Wire types (`crates/gearbox-api/src/wire/machine.rs`)

```rust
#[datapod::datapod(name = "gearbox.link_record.v1")]
pub struct LinkRecord {
    pub x: f64, pub y: f64, pub z: f64,          // static offset to parent (REP-103, metres)
    pub qw: f64, pub qx: f64, pub qy: f64, pub qz: f64,
    pub index: u32, pub parent_index: u32,       // u32::MAX for base_link
    #[dp(bytes)] pub props: Vec<u8>,             // name, parent, role, prim, joint, body, coupling.*
}

#[datapod::datapod(name = "gearbox.link_pose.v1")]
pub struct LinkPose {                            // one per link per tf sample
    pub x: f64, pub y: f64, pub z: f64,
    pub qw: f64, pub qx: f64, pub qy: f64, pub qz: f64,
    pub index: u32, pub stamp_ms: u32,
    #[dp(bytes)] pub props: Vec<u8>,             // name
}
```

Topics on the machine agent (`crates/gearbox-api/src/topics.rs`):

| Topic | Mode | Request → Response |
|---|---|---|
| `/machines/<ns>/links` | que/ans | `Ping` → `LinkRecord` × N, base_link first |
| `/machines/<ns>/tf` | pub/sub | `LinkPose`, one sample per link per step, only while a subscriber asked for it through `/cmd` `tf = on` |

`MachineInfo` props gain `link_count`, `links_derived` (`true`/`false`) and
`base_link` (prim path). `MachineConfig` gains `links: Vec<LinkDesc>`;
`MachineAgent::new` hosts the que/ans server and answers from it.

### 1.3 CLI (`crates/gearbox-cli/src/cli/machine.rs`)

| Leaf | Does |
|---|---|
| `machine links [NS]` | tree view: name, role, parent, offset; `--json` gives the records; `--flat` gives a table |
| `machine tf [NS] [--rate HZ] [--link NAME]` | turns `tf` on, tails poses, turns it off on exit |

`machine info` prints `links: 9 (derived)` or `links: 9`.

### 1.4 Python (`scripts/gearbox_client.py`)

`Machine.links() -> list[dict]` and `Machine.tf(on: bool)` plus a
`LinkRecord` / `LinkPose` mirror. `oxbo_follow_points.py` gets an optional
`--tf` flag that records wheel angles, as the first consumer.

### 1.5 Assets

`tractor.usd`, `oxbo.usd`, `hunter.usd`, `husky.usd`, `tracer.usd` get
`GearboxLinkAPI` on every rigid body, `base_link` on the chassis (second
form where the chassis basis is not REP-103, which is all of them today),
`role = "wheel"` on wheels and `role = "steer"` on knuckles. Until then they
load in derived mode; the pane shows the warning.

### 1.6 Validation tests

`bin/gearbox/src/controller.rs` tests, one USDA snippet each, mirroring
§7.3: unmarked body in strict mode, two bases, duplicate name, joint to a
foreign prim, kinematic loop, disconnected link, parent cycle, wrong role
under a controller relationship. Plus `tractor.usd` discovers the expected
nine links with `derived = false` once 1.5 lands.

## 2. Attachments

### 2.1 Data model

```rust
pub struct CouplingSpec {
    pub name: String,           // gearbox:coupling:name or prim name
    pub side: CouplingSide,     // Hitch | Coupler
    pub kind: CouplingType,     // §2.1 of TOOLS_SPEC, 12 values
    pub iso_category: Option<u8>,
    pub capacity_kg: Option<f32>,
    pub services: Vec<String>,  // pto, hydraulic, electrical, isobus
    pub lift_joint: Option<String>,
    pub excludes: Vec<String>,
}

pub struct Attachment {
    pub master: String,         // namespace
    pub slave: String,          // namespace
    pub hitch: String,          // coupling name on the master
    pub coupler: String,        // coupling name on the slave
    pub kind: CouplingType,
    pub joint: ImpulseJointHandle,
    pub filtered: Vec<(ColliderHandle, InteractionGroups)>, // to restore on detach
    pub controlled: bool,       // slave has controllers
}

#[derive(Resource, Default)]
pub struct Attachments { pub list: Vec<Attachment> }
```

A `CouplingSpec` hangs off the `LinkSpec` of the coupling prim
(`role = "tool"`). Discovery reads `GearboxCouplingAPI` on the same walk as
links. A coupling on a prim that is not a link is a §8 refusal.

### 2.2 Joint per coupling type

Built with `GenericJointBuilder` from the hitch frame to the coupler frame,
both expressed in their body's local frame from the composed prim xforms:

| type | locked | free with limits |
|---|---|---|
| `drawbar`, `clevis`, `piton`, `pivot_wagon`, `fifth_wheel`, `three_point_semi_mounted` | all translations | yaw free; pitch ±20° (±15° piton, fifth_wheel); roll ±10° (drawbar), 0 otherwise |
| `hitch_hook`, `cuna`, `ball` | all translations | all rotations, cone limit 25° / 20° / 30° |
| `three_point_mounted`, `chassis_mounted`, `loader_carriage` | everything | — |

Pose: the coupler frame is placed on the hitch frame rotated 180° about Z
(§2.2). With `teleport`, every rigid body of the slave is moved by the same
rigid transform that takes the coupler frame there, velocities zeroed, then
the joint is inserted. Without `teleport`, the current offset must be within
0.25 m and 30°, and the joint is inserted where the bodies are; rapier pulls
them together over the next steps.

Collision: the hitch link's colliders and the coupler link's colliders get an
`InteractionGroups` that excludes each other; the previous groups are kept in
the `Attachment` and restored on detach.

Composite mass: `Attachments` recomputes per master the sum of rigid-body
masses of every machine below it; `RAYCAST_ENGINE_FORCE_*` scale with it
instead of the chassis mass (`controller.rs`, the raycast vehicle force law).

### 2.3 Wire types and topics

```rust
#[datapod::datapod(name = "gearbox.attach_request.v1")]
pub struct AttachRequest {
    pub session: u64,
    pub teleport: u32, pub _pad: u32,
    #[dp(bytes)] pub props: Vec<u8>,   // slave, hitch, coupler
}

#[datapod::datapod(name = "gearbox.detach_request.v1")]
pub struct DetachRequest {
    pub session: u64,
    #[dp(bytes)] pub props: Vec<u8>,   // slave
}

#[datapod::datapod(name = "gearbox.attachment.v1")]
pub struct AttachmentRecord {
    pub controlled: u32, pub depth: u32,
    #[dp(bytes)] pub props: Vec<u8>,   // master, slave, hitch, coupler, type, denied
}
```

On the master's agent:

| Topic | Mode | Request → Response |
|---|---|---|
| `/machines/<ns>/tools/attach` | req/res | `AttachRequest` → `Status` (REFUSED: session, type mismatch, occupied, loop, tolerance; NOT_FOUND: slave or coupling) |
| `/machines/<ns>/tools/detach` | req/res | `DetachRequest` → `Status` |
| `/machines/<ns>/tools` | que/ans | `Ping` → `AttachmentRecord` × N, depth-first |

Every machine agent hosts the three; a machine with no hitch answers
`UNSUPPORTED` on attach and an empty list. Scene events gain
`event_kind::ATTACHED = 5` and `DETACHED = 6` with `master`, `slave`,
`hitch`, `coupler` props, so a watcher sees composites change without polling.

While attached, on the slave's agent:

- `/claim` and `/cmd_vel` answer `REFUSED` with `attached_to = <master>`.
- `/info` props gain `attached_to`, `via_hitch`, `via_coupler`.
- `/state` and `/odom` keep publishing; `/links` answers the slave's own
  tree with `parent` of `base_link` set to `<master>/<hitch link>`.

On the master:

- `/cmd` with `tool = <slave_ns>` (or `a/b` for nesting) routes the
  `ControllerCommand` to that slave's controller. Without `tool` it is the
  master's own controller, as today.
- `/state` props gain `tools = trailer_1,trailer_2` and
  `tool.<ns>.controller.<name>.*` for each slave controller state.
- `/links` answers the composite: the master's tree, then each slave's tree
  with link names prefixed `<slave_ns>/`, `base_link` re-parented under the
  hitch link, as §4 of `TOOLS_SPEC.md` draws it.

The bus code is in `MachineAgent::poll` (`crates/gearbox-api/src/machine.rs`)
for the request handling and in `bin/gearbox/src/controller.rs` for the
physics; the agent collects requests into `pending_attach: Vec<AttachRequest>`
the way `commands` collects `ControllerCommand`s, and a system applies them
and answers.

### 2.4 Static attachments in a world layer

Discovery collects `GearboxAttachmentAPI` prims (`hitch` and `coupler`
relationships) into `MachineInstanceSpec`-independent `StaticAttachment`s on
the loaded asset. Once both target machines report `machine_ready`, the
loader queues an attach with `teleport = true` through the same code path.
Failures are logged and shown in the pane; the machines stay loaded and
separate.

### 2.5 CLI

| Leaf | Does |
|---|---|
| `machine tools list [NS]` | attachments of a master, depth-first, with type and controlled flag |
| `machine tools attach [NS] SLAVE [--hitch NAME] [--coupler NAME] [--teleport] [--take]` | claims the master (or uses the held session), attaches, prints the record. Hitch and coupler default to the only free pair of matching type; ambiguity is exit 2 listing the candidates |
| `machine tools detach [NS] SLAVE [--take]` | detach |
| `machine tools couplings [NS]` | the machine's hitches and couplers, free or occupied |
| `machine cmd --tool SLAVE …` | routes to a slave controller |
| `spawn from` manifests | `[[attach]] master = "t1" slave = "trailer_1" teleport = true` entries after the spawns |

`machine links` on a master prints the composite.

### 2.6 Python

`Machine.attach(slave, hitch=None, coupler=None, teleport=False)`,
`Machine.detach(slave)`, `Machine.tools()`, and `Gearbox.events()` decoding
the two new event kinds. `scripts/tractor_trailer.py`: spawn a tractor and a
trailer 3 m behind it, attach with teleport, drive a figure eight with the
follow-points logic, detach, drive away.

### 2.7 Assets

- `tractor.usd`: `rear_drawbar` hitch (`drawbar`, at the rear of the chassis,
  0.45 m up) and `rear_three_point` (`three_point_mounted`, `lift` unset until
  phase 5). Both are `Xform` links with `role = "tool"` under the chassis.
- New `trailer.usd`: single-axle flatbed. Chassis body, two passive wheel
  joints, `drawbar_eye` coupler (`drawbar`) 2.2 m ahead of the axle,
  `GearboxMachineAPI` with `kind = "trailer"`, no controllers. ~300 lines of
  USDA plus a reused wheel mesh from the tractor.
- New `world/yard.usda`: flat field referencing a tractor and a trailer with
  a static `GearboxAttachmentAPI` between them, for phase 4.

### 2.8 Master to slave inputs and slave requests

Phase 5, the `TOOLS_SPEC.md` §5.2 and §5.3 content:

- `ControllerInputs` (the per-step input block builtin controllers read)
  gains `master: Option<MasterState>` with ground speed, heading, roll,
  pitch, hitch position, PTO rpm. Filled for every controller of an attached
  slave each step from the master's last state.
- `gearbox:controller:<n>:requests` on slave controllers and
  `gearbox:machine:grants` on masters, discovered and reported in the
  attachment record as `denied`. Granted `speed` and `steering` requests
  write the master's `cmd_vel` when the session sent none this step.
- `builtin:hitch`, `builtin:pto`, `builtin:hydraulic_valve` as new controller
  types in `controller.rs`, driving one joint each; the PTO coupling adds a
  fixed-ratio motor joint between master and slave PTO stubs on attach.
- `builtin:trailer_steer`, `builtin:brake`, `builtin:joint_position` for
  controlled trailers.

## 3. Phases

Each phase ends with something runnable and a test that proves it.

### Phase 1 — link discovery and `machine links`

- `LinkSpec`, `LinkTree`, discovery in derived and strict mode, §7.3
  validation with reasons to log, pane and a `machine_rejected` event.
- `LinkRecord`, `/machines/<ns>/links`, `MachineConfig::links`, `machine
  links`, `Machine.links()` in Python.
- `tractor.usd` marked per §7.1; the other assets stay derived.

Accept: `gearbox machine links tractor` prints nine links under `base_link`
with wheels as `wheel` and knuckles as `steer`; `gearbox machine links oxbo`
prints a derived tree and `machine info oxbo` says `derived`; each §7.3 rule
has a test that rejects a snippet with the expected reason; the fake host
answers `/links` for its fake machines with a two-link tree so the CLI test
covers the leaf.

### Phase 2 — `tf`

- `LinkPose`, `/machines/<ns>/tf` gated by `/cmd tf = on|off`, published
  from the physics writeback at the controller rate.
- `machine tf` and `Machine.tf()`.

Accept: `gearbox machine tf tractor --link wheel_front_left` shows the wheel
turning while `machine move` drives; publishing stops when the CLI exits.

### Phase 3 — couplings and runtime attach

- `CouplingSpec` discovery, `Attachment`, joint per type, collision groups,
  composite mass in the force law.
- `AttachRequest`, `DetachRequest`, `AttachmentRecord`, the three topics on
  every machine agent, slave refusals, master `/cmd` routing by `tool`,
  composite `/links`, `ATTACHED` / `DETACHED` events.
- `trailer.usd`, tractor hitches, `machine tools list/attach/detach/couplings`,
  Python attach/detach, `tractor_trailer.py`.

Accept: `gearbox spawn machine tractor.usd --ns t1 && gearbox spawn machine
trailer.usd --ns tr1 --at 0 0 -4 && gearbox machine tools attach t1 tr1
--teleport && gearbox machine move t1 --forward 2 --turn 0.4 --for 10s`
drives a turning tractor with the trailer following on the drawbar and no
interpenetration; `gearbox machine move tr1` exits 4 with `attached to t1`;
`machine links t1` shows `tr1/base_link` under `rear_drawbar`; `detach`
leaves the trailer standing where it is and `move tr1` is refused with exit 5
(no cmd_vel controller), not 4. The round-trip test does all of this against
the fake host with a two-machine fake composite.

### Phase 4 — static attachments and manifests

- `GearboxAttachmentAPI` in world layers, `world/yard.usda`, `[[attach]]`
  entries in `spawn from` manifests, `scene reset` replays them.

Accept: `gearbox spawn world world/yard.usda` yields two machines and one
attachment in `machine tools list` without a CLI attach.

### Phase 5 — services and requests

- §2.8 in full: master inputs on slave controllers, requests and grants,
  `builtin:hitch` / `pto` / `hydraulic_valve`, `builtin:trailer_steer` /
  `brake` / `joint_position`, PTO coupling motor.
- A `sprayer.usd` slave with a `builtin:joint_position` boom and a
  `requests = ["speed"]` controller as the example.

Accept: `gearbox machine cmd t1 --tool sprayer boom position=1` folds the
boom; the sprayer's controller sees the tractor speed in its inputs; a slave
requesting `hitch:rear_lift` on a master without that grant shows
`denied = hitch:rear_lift` in `machine tools list`.

### Phase 6 — later

- DDOP export (`TOOLS_SPEC.md` §7.3 to §7.5): element tree and process data
  over `/cmd`, `gearbox machine ddop NS` writing the XML.
- Link tree on the host's `/gearbox/scene/list` for props, so bales and
  markers appear in one `tf`-style view.
- A `gearbox tf` viewer extension that draws frames in the sim.

## 4. Risks

- **Joint stability with mass ratios.** A 2.7 t tractor towing a 6 t trailer
  on impulse joints can jitter or stretch. Mitigation: `GenericJoint` with
  the solver iterations already raised for vehicles; if it still stretches,
  attach through a `MultibodyJoint` when both ends are impulse-jointed and
  fall back to impulse otherwise. Decide in phase 3 with the trailer demo.
- **Teleport into the terrain.** Moving the slave onto the hitch on a hill
  can bury its wheels. Mitigation: after the rigid move, raycast each wheel
  down against the terrain mesh sampler from `world.rs` and lift the whole
  slave by the worst penetration before inserting the joint.
- **Derived trees hide asset errors.** Derived mode exists to keep old
  assets loading; it must warn every load, and the pane must show it, or
  nobody marks the assets. Phase 1 marks `tractor.usd` so one asset shows
  the strict path end to end.
- **Slave refusals surprise scripts.** A script that owned a trailer before
  it was attached keeps calling `cmd_vel` and gets `REFUSED`. The Python
  client's re-claim on refusal (`gearbox_client.py`) must not loop on
  `attached_to`; it raises with the master's namespace instead.
- **Directory churn.** Attach and detach change what the master's `/state`
  and `/links` return but never which topics exist, by design. If phase 5
  needs per-slave topics after all, they are created once per attach and
  the directory revision bumps; agentio handles that, but the CLI's
  `api topics` table would need to read the directory instead of its static
  list.
- **Composite mass and the speed servo.** Scaling engine force with 3× the
  mass also scales braking impulses; check that an unloaded tractor after a
  detach does not keep the high gains (recompute on detach, not only on
  attach).

## 5. Implementation notes (2026-09-12)

Phases 1 to 4 are implemented. Where the code differs from the plan above,
the code wins and the difference is recorded here.

- **Link discovery lives in `bin/gearbox/src/links.rs`**, called from
  `discover_machines_from_usd`. Derived mode is exactly §1.1; strict mode
  records §7.3 failures in `LinkTree::errors`, the Machine controllers pane
  shows them in red, `sync_machine_agents` skips the machine and publishes
  one `machine_rejected` event with `reason.<n>` props.
- **Static offsets are in the asset's Z-up frame**, composed from
  `xformOpOrder` (translate, orient, rotateXYZ, rotateX/Y/Z, transform).
  No REP-103 re-basing of the chassis is done: the assets author their
  chassis forward as −Y and the offsets say so honestly.
- **`/machines/<ns>/links`** answers `gearbox.link_record.v1` (offset and
  indices in the header, name / parent / role / prim / joint / body /
  `coupling = side|type|name` as props). **`/machines/<ns>/tf`** streams
  `gearbox.link_pose.v1` world poses (sim frame, Y up) while `/cmd` with
  `tf = on` was received; the toggle needs no session.
- **Every valid machine gets an agent**, not only those with a `cmd_vel`
  controller. A machine without one refuses `cmd_vel` with `UNSUPPORTED`
  from the CLI's side (no controller) and publishes `/state` from its body
  pose, so a trailer reports where it is.
- **Attachments are `bin/gearbox/src/attach.rs`.** Requests arrive on the
  master agent (`/tools/attach`, `/tools/detach`, que/ans `/tools`) and are
  parked with their reply token until the sim's `serve_attachments` system
  acts and answers. The joint is a rapier `GenericJoint` per §2.2 with
  `contacts_enabled(false)`, so the two coupled bodies never collide; no
  collision groups were needed. Teleport moves every slave body by the rigid
  transform that puts the coupler frame on the hitch frame rotated 180°
  about the up axis; without teleport the 0.25 m / 30° tolerance applies.
- **Composite mass** is a `TowedMass` resource keyed by machine id, summed
  over everything below a master and added to the chassis mass in the
  raycast vehicle force law.
- **Coupling frame convention as authored.** `tractor.usd` has
  `chassis/rear_hitch` (drawbar, identity rotation, 1.9 m behind the origin);
  `trailer.usd` has `chassis/drawbar_eye` (drawbar, rotated 180° about Z,
  2.25 m ahead of its origin) and its own `rear_hitch` for tandem use. With
  the 180° rule the trailer faces the same way as the tractor.
- **Slave refusals** carry `attached_to` and a message; the Python client
  raises rather than re-claiming when it sees them.
- **Master `/cmd` with `tool = <slave>`** is re-queued onto the slave
  agent's command list by `serve_attachments`; no controller consumes
  `ControllerCommand`s yet, so this is plumbing for phase 5.
- **Static attachments** (`GearboxAttachmentAPI`, `world/yard.usda`) are
  collected when a machine-category USD is spawned and applied with
  teleport once both machines have agents (up to 1200 frames). A
  multi-machine asset spawned with `--ns yard` names its machines
  `yard_<id>`; `spawn machine` waits for any `yard_*` namespace.
- **Manifests** take `[[attach]]` tables (`master`, `slave`, optional
  `hitch`, `coupler`, `teleport = true` by default) after the spawns, and
  `scene reset` replays them.
- **Fake host** attaches without physics (records the tool, refuses the
  slave's commands, re-parents links) so the CLI tests cover
  `machine tools attach / list / detach` and the slave's refusal.
- **Not verified in a window:** the physical joint behaviour (trailer
  following, stability at the tractor / trailer mass ratio). Phase 3's
  acceptance run needs the simulator window; `scripts/tractor_trailer.py`
  is the script for it.
- **Phase 5 is in `bin/gearbox/src/services.rs`.** The six service
  controller types drive one rapier joint each through motors
  (`set_motor_position` / `set_motor_velocity`), found at runtime by the
  pair of bodies a USD joint connects, in the impulse set or the multibody
  set. Commands arrive as `/cmd` props keyed by `controller = <instance>`
  and are merged per controller in `ServiceCommands`, so a motor keeps its
  last setting. `MasterInputs` gives every attached slave its master's
  ground speed, heading, roll, pitch, hitch positions and PTO state each
  step (`feed_master_inputs`). `gearbox:controller:<n>:requests` and
  `gearbox:machine:grants` are discovered; the ungranted set is reported as
  `denied` on the attachment record and the `attached` event. A slave sends
  a request as `/cmd` with `request = speed|steering` and `value`; granted
  ones land in `TimRequests` and drive the master's `cmd_vel` while its own
  session is quiet. `builtin:trailer_steer` follows the master's heading
  when no angle is commanded. Assets: `tractor.usd` gained a
  `rear_three_point` hitch and `grants = [speed, steering]`; `sprayer.usd`
  is a three-point slave with a `boom` `builtin:joint_position` controller
  that requests `speed` and `hitch:rear_lift` (the second is denied by the
  tractor, on purpose).
- **PTO and valve binding** (§6.2) needs two coupler-side attributes the
  spec left open: `rel gearbox:coupling:ptoJoint` names the slave joint the
  master's PTO spins, `rel gearbox:coupling:valveJoints` the slave joints
  bound to the master's hydraulic valves in valve order. Bound joints with
  no controller of their own follow the master's PTO speed and valve flows;
  a `builtin:joint_velocity` on the PTO joint follows it too unless a
  velocity was commanded directly. There is no fixed-ratio rapier joint
  between the two shafts, the slave joint's motor is set to the master's
  speed each step.
- **Requests `hitch:<instance>`, `pto:<instance>`, `aux_valve:<n>`** write
  the master's matching service controller (`ServiceCommands`), so a slave
  can raise the hitch, engage the PTO or open a valve when granted.
- **The master's `/state`** carries `tool.<slave>.controller.<instance>.<key>`
  for every attached slave's service controller and `pd.<element>.<Ddi>`
  for its own process data.
- **Phase 6 is in too.** `bin/gearbox/src/elements.rs` discovers the ISO
  11783-10 element tree (`GearboxElementAPI`, `gearbox:element:*`,
  `gearbox:pd:*`), derives the device, connector (from couplers, with DDI
  157) and navigation elements, checks numbers and structure per §8, and
  warns on DDI names it does not know (the full ISO 11783-11 dictionary is
  not shipped; a small table maps the ones the controllers use).
  `/machines/<ns>/elements` answers `gearbox.element_record.v1`;
  `gearbox machine elements`, `gearbox machine ddop [--out FILE]` (ISOXML
  `DVC`/`DET`/`DPD`/`DPT`) and `gearbox machine pd ELEMENT DDI VALUE` use
  it. `builtin:section_control` and `builtin:rate_control` run over the
  element tree with ground speed from the master (`ProcessData` resource;
  values ride `/state` as `pd.*`). `gearbox scene tree` prints one tf-style
  view of machines and props (props and markers carry `link = base_link`
  in `scene list`). Link frames are drawn as gizmos in the window for any
  machine whose tf stream is on, so `gearbox machine tf` shows them live.
  DDOP import (§7.5, the other direction) is not implemented.
- **DDOP offsets** follow the spec's REP-103 formula `(x, −y, −z)`; the
  repo's assets author their chassis forward as −Y, so their exported
  offsets are rotated until they are re-based. `drpOffset` is not read.
