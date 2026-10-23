# Gearbox specifications

Contracts between machine assets, the runtime and bus clients. Each file
describes what the code does; anything not implemented is marked as such.
Dated notes, measurements and asset logs live in `../docs/`.

| File | Covers |
|---|---|
| [ROBOT_CONTRACT.md](ROBOT_CONTRACT.md) | The shared machine model and joint command contract: profiles and `schemaVersion`, scope and activation, units, typed joint endpoints, DOFs and command interfaces, frames and the control frame, owners and the arbiter, physics-tick scheduling, the v2 API, compatibility and rollback, the capability matrix and the Isaac conformance harness. Takes precedence over the files below where they meet. |
| [MACHINE_SPEC.md](MACHINE_SPEC.md) | The USD authoring contract for a machine: stage metadata, the machine prim and id, the link tree and its validation, bodies, mass, colliders, joints and drives, joint roles, wheels and tyre registration, steering joints, suspension, tracks, actuator devices and the motor ownership rule, sensor links, couplings, elements and named values, rejection rules, name heuristics, fixed conventions, and a minimal machine in USDA. |
| [CONTROLLER_SPEC.md](CONTROLLER_SPEC.md) | Controllers and the runtime API: loading and discovery, controller instances, the drive controllers (Ackermann, diff drive, tracked, copter), service controllers (hitch, PTO, valves, joint position and velocity, brake, trailer steer), work controllers, `external:process`, master/slave requests, the agent topics (sessions, `/cmd_vel`, `/cmd`, `/state`, sensors, `/actuate`), unused attributes and known gaps. |
| [TOOLS_SPEC.md](TOOLS_SPEC.md) | Couplings and attachments: coupling types and hitch joints, static and runtime attach and detach, the composite link tree, command routing between master and slaves, what masters and slaves author. |
| [DRIVETRAIN.md](DRIVETRAIN.md) | Wheel torque limits of the Ackermann drive: grip, `maxWheelTorqueNm`, `maxPowerKw`, unloaded wheels, parking torque, traction control, passive wheels, telemetry. |
| [STEERING.md](STEERING.md) | The geometric Ackermann steering solver: geometry, requirements, pivot, rolling vector, curvature limiting, steer servos. |
| [TRACKED_DRIVE.md](TRACKED_DRIVE.md) | `builtin:tracked_cmd_vel` and the `gearbox:machine:tracks` JSON: track motors, carrier-link values, asset binding, rollers, FEM visuals, telemetry. |
| [TYRE_PRESSURE.md](TYRE_PRESSURE.md) | Pressure tyres: the tyre asset contract, registration, `gearbox:value:tyre_*`, pressure controls, telemetry, axle identity, model limits, rubber rendering, saved configurations. |
| [FEM_CONTACTS.md](FEM_CONTACTS.md) | The `fem_contacts` JSON inside a track definition for the experimental FEM track: shapes, fields, validation. |

## Versions, profiles and evidence

- A machine asset is read under one profile: `standard` (UsdPhysics or Isaac
  articulation, no Gearbox metadata), `gearbox_v1_legacy` (`GearboxMachineAPI`,
  `gearbox:machine:schemaVersion` absent or 1) or `gearbox_v2`
  (`schemaVersion = 2`). An unknown version is rejected. Profiles are in
  [ROBOT_CONTRACT.md](ROBOT_CONTRACT.md) §2, compatibility and rollback in §12.
- Wire messages are versioned by topic: v1 payloads keep their bytes; the
  v2 joint command, control, state, TF and description topics are added beside
  them (ROBOT_CONTRACT §10).
- A statement here is one of: **Isaac-verified** (compared with Isaac Sim by
  `oslo make test-isaac-contract`), **native-verified** (a named test),
  **implemented** (code, no dedicated test) or **not implemented** (a named
  diagnostic or a listed gap). The capability matrix (ROBOT_CONTRACT §13)
  names the tests. A proposal that is not built belongs in `../plans/`, not
  in these files.
