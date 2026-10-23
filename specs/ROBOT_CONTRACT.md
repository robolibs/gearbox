# Robot structure and control contract

One compiler turns a composed USD stage into the machine model that the
manifest, links and TF, controller and service binding, the joint command
path and the v2 API read. This file states what that model is, how authored
values become it, and how DOFs are commanded. It amends
[MACHINE_SPEC.md](MACHINE_SPEC.md), [CONTROLLER_SPEC.md](CONTROLLER_SPEC.md),
[TOOLS_SPEC.md](TOOLS_SPEC.md) and [STEERING.md](STEERING.md); where they
disagree, this file wins and the other file says so.

Code: `bin/gearbox/src/machine_model/` (compiler), `bin/gearbox/src/joint_control/`
(arbiter, scheduling, v2 endpoints), `bin/gearbox/src/physics/commit.rs` (the
actuator write of each DOF per tick).

```text
stage + selection -> scopes -> resolve -> normalize -> validate -> manifest
                                                       |
            physics attach <- accepted model -> links/TF, controllers, v2 API
                                                       |
   producers -> arbiter (owner, lease, batch) -> ControlTick -> commit -> backend step
```

## 1. Evidence levels

| Level | Meaning |
|---|---|
| **Isaac-verified** | Same fixture and schedule run in Gearbox and in Isaac Sim 6.2 through `oslo make test-isaac-contract`; compared within tolerances fixed in the schedule before the run. |
| **Native-verified** | A named Gearbox test exercises the behaviour, mutation-checked where noted. |
| **Implemented** | Code exists; no dedicated test. |
| **Not implemented** | Rejected with a named diagnostic, or listed in §15. |

## 2. Profiles and versions

| Profile | Selected by | Strict |
|---|---|---|
| `standard` | A UsdPhysics or Isaac articulation marker (`PhysicsArticulationRootAPI`, `IsaacRobotAPI`) outside any Gearbox machine | yes |
| `gearbox_v1_legacy` | A Gearbox machine root with no `gearbox:machine:schemaVersion`, or `1` | no |
| `gearbox_v2` | A Gearbox machine root with `int gearbox:machine:schemaVersion = 2` | yes |

- A Gearbox machine root is a prim with `GearboxMachineAPI`, `gearbox:machine:kind`,
  `gearbox:machine:idPolicy`, a `gearbox:machine:body` relationship or a
  `schemaVersion`. Compiler and runtime discovery use the same test.
- Any int other than 1 or 2 is `unknown_schema_version`; a non-int is
  `invalid_schema_version`. Both reject. This binary is the first that reads
  the version; older binaries ignore it (§12).
- Strict profiles turn the `strict_or_warn` checks into errors, give colliders
  of a body with an authored `physics:mass` no mass of their own, take the
  `PhysxRigidBodyAPI` fallbacks for unauthored body damping (§4) and the
  UsdPhysics fallback target for drives that author none (§5).
- No profile synthesizes a controller. A `standard` machine gets structure,
  DOFs, frames and command interfaces and no controller; a role a drive
  controller would need is reported as `suggested_role`. Under any Gearbox
  profile, empty roles of an authored controller are filled from the
  declared `gearbox:machine:role:*` relationships and reported as
  `legacy_inference`; a service with no target takes the first `toolJoints`
  DOF the same way. Joint names never fill a role: they only produce the
  `suggested_role` diagnostics of a `standard` machine and the legacy side
  and axle hints of MACHINE_SPEC §17.

Native-verified: `s02_articulation_marker_positions`, `s18_isaac_reporting_and_arrays`,
`bare_isaac_articulation_gets_structure_without_guessed_controllers`,
`s22_unknown_versions_and_invalid_numbers`.

## 3. Scope, identity and activation

- **Selection.** An asset load scans the subtree of the stage's `defaultPrim`
  (else `/`); a world load (`describe-machines --world`, the audit) scans `/`.
  Every Gearbox machine root is a scope. Every articulation marker outside
  one is a `standard` scope, even a lone body with no joints; bodies joined
  without any marker are not a machine. A root inside another is
  `nested_machine_root`; two machines with one id are `duplicate_machine_id`,
  an error under strict profiles and a warning under legacy, by each machine's own profile.
- **Identity.** A machine is `(machine_id, generation, load_instance)`. Each
  compiled load takes a new generation; commands name the generation they
  were built for (§8).
- **Compiler rejection.** A machine with any compiler error is rejected
  whole: its bodies become a fixed, non-simulating preview, none of its
  joints is built, and no controller, service, device, external process or
  DOF binding is registered. Invalid stage units (`invalid_units`: a
  `metersPerUnit` or `kilogramsPerUnit` not finite and positive, an `upAxis`
  other than Y or Z) reject every machine on the stage. Native-verified:
  `c19_a_rejected_machine_activates_nothing` (mutation-checked),
  `a_rejected_machine_body_is_a_fixed_preview`, `c24_a_failed_activation_binds_nothing`,
  `s13_units_are_equivalent_across_stages`.
