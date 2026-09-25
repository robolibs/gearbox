# Torque-driven continuous tracks

`builtin:tracked_cmd_vel` (`bin/gearbox/src/controller/tracked.rs`) takes
forward m/s and left-positive yaw rad/s and commands two Molla track motors on
the sprocket joints. It never sets chassis pose or velocity.

## Controller

| Attribute | Type | Default | Effect |
|---|---|---|---|
| `gearbox:controller:<n>:trackWidth` | float, m | **required** | Distance between belt centrelines. Missing or ≤ 0: the controller reports an error and drives nothing. |
| `gearbox:controller:<n>:maxWheelTorqueNm` | float | 300 | Torque ceiling of each track motor (N·m). |
| `gearbox:controller:<n>:maxPowerKw` | float | 10 | Drive power, split equally: each motor gets `maxPowerKw × 500` W. |
| `gearbox:controller:<n>:body` | rel | machine body | Chassis. |

Each frame:

1. A non-finite command becomes zero. Forward speed is clamped to ±3 m/s and
   yaw to ±2 rad/s.
2. Yaw feedback runs while both sprockets report ground contacts and the yaw
   command is at least 0.001 rad/s: `error = commanded yaw − measured yaw`,
   `trim += 3 × error × dt` (clamped ±6), and the yaw command becomes
   `yaw + 2 × error + trim`. Otherwise the trim resets to 0.
3. Belt speeds are `v − ω·trackWidth/2` (left, `side = +1`) and
   `v + ω·trackWidth/2` (right), scaled together so neither exceeds 3 m/s.
4. Rollers (below) follow the measured belt speed; telemetry and wheel
   encoders are updated.

When the controller is disabled, removed, or fails (missing track definitions,
`trackWidth`, chassis, carrier, sprocket, drive joint, carrier link or belt
colliders), its track registrations are removed and the failure is logged
once.

## Carrier link values

Read from the authored `gearbox:value:*` of the carrier link (`link` in the
track definition) when the track is first registered:

| Value | Default | Effect |
|---|---|---|
| `track_speed_gain` | 120 | Motor response, N·m per rad/s. |
| `track_slip_damping` | 8000 | Belt grip, N·s/m. |
| `track_friction_long` | 0.85 | Longitudinal belt friction. |
| `track_friction_lateral` | 0.65 | Lateral belt friction. |

Scale `track_speed_gain` and `track_slip_damping` with machine mass.

## Asset binding

`gearbox:machine:tracks` on the machine prim is a JSON array of exactly two
track definitions, one per side. Any error rejects the machine.

| Field | Required | Contract |
|---|---|---|
| `link` | yes | Carrier link name; telemetry and material values live on it. |
| `carrier`, `sprocket` | yes | Rigid-body prim paths joined by a revolute joint. |
| `contacts` | yes, non-empty | Belt-support collider prims belonging to the carrier. |
| `side` | yes | `+1` left, `−1` right; the two sides multiply to −1. |
| `radius` | yes | Drive sprocket pitch radius, m, > 0. |
| `axle` | yes | Exactly `[1, 0, 0]` in the carrier frame. |
| `forward` | yes | Unit vector orthogonal to `axle`. |
| `path` | yes | Closed belt centreline, ≥ 3 carrier-local points in the YZ plane (x = 0), perimeter ≥ 0.01 m. |
| `treads` | yes, non-empty | Ordered visual prims, directly parented to the carrier. |
| `fem`, `fem_contacts`, `fem_visuals` | no | Experimental FEM data ([FEM_CONTACTS.md](FEM_CONTACTS.md), below). |

Every path must lie under the machine prim and is rebased with it. Sprocket,
contact and tread prims must each belong to one track only.

A tracked machine skips tyre preparation and pressure-tyre registration. Its
belt colliders supply normal support and Molla owns their longitudinal and
lateral friction forces. Each carrier should use several support patches along
the footprint to spread normal load.

The tread prototype has X across the belt, Y along travel and Z inward.
Treads start evenly spaced around the path; their position follows measured
Molla belt travel, including reverse motion and slip, not commands.

## Rollers

Links whose parent is the carrier link, that have a rigid body other than the
sprocket, a joint to the carrier and an authored
`gearbox:value:rolling_radius_m` > 0, get a follower motor on that joint's
AngX each frame: a Force-model velocity motor at
`belt speed / rolling_radius_m`, damping 1, max torque 2 N·m. Author them as
revolute joints. Links without `rolling_radius_m` are left alone.

## GPU FEM visual preparation

The optional `fem_visuals` object in each track definition declares
`version: 1`, `deformable` (prim roots replaced by the FEM surface) and
`material` (one of those roots, supplying the rubber StandardMaterial).
Include the continuous carcass and every animated tread, not the carrier,
wheel links or contact colliders. Paths are rebased with the owning machine
and resolved within its scene instance.

`FemMachineVisuals::prepare` validates these targets without changing physics
ownership or displayed materials. It prepares two soft surfaces from FEM node
indices and immutable mesh-to-body bindings for the remaining rigid meshes,
keeping source material handles. Missing, shared, overlapping or ambiguous
targets fail rather than leaving part of the belt frozen. The CPU asset gate is
`GEARBOX_TRACK_ASSET=/absolute/machine.usdz oslo make test-fem-visuals`. This
preparation does not activate FEM in the simulator menu or synchronize CPU TF
consumers with GPU poses.

## Telemetry and validation

Carrier link values publish `track_travel_m`, `track_speed_mps`,
`track_torque_nm`, `track_force_n`, `track_normal_load_n` and
`track_contacts`. The controller's wheel encoders report sprocket angle
(`travel / radius`) and angular speed. The controller state carries pose,
heading, roll, pitch, forward speed and yaw rate of the chassis.

`GEARBOX_TRACK_ASSET=/absolute/machine.usdz oslo make test-tracked-machine`
runs the imported-asset CPU regression; `oslo make test` runs the generic
tests.

This is a reduced continuous-belt model: no individual shoe contact, elastic
belt tension, derailment or soil excavation. The Molla track solver is
CPU-only. Follower motors approximate belt coupling; they are not an elastic
belt transmission.
