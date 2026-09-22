# Torque-driven continuous tracks

`builtin:tracked_cmd_vel` accepts forward m/s and left-positive yaw rad/s.
It commands Molla track motors; it does not set chassis pose or velocity.
The controller shares authored `maxPowerKw` equally between two motors and
uses `maxWheelTorqueNm` as each motor's torque ceiling. Bounded yaw feedback
compensates for skid-steering slip while ground contacts exist. Feedback
clears at zero yaw or loss of support. Belt target speeds are limited to 3 m/s.

## Asset binding

`gearbox:machine:tracks` is a JSON string containing two track definitions:

- `link`: carrier link name for telemetry.
- `carrier`, `sprocket`: rigid-body prim paths, joined by a revolute joint.
- `contacts`: belt-support collider prim paths belonging to the carrier.
- `side`: +1 left, -1 right.
- `radius`: drive sprocket pitch radius in metres.
- `axle`, `forward`: orthogonal unit vectors in the carrier frame; this visual
  adapter requires axle +X and a path in the carrier's YZ plane.
- `path`: closed belt centreline points in carrier-local metres, in the YZ plane.
- `treads`: ordered visual prim paths, directly parented to the carrier.

The tread prototype has X across the belt, Y along travel and Z inward.
Treads start evenly spaced around the path. Their position follows measured
Molla belt travel, including reverse motion and slip, rather than commands.
Passive wheel links under each carrier use `rolling_radius_m` and revolute
joints. Their low-torque follower motors track measured belt speed.

Each carrier should use multiple support patches along the footprint to
distribute normal load. Belt colliders supply normal support; Molla owns
their longitudinal/lateral friction forces. Pneumatic tyre registration and
tyre collider preparation are skipped for tracked machines.

## GPU FEM visual preparation

The optional `fem_visuals` object in each track definition declares `version: 1`,
`deformable` (prim roots replaced by the FEM surface), and `material` (one of those
roots supplying the rubber StandardMaterial). Include the continuous carcass and
every animated tread, not the carrier, wheel links or contact colliders. Paths
are rebased with the owning machine and resolved within its scene instance.

`FemMachineVisuals::prepare` validates these targets without changing physics
ownership or displayed materials. It prepares two soft surfaces from FEM node
indices and immutable mesh-to-body bindings for the remaining rigid meshes,
preserving source material handles. Missing, shared, overlapping or ambiguous
targets fail rather than leaving part of the belt frozen. The corresponding CPU
asset gate is `GEARBOX_TRACK_ASSET=/absolute/machine.usdz oslo make test-fem-visuals`.
This preparation API does not yet activate FEM in the simulator menu or synchronize
CPU TF consumers with GPU poses.

## Telemetry and validation

Carrier link values publish `track_travel_m`, `track_speed_mps`,
`track_torque_nm`, `track_force_n`, `track_normal_load_n`, `track_contacts`.
Controller wheel encoders report measured sprocket angle and angular speed.
Disabled/unloaded controllers remove their track registrations.

Run `GEARBOX_TRACK_ASSET=/absolute/machine.usdz oslo make test-tracked-machine`
for the imported-asset CPU regression. `oslo make test` runs generic tests.

This is a reduced continuous-belt model, not individual rubber/shoe contact,
elastic belt tension, derailment or soil excavation. The current Molla track
solver is CPU-only. CEOL geometry, inertia and drivetrain values are estimated,
not manufacturer-calibrated. Passive follower motors approximate belt coupling;
they are not an elastic belt transmission constraint.