- **Discovery rejection.** The runtime's legacy validations (link tree,
  devices and the ownership rule, sensors, tracks, couplings; MACHINE_SPEC
  §16) withhold the agent, controllers, services, devices and processes, but
  the machine's physics is built and simulates as passive bodies and joints.
- **Manifest.** `gearbox describe-machines FILE` (or `oslo make describe-machines --file=FILE`)
  prints the canonical JSON: sorted keys, 12 significant digits, quaternions
  `wxyz` with `w ≥ 0`, `null` for unbounded, every value with its `source`
  (`authored`, `schema_default`, `profile_default`, `legacy_derived`,
  `derived`). Goldens: `tests/fixtures/robot_contract/*.manifest.json`, rewritten
  only by `oslo make write-robot-goldens` and reviewed by diff.

## 4. Units, mass and body properties

L = metres per stage unit (`metersPerUnit`, schema fallback 0.01), M = kilograms
per mass unit (`kilogramsPerUnit`, fallback 1). Angles are authored in degrees.

| Quantity | SI value |
|---|---|
| length, joint frame, prismatic coordinate/limit | × L |
| mass | × M |
| diagonal inertia | × M·L² |
| density | × M / L³ |
| revolute coordinate/limit/target | degrees → rad |
| linear effort, break force, linear `maxForce` | × M·L |
| angular effort, break torque, angular `maxForce` | × M·L² |
| force drive, linear stiffness/damping | × M |
| force drive, angular stiffness/damping | × M·L² · 180/π |
| acceleration drive, linear stiffness/damping | × 1 |
| acceleration drive, angular stiffness/damping | × 180/π |
| body damping (`physxRigidBody:linearDamping`, `angularDamping`) | × 1 (1/s) |

- **Two readers, one rule.** The compiler converts the composed stage into
  the model (manifest, arbiter drive specs, mass and damping policy). Physics
  attachment reads usd_bevy's projected values and converts them with the
  same `StageUnits` functions, in f32. Both apply a body's uniform world
  scale to its joint frames and centre of mass. Native-verified:
  `s13_units_are_equivalent_across_stages`, `force_drive_gains_follow_their_dimensions`,
  `a_joint_frame_follows_its_body_scale`; Isaac-verified gains, caps and
  limits (`e02_arm` structure).
- **Acceleration drives.** Isaac 6.2 / PhysX reports an angular acceleration
  drive authored at stiffness 1, damping 0.05 as 57.2958 and 2.86479 per
  radian, and the measured response matches; Gearbox converts the same way.
  Isaac-verified (`e01_accel`), native `native_acceleration_drive_follows_the_degree_convention`.
- **Mass.** Under a strict profile a body with `physics:mass` takes that mass
  and its colliders add none; mass without `diagonalInertia` warns
  (`missing_inertia`). An inertia that is negative, not finite or breaks the
  triangle inequality is `invalid_inertia` under every profile. Legacy keeps
  v1 collider mass.
- **Body damping.** Authored values win under every profile (v1 ignored
  them). Unauthored: strict profiles take the `PhysxRigidBodyAPI` fallbacks,
  linear 0 and angular 0.05; legacy machines and bodies outside any machine
  keep v1's 0.1 / 0.5. No current asset authors body damping. Isaac-verified
  against the installed schema fallbacks (`e02_arm`) and by the velocity
  response of `e01_accel`.
- **Scale.** Nonuniform scale, shear and reflection of a body are errors
  under strict profiles (`nonuniform_scale`, `shear_transform`,
  `reflection_transform`); mass and inertia stay as authored under scale.
- Non-finite or out-of-range values are `invalid_numeric`, `invalid_mass`,
  `invalid_inertia`, `invalid_limit`, `invalid_drive`.

## 5. Joints

- **Typed endpoints.** Each side of a joint resolves to `world`, a `body` of
  the machine, or `invalid`. An empty relationship is the world. A target
  that is a non-body prim resolves to its nearest rigid-body ancestor
  (`endpoint_via_ancestor`) or, without one, a static world frame
  (`static_endpoint`). A target that does not exist (`invalid_endpoint`), two
  world sides (`both_endpoints_world`) and one body on both sides
  (`same_body_endpoints`) are errors under every profile, which rejects the
  machine. A body of another machine (`cross_machine_joint`) is an error
  under strict profiles; under legacy it warns and the joint is built. **An unresolved
  target never pins the other body to the world.** Native-verified:
  `s03_world_endpoint_on_either_side`, `s04_invalid_and_inactive_targets`,
  `an_unresolved_target_never_pins_the_other_body`.
- **World anchors.** A joint with one world side is built against a fixed
  anchor body at the authored world frame, so its DOFs keep moving.
  Native-verified: `a_world_revolute_joint_keeps_its_degree_of_freedom`.
- **Graph.** Joints are `tree`, `closure` (`physics:excludeFromArticulation`)
  or `world_anchor`. A closure is a physical constraint and never a parent in
  the link tree. Any cycle of non-excluded joints, including two of them
  between the same two bodies, is `unclassified_cycle` under every profile.
  Two joints between one pair stay two joints with two bindings when one of
  them is excluded. Disabled joints (`physics:jointEnabled = 0`) are reported
  (`disabled_joint`) and not built. An axis token other than X, Y or Z is
  `invalid_numeric`.
- **Limits.** Each side is independent: an absent or infinite side stays
  unbounded, in the manifest (`null`) and in the backend. D6 axes with no
  limit on a side are open on that side.
- **Drives** keep their authored type, gains and cap through both readers.
  An unknown `drive:<dof>:physics:type` is `invalid_drive` (error). A drive
  applied with a D6 token (`rotX`, `transX`, …) on a revolute or prismatic
  joint is `nonstandard_drive_token`, and a drive that names no free DOF of
  its joint is `invalid_drive`; both are errors under strict profiles and
  warnings under legacy.
- **Drive targets.** Under a strict profile a drive that authors neither
  `targetPosition` nor `targetVelocity` takes the UsdPhysics fallback, target
  position 0: a stiff drive springs to 0 and a damper holds the joint still,
  as in Isaac. Legacy machines keep such a drive idle (MACHINE_SPEC §6.4).
  Isaac-verified: `e02_lifecycle` (both return to 0 after a reset).

Supported joint types and their status are in §13.

## 6. DOFs and command interfaces

- DOFs are ordered by joint path, then token (a D6 joint's in the order
  trans X, Y, Z, rot X, Y, Z). A scalar joint's DOF is named
  after the joint (`isaac:NameOverride`, else the leaf name); a D6 DOF is
  `<joint>:<token>`. Spherical joints have no DOFs; a distance joint has one,
  which is never bound for commands. Each DOF has a unit (`rad` or `m`), its
  drive, limit, initial position and velocity, and a passive kind
  (`spring`, `damper`, `zero_gain`, `no_drive`).
- Frame, joint and DOF names are unique per machine: a repeat is
  `duplicate_name`, an error under strict profiles. With leaf-name defaults,
  two joints called `hinge` reject a strict machine (`s17_duplicates_and_nested_roots`).
- `uniform token[] gearbox:joint:<token>:commandInterfaces` lists what an
  external owner may command: `position`, `velocity`, `effort`. Unknown
  tokens, interfaces for a token the joint has no DOF for (any on a
  spherical joint, a locked D6 axis), interfaces on a distance joint, and
  interfaces the drive cannot realize (position needs stiffness > 0,
  velocity damping > 0) are `invalid_interface` errors (`s16_joint_type_capabilities`). With none authored, only internal owners
  command the DOF. A legacy machine derives a DOF's interfaces from the
  controllers bound to it (`legacy_derived`); strict profiles derive none.
- `token gearbox:joint:<token>:defaultMode` is `disabled` (default),
  `position`, `velocity` or `effort`, and a mode other than `disabled` must
  be among its command interfaces (authored, or derived under legacy). It is reported in the manifest only;
  the runtime's load-time actuator is the authored drive (§5, §8.3 Reset).
- An authored `state:<token>:physics:position` that disagrees with the pose
  by more than 1e-3 warns (`initial_state_mismatch`).

## 7. Frames, sites and the control basis

- **Frames.** Every body is a frame (frame index = body index). Sites are
  frames rigidly attached to a body: `GearboxLinkAPI` prims that are not
  bodies, couplings, Isaac sites, sensor mounts and elements. A site's offset
  is the composed rigid transform from its owner body; sensors are mount
  frames only, with no sensor-model change.
- **Reporting tree.** The link tree is re-rooted at the base frame
  (`gearbox:machine:baseFrame`, v2; else the control body) and walked over
  tree joints; `reversed` marks a joint whose body0 is the child. A body the
  tree does not reach warns (`disconnected_body`) under every profile.
  `conflicting_parent` (an error under strict profiles) marks a jointed body
  that also authors `gearbox:link:parent`, a frame parent outside the
  machine and a cycle of frame parents. Two tree joints into one body whose
  parents are already connected are an `unclassified_cycle`.
- **Control frame.** The body-local drive basis is +X forward, +Y left,
  +Z up; only its rotation is used. A v2 machine needs `gearbox:machine:body`
  (`missing_control_body`) and may declare `rel gearbox:machine:controlFrame`,
  a frame riding that body; without it the base frame is the control frame
  (`profile_default`), so an unmigrated v2 asset drives along its chassis +X.
  Legacy machines derive the control frame as the chassis frame turned −90°
  about Z, v1's −Y forward. A `standard` machine has none, as it has no drive
  controller. Steering, body-local commands, heading and controller state
  use it. Native-verified: `the_declared_control_frame_sets_the_drive_direction`
  (a v1 and a v2 cart drive along their own forward axes, headings 0 and π/2),
  `s10_up_axis_and_control_frame`.
- **TF v2** publishes every edge of the reporting tree, sites included, as a
  parent-relative transform in the right-handed Z-up SI world; its props
  carry `convention` (`right_handed_z_up`), `root`, and the root's world pose
  as `root_translation` and `root_rotation_wxyz`. Native-verified:
  `tf_v2_sites_follow_their_moving_bodies`, `real_weedunit_tips_follow_their_tool_bodies`;
  Isaac-verified initial site pose and the tool site through motion (`e02_arm`).

## 8. Joint commands

### 8.1 Identity and owners

A command names a **DOF key** `(machine, generation, joint path, token)`.

| Owner class | Who |
|---|---|
| `controller` | A drive controller of the machine (Ackermann, diff drive, tracked) |
| `service` | Hitch, PTO, valves, joint position/velocity, brake, trailer steer, coupling |
| `device` | An authored motor device with no other owner |
| `direct` | An external client holding a server-issued lease |
| `test_driver` | The trusted deterministic replay driver |

- Owners are assigned when a machine binds, from the model's controller and
  coupling bindings. Two internal owners of one DOF are `duplicate_owner` and
  reject the machine. A brake and a trailer steer own the joints the runtime
  drives (wheelJoints, else the brake role, else every wheel; steerJoints,
  else the steering role). A brake skips a DOF a drive controller owns, and
  its writes there are refused (`s23_service_targets_bind_the_joints_the_runtime_drives`);
  a trailer steer on a drive-owned DOF is `duplicate_owner`. A drive
  controller owns its own wheel and steer joints together with the machine's powered, passive and steering roles, as the runtime drives both.
- A service's target is the exact joint it names; a joint that was not built
  drives nothing.
- **Legacy query path.** Drive controllers find their wheel and steer joints
  by the bodies they join (`owned_joints_between`), restricted to the joints
  their owner holds. A joint no machine has bound (outside any machine, or
  before its machine binds) is leased to its first writer under
  `unbound:<machine>` with every interface and legacy caps.

### 8.2 Setpoints

| Setpoint | Effect |
|---|---|
| `Position { q, qdot? }` | Drive target position with the DOF's authored gains, clamped to its limits |
| `Velocity { qdot }` | Drive target velocity; a joint rate, not a ground speed |
| `Effort { force }` | Generalized force on the coordinate, N or N·m; drive gains off |
| `Hold` | Position at the coordinate measured once when the hold begins |
| `Free` | No actuator: the motor and the authored drive are off; limits stay |
| `Servo { … }` | An internal owner's own gains and model; rejected from external owners |

- With no authored drive and no motor device, `Position` and `Hold` have zero
  gain and do nothing; an external `Hold` needs the `position` interface.
- Effort is real generalized force (`JointTargetMode::Effort` in Molla), not a
  saturated velocity motor. Native-verified: `c01_true_effort_is_a_generalized_force`,
  `c23_commanded_effort_is_not_measured_effort`; Isaac-verified effort-phase
  accelerations (`e02_arm`, within the 10 % tolerance; observed 0.3 %).

### 8.3 Arbiter

- One owner per DOF. A batch is validated whole and accepted whole; a bad
  command rejects it with one of: `UnknownDof`, `StaleGeneration`, `NotOwner`,
  `StaleEpoch`, `OutOfOrder` (sequences grow per DOF per owner; a new lease
  is a new owner, so numbering restarts), `InterfaceNotAllowed`,
  `ServoFromExternal`, `NonFinite`, `UnboundedLiveCommand`, `DuplicateDof`,
  `Revoked`. `acquire` fails with `Conflict` when another direct lease holds a
  DOF and `InterfaceNotAllowed` when the DOF allows none.
- **Validity.** `Latched` (internal owners, re-evaluated every due tick; v1
  `actuate` device commands too), `Ticks(n)` (replay), `WallMs(ms)` (live
  external commands; default 500 ms, `command_validity_ms` in the manifest,
  at most 10 000 ms). Wall-time validity runs on during a pause or STEP.
  A live velocity or effort command without a bound is rejected.
- **Expiry and loss of authority.** A lapsed command, a revoked lease or a
  quiesced owner takes its DOF to the safe setpoint before the next physics
  step: after velocity, velocity 0; after effort or nothing, `Free`; after an
  internal velocity servo, the same servo at rate 0; an internal position
  servo stays on its target; otherwise `Hold`.
- **Leases.** `acquire` suspends the DOFs' internal owners and holds the DOFs
  where they are; a second `acquire` on the same session first releases the
  previous lease without resuming. `release` with `resume` hands the DOFs
  back to their suspended owner; without it, and on revocation (a new
  session or generation), the DOFs take their safe setpoint and stay
  ownerless until acquired again and released with `resume`, or reset.
- **Reset.** A control reset (v2 `control_v2` op `RESET`, or the arbiter's
  `reset_machine`) clears every command, hold and lease of the machine,
  gives each DOF back to its default owner and keeps the generation; no body
  moves. A DOF left without a command returns on the next tick to its
  load-time actuator: the authored drive with its targets (§5), a motor
  device's rest command (its authored target, or holding where it stands),
  or nothing. A scene reset or reload is different: the machine unbinds and
  returns with a new generation. Native-verified: `c18_reset_touches_one_robot`,
  `c18_a_reset_dof_returns_to_its_load_time_actuator` (mutation-checked),
  `c21_v2_leases_follow_the_session_and_a_reset_clears_them`; Isaac-verified:
  `e02_lifecycle`.
- **Caps.** The applied cap is `min(authored maxForce, requested)`; the
  hardware and safety terms of the arbiter are hooks this runtime leaves
  unset. Legacy decoding may use a requested cap above the authored one. An
  absent cap is no bound.
- **Commit.** Inside `PhysicsWorld::step`, every DOF with a command, or whose
  command just ended, gets exactly one actuator write per tick (`writes`
  counts them). Native-verified: `physics_command_inventory_passes`,
  `c06_one_commit_write_per_dof_per_tick`. Other paths act on joints:
  - a motor device realizes its committed command: a drive motor inside the
    commit, a backend (Molla) device during the backend step after it;
  - an authored drive acts as built on a DOF nobody commands;
  - joint friction and soil grip act on joints without an authored drive,
    a device or a command;
  - device settings (speed, acceleration, PID, `brake=` damping) configure a
    device; they are not targets.
  Every machine scan goes through the compiler.

## 9. Scheduling

- Drive controllers, services and tracked drives run in the `ControlTick`
  schedule, once per physics step, before the commit. Work controllers and
  the copter run per rendered frame. Native-verified:
  `schedule_inventory_passes` (C12, C13, C15; C12 mutation-checked).
- `gearbox:controller:<id>:updateRateHz` is functional: a simulation-time
  phase accumulator makes a controller due on the right physics ticks, also
  for non-integer ratios. Its `dt` is the time since its last due tick; on
  its first due tick, one physics tick. Unauthored is the physics rate.
  Non-positive is an error; above the physics rate is clamped (an error under
  strict profiles), `invalid_controller_rate`.
- **Pause and step.** While paused no physics tick and no control tick runs.
  `STEP n` (the `PhysicsStepBudget`) counts physics ticks, at most 12 per
  rendered frame, and pauses after the last. Catch-up is capped at 12 ticks
  per frame and 0.25 s of frame time. The clock state's `step` counter counts
  rendered frames while active; a TOGGLE does not clear a pending budget.

## 10. v2 API

New machine topics, beside the unchanged v1 ones (v1 payload bytes are
pinned by `v1_payloads_keep_their_bytes`):

| Topic | Kind | Message |
|---|---|---|
| `description_v2` | req/res | `MachineDescriptionV2 { generation, manifest }`: the canonical manifest |
| `control_v2` | req/res | `ControlRequestV2 { session, generation, epoch, op, resume, names }` → `ControlResponseV2 { epoch, generation, tick, code, props }`; op `ACQUIRE` 0, `RELEASE` 1, `RESET` 2 |
| `joint_command_v2` | req/res | `JointCommandV2 { session, epoch, sequence, generation, max_effort, validity_ms, modes, values, feedforward, names }` → `JointCommandAckV2 { sequence, generation, tick, status, code, message }` |
| `joint_state_v2` | pub | `JointStateV2 { generation, tick, sim_time_s, valid, position, velocity, measured_effort, commanded_effort, target, names }` |
| `tf_v2` | pub | `TfV2 { generation, tick, sim_time_s, translation, rotation, parents, children, props }` |

- Modes: position 0, velocity 1, effort 2, hold 3, free 4. Values are SI.
- `control_v2` and `joint_command_v2` need the agent's current claimed
  session and the current generation; `description_v2` needs neither. A
  command batch is one arbiter batch (§8.3).
- For a machine with Isaac reporting metadata, joint state follows the
  `isaac:physics:robotJoints` order and leaves out the DOFs of unlisted joints.
- Acks are `accepted` (1) or `rejected` (2); the `received` (0) and
  `applied` (3) values are reserved and not sent. Codes: `USAGE` for
  malformed batches and an overlong validity, `NOT_FOUND` for unknown DOF
  names, `UNSUPPORTED` for an unknown mode, `REFUSED` for session, lease,
  generation and every arbiter rejection, whose reason is in `message`.
- State and TF are published once per rendered frame, after its physics
  steps, so intermediate ticks are not streamed; a joint velocity is NaN on
  a frame with no new tick.
- Client calls: `description_v2`, `acquire_v2`, `release_v2`, `reset_v2`,
  `joint_command_v2`, `joint_state_v2`, `tf_v2` in `gearbox_api::client`.
  Native-verified: `lifecycle_inventory_passes` (C19–C24, the server in C21),
  `v2_messages_round_trip`.

## 11. Couplings

- A coupling between two v2 machines aligns the full coupling frames
  (position and rotation); any other pair keeps the v1 position-only
  alignment. See [TOOLS_SPEC.md](TOOLS_SPEC.md).
- A coupler's PTO and valve joints that no controller of the slave claims
  belong to its `coupling` service owner. When the master detaches, that
  owner is quiesced: its DOFs take their safe setpoint (a PTO's velocity
  servo stops on its own damping) and keep their owner
  (`c20_detached_joints_stop_and_keep_their_owner`). Joints the slave's own
  services own stay with those services.
- Link offsets are composed rigid transforms between world matrices.

## 12. Compatibility and rollback

| Behaviour | v1 runtime | This runtime, legacy asset | This runtime, v2 or standard asset |
|---|---|---|---|
| `schemaVersion` | ignored | absent or 1 | 2; anything else rejected |
| Controller for a bare articulation | Isaac Ackermann guessed from names | — | none (standard) |
| Joint with one world side | body fixed to the world | anchored, keeps its DOF | anchored, keeps its DOF |
| Unresolvable joint target | pinned to world | not built; the machine rejected | not built; the machine rejected |
| Joint cycles, bad inertia, unknown drive type, two owners of a DOF, non-positive controller rate | not checked (a cycle panicked the adapter) | rejected | rejected |
| One-sided limits | ignored (both sides needed) | applied | applied |
| Distance joints | skipped | built | built |
| Break and cone limits | ignored | applied | applied |
| `maxForce` | not scaled | × M·L or M·L² | × M·L or M·L² |
| Linear force-drive gains | ÷ `metersPerUnit` | × M | × M |
| Angular acceleration-drive gains | as authored | × 180/π | × 180/π |
| Effort command | saturated velocity motor | generalized force | generalized force |
| Drive with no target | idle | idle | target 0 |
| Body damping | 0.1 / 0.5 | authored or 0.1 / 0.5 | authored or 0 / 0.05 |
| Collider mass on an explicit-mass body | added | added | none |
| Control frame | −Y forward | −Y forward (derived) | declared, else base frame +X |
| Coupling alignment | position only | position only | full frame between two v2 machines |
| Command scheduling | per rendered frame | per physics tick | per physics tick |

- This runtime reads `schemaVersion` absent, 1 and 2. The designated rollback
  runtime is this commit built with the `schema-v1-only` feature: it reads
  legacy assets and rejects every v2 one (`unknown_schema_version`). A
  rollback restores it together with the v1 package revisions, machines
  submodule `676be7d`. `oslo make test-rollback-guard` builds it into
  `target/rollback` and checks that it rejects all 24 production v2 packages
  and loads all 24 v1 revisions (restored from Git LFS). Binaries before this
  one ignore the version and must be paired with the v1 revisions too.
- **Migration.** Every production package under `machines/usd` is v2
  (2026-10-23, after the user's review): `oslo make audit-machine-assets
  --asset-root=machines/usd --require=v2` reports 24 of 24 `gearbox_v2`.
- How the packages were migrated:
  - `oslo make stage-v2-migration` (five representative packages; `--all`
    for every package) copies packages under
    `target/robot-contract/migration-*/packages/` and writes v2 overlays in
    `v2/`. Each overlay adds `schemaVersion = 2`, a declared control frame
    equal to the legacy basis, and the command interfaces v1 derived and the
    drive can realize (in `layers/`).
  - Reviewed per-asset metadata (`scripts/robot_contract/migrations/*.overs`):
    - the weed unit: unique joint names on its side units;
    - the Kubota and Fendt: `toolJoints` without the passive linkage joints;
    - the Ropa: `toolJoints` without its 161 welded joints, and the stale
      `PhysicsDriveAPI:angular` deleted from its 125 welded rollers, whose
      controllers its own layer already disables.
  - `oslo make promote-v2-migration [--apply]` bakes each overlay into its
    package (`scripts/robot_contract/promote_v2.py`). A new root layer
    `<name>_v2.usda` and `<name>_interfaces.usda` come first; every original
    entry follows unchanged, stored and 64-byte aligned. The promoted copy
    must compile to the same model as the staged overlay, pass the
    `--require v2` audit and the real scenarios; only then does `--apply`
    replace `machines/usd`.
- Result:
  - Same bodies, joints, DOFs, frames and controllers, and the same control
    axes, as v1.
  - Every body's damping takes the strict fallbacks (§4).
  - Six packages drop a `velocity` interface on passive wheels or idlers with
    no authored damping.
  - The Kubota, Knoche and weed unit scenarios of `test-real-machines`
    (through discovery and the runtime) pass on the v2 packages and on the
    v1 revisions. The other 21 packages are compared by compiled manifest.

## 13. Capability matrix

| Feature | Status | Evidence |
|---|---|---|
| `joint.revolute`, `joint.prismatic` | exact, Isaac-verified | `s01_floating_fixed_and_branched`, `s16_joint_type_capabilities`, `c01_revolute_and_prismatic_commands_move_their_joints`, `e02_arm` |
| `joint.fixed` | exact | `s16_joint_type_capabilities`, `a_fixed_joint_keeps_the_rotation_of_its_authored_frames` |
| `joint.d6` | exact | `s16_joint_type_capabilities`, `a_d6_hinge_swings_down_to_its_limit` |
| `joint.spherical` | approximate: cone limits are soft | `ball_distance_and_break_limits_carry_their_authored_sides`, `a_spherical_cone_limit_stops_the_swing` |
| `joint.distance` | exact | `a_distance_joint_holds_its_maximum` |
| `joint.break_limits` | approximate: breakage follows the backend's effort estimate | `a_break_force_releases_an_overloaded_joint` |
| `joint.world_anchor` | exact, Isaac-verified | `s03_world_endpoint_on_either_side`, `a_world_revolute_joint_keeps_its_degree_of_freedom`, `e02_arm` |
| `joint.closure` | approximate: soft loop constraint | `s06_loop_closure_is_physical_and_never_a_parent` |
| `drive.force` | exact, Isaac-verified | `force_drive_gains_follow_their_dimensions`, `s13_units_are_equivalent_across_stages`, `e02_arm` |
| `drive.acceleration` | exact, Isaac-verified | `acceleration_drive_gains_ignore_mass_and_length`, `native_acceleration_drive_follows_the_degree_convention`, `e01_accel` |
| `limit.one_sided` | exact | `s21_one_sided_and_unbounded_limits` |
| `command.position`, `command.velocity`, `command.effort` | exact, Isaac-verified | `c01_revolute_and_prismatic_commands_move_their_joints`, `c01_true_effort_is_a_generalized_force`, `e02_arm` |
| `command.multi_dof_joint` | not supported: spherical joints have no DOF, distance DOFs are not bound, interfaces there are `invalid_interface` | `s16_joint_type_capabilities` |
| `body.uniform_scale` | approximate: mass and inertia as authored | `s12_physical_scale`, `a_joint_frame_follows_its_body_scale` |
| `composition.instance` | not supported | `s15_variants_payloads_and_instances` |
| `composition.unloaded_payload` | not supported | `s15_variants_payloads_and_instances` |
| hold, free, reset, pause | exact, Isaac-verified | `command_inventory_passes`, `c18_a_reset_dof_returns_to_its_load_time_actuator`, `c15_pause_single_step_and_catch_up`, `e02_arm`, `e02_lifecycle` |
| declared drive mapping (Ackermann, diff drive) | exact for the steady kinematic mapping, Isaac-verified | `e03_native_mapping_cases`, `e03_mapping` |
| real-asset sites (`ball_tip`, `tool_tip`) on moving tool bodies | exact, native-verified on v1 and staged v2 | `real_weedunit_tips_follow_their_tool_bodies` (ignored by default; run by `test-real-machines`) |

The rows from `joint.revolute` to `composition.unloaded_payload` are
`machine_model::capabilities::TABLE`, which each manifest reports per use
(`command.multi_dof_joint` is listed but never reported); every evidence name
there must be a real test (`capability_evidence_names_real_tests`). The last
three rows are harness results (§14).

## 14. Conformance harness

| Recipe | Runs |
|---|---|
| `oslo make test-machine-contract` | S01–S23 structural fixtures, goldens, capability evidence |
| `oslo make test-joint-control` | everything under `joint_control::` (C06–C18, C12, C13, C15, the lifecycle cases and the native conformance tests) and the physics command cases with the commit boundary |
| `oslo make test-control-lifecycle` | C19–C24, C07, C16–C18, the reset commit case, the rejected preview, v1 golden bytes, v2 round trips |
| `oslo make test-control-conformance` | native arm schedule and lifecycle, drive units, E03 mapping, control frame, TF sites |
| `oslo make test-isaac-contract` | the above, then each schedule in headless Isaac Sim, compared |
| `oslo make test-real-machines` | Kubota tractor lifting the Knoche harrow, Kubota driving under pressure changes, weed unit ball_tip and tool_tip following their tool bodies; `--tractor`, `--implement`, `--weedunit` pick other packages |
| `oslo make stage-v2-migration [--all]` | staged v2 overlays, before/after reports, `--require v2` audit |
| `oslo make test-rollback-guard` | the `schema-v1-only` rollback build rejects the 24 production v2 packages and loads their v1 revisions |
| `oslo make promote-v2-migration [--apply]` | stage every package, bake the overlays into package copies, check them, and with `--apply` replace `machines/usd` |
| `oslo make verify-robot-contract` | machine contract, joint control, control lifecycle, hitch and services, the Isaac contract with native conformance, the asset audit, real machines on the production v2 packages and on the v1 revisions, the rollback guard, every production package v2; prints which passed |

- **Isaac runs** go through the Nix launcher (`isaacsim --no-window --no-ros-env
  --exec scripts/robot_contract/isaac_probe.py`), one Kit child per schedule
  under `timeout --kill-after=15 300` and the probe's own 240 s watchdog. The
  launcher's bubblewrap dies with that child; no other Kit process is
  touched. A missing launcher or `VERSION` fails the gate as not run. Each
  child's exit status is kept beside its log (`kit.log.exit`). A child that
  exits before starting the probe is tried once more, its log kept as
  `kit-startup-failure.log`; a second such failure fails the gate. A probe
  that ran is never retried, so a mismatch always fails.
- The probe exits 0 completed, 2 startup failure, 3 absent feature,
  4 rejected asset, 5 runtime error, 6 timeout, and writes `preflight.json`
  (Isaac, Kit, USD, enabled physics and core extensions), `overlay.json`
  (every spec the Gearbox overlay authors; a physics property there rejects
  the run), `structure.json` (USD graph in SI, tensor-API DOF order, gains,
  caps, limits, link masses and inertias, schema fallbacks), `trace.jsonl` and `status.json`;
  a mapping schedule (E03) writes `preflight.json`, `mapping.json` and
  `status.json` only.
- `scripts/robot_contract/compare.py` checks structure, targets, motion and
  lifecycle as separate classes; `evidence.py` writes `run.json` with source
  and submodule revisions, executable and input hashes and per-schedule
  results. Evidence lands in `target/robot-contract/<kind>-<time>/`.
- Gates use the schedules' tolerances (positions 0.02 rad or m, rates and
  velocities 5 %, effort accelerations 10 %, hold drift 0.01, reset return
  0.02, E03 steer 1e-3 rad and wheel rates 0.5 %). Observed on 2026-10-23:
  - **E01** (`e01_arm.usda` + `e01_arm_gearbox.usda`): 66 structure checks equal.
  - **E02** (`schedules/e02_arm.json`, 600 ticks at 120 Hz): mode and target
    equal on every tick; settled positions within 2e-4; velocity-phase rate
    within 0.02 %; shoulder hold drift 5.1 mrad in both.
  - **Lifecycle** (`schedules/e02_lifecycle.json`): 60 rendered frames with
    physics paused advance no tick and move nothing in either (natively
    through the simulator's own step planner and stepper, in Isaac as Kit
    app updates), and both resume at their speed; after a control reset both
    return the shoulder and slide to their load-time target 0 through their
    authored drives, 0.344 vs 0.347 rad 30 ticks in.
  - **E03** (`schedules/e03_mapping.json`): Gearbox's Ackermann and diff-drive
    controllers, with the chassis on the commanded no-slip twist so their
    trims rest at zero, emit the same steer angles and wheel rates as Isaac's
    own `AckermannController` and `DifferentialController` (Isaac input
    `steering = atan(L·ω/v)`). Both take the commanded speed at the unsteered
    axle centre.

## 15. Not covered

- Contact, tyre and track dynamics, sensors and rendering are not compared;
  the E02 schedules use a contact-free arm in zero gravity.
- E03 covers the steady kinematic mapping only: command shaping, steering
  sweep, speed and yaw trims and power limits are Gearbox behaviour with no
  Isaac counterpart.
- `isaacsim.ros2.control` is not tested; plan 001 makes it optional.
- The audit, staging, promotion and rollback checks are compiler checks with a world
  selection; discovery and the runtime are exercised on the Kubota, Knoche
  and weed unit only.
