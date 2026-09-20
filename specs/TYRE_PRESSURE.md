# Pressure tyres in the isolated Molla backend

Run this worktree with `GEARBOX_PHYSICS=molla`. Rapier retains rigid tyres;
it does not implement these pressure mechanics.

Procedural field layouts can author `default_friction` and per-region `friction`
for footprint sampling. Missing values retain collider friction; no coefficients
are inferred from visual profile names. See the [field friction contract](../crates/gearbox-fields/README.md#physical-friction-in-the-molla-worktree)
for schema, interpolation, update semantics and current terrain/backend limits.

## Required machine asset contract

**REQUIRED CONTRACT — authoring requirement; loader enforcement incomplete.**
This is part of full Gearbox machine compatibility, not an optional feature
for selected brands. Loading successfully does not establish compliance.

Every machine must explicitly identify its ground-contact model. Every
pneumatic wheel, including passive trailer/implement wheels and casters,
must provide the following information:

| Required information | Contract |
|---|---|
| Wheel identity and motion | Stable wheel link, owning machine, wheel rigid body, real rolling joint, hub centre and rolling axis. Steering is a separate joint where applicable. |
| Contact type | Pneumatic tyre, rather than inferring it from a wheel name. Solid wheels, tracks and other contact models declare their actual type. No-wheel machines declare tyre requirements inapplicable. |
| Mesh bindings | Explicit deformable rubber/tread mesh targets and separate rigid rim/hub targets, all owned by this wheel. Collider helpers are not rubber. Renaming meshes must not change behaviour. |
| Reference geometry | Unloaded outer radius including tread, bead/rim radius, width and reference pressure, in declared units. Reference geometry stays immutable during inflation and wheel rotation. |
| Pressure configuration | Initial gauge pressure, supported minimum/maximum, and positive bounded inflation/deflation rate. User-facing pressure is gauge bar; solver pressure is Pa. |
| Tyre response | Carcass stiffness/damping, tread/material and hysteresis parameters with a documented supported load/pressure range. Simulation defaults are not manufacturer calibration. |
| Axle identity | Explicit axle/group membership for consistent whole-machine, axle and individual-wheel controls. |

Solid wheels, tracks and non-wheel implements are exempt from pneumatic
pressure/deformation fields, not from declaring the appropriate contact type.
Decorative wheels must be distinguishable from load-bearing ones.

### Existing fields and implementation gaps

The isolated adapter already consumes these numeric wheel-link attributes:

| Attribute | Units |
|---|---|
| `gearbox:value:tyre_axle` | Positive integer axle id |
| `gearbox:value:tyre_pressure_bar` | Initial gauge bar |
| `gearbox:value:tyre_min_pressure_bar` | Gauge bar |
| `gearbox:value:tyre_max_pressure_bar` | Gauge bar |
| `gearbox:value:tyre_pressure_rate_bar_s` | Bar per simulated second |
| `gearbox:value:tyre_width_m` | Metres |
| `gearbox:value:tyre_carcass_stiffness_pa_m` | Pa/m |
| `gearbox:value:tyre_tread_stiffness_n_m3` | N/m³ |
| `gearbox:value:tyre_damping_ratio` | Dimensionless |
| `gearbox:value:tyre_hysteresis_fraction` | Dimensionless |

Explicit contact-type, rubber/rigid mesh-binding and reference-geometry
schema fields are not yet defined/consumed. They must be implemented before
the loader can certify this contract. The existing mesh-name recognition,
collider/mesh dimension inference and unauthored defaults remain legacy import
fallbacks; they must not silently certify an asset as fully compatible.

### Compatibility validation and acceptance

The compatibility validator must reject or report non-compliant pneumatic
wheels with the machine id, wheel path, missing/invalid field and required
correction. It must detect unresolved/cross-wheel mesh bindings, conflicting
rigid/rubber membership, missing rolling joints, inconsistent geometry,
non-finite/non-positive dimensions and rates, and invalid pressure ranges.
Legacy preview loading may continue with a clear degraded-status report.
An unsupported backend must report the missing capability, not silently
present rigid tyres as pressure-compatible behaviour.

Each machine's acceptance test must demonstrate:

1. All pneumatic wheels are discovered without name/brand heuristics, with
   correct axle membership and pressure controls, including passive wheels.
2. At fixed load and surface, minimum/reference/maximum pressure and both
   transition directions change support, loaded radius, footprint and visible
   rubber consistently. High-pressure rubber is rounder, but remains loaded.
3. Rubber stays on the sampled ground; rims/hubs remain rigid. Inflation does
   not teleport the chassis, reset wheel speed or create duplicate support.
4. Paused/running edits, per-wheel/axle/all groups, supported save/reload,
   attach/detach and unrelated spawn/remove operations retain valid state.
5. Renaming mesh prims while updating explicit relationships leaves behaviour
   unchanged; missing metadata cannot silently disable deformation.

These are required gates, not a claim that every gate or machine has already
passed. Current live visual coverage is the Kubota; Krampe naming recognition
alone does not establish trailer compatibility.

## Controls

The Machine sidebar has all-tyre, axle and individual-wheel target sliders.
Group ranges are the intersection of member ranges. Mixed targets remain
unchanged until the group slider is edited. Applied pressure is separate
from the requested target: target edits work while paused; inflation/deflation
advances only during successful simulated steps.

The sidebar identifies the instance and active physics backend. Its pressure
status distinguishes paused, adjusting and at-target states; applied pressure
is shown above the group slider, and each wheel reports deflection in mm.
Only loaded rubber flattens and bulges; a pressure target is not immediate
whole-wheel scaling. At low frame rates the simulation clock, and therefore
inflation/deflation, can run slower than wall time.

Launch this isolated build separately from the original Gearbox:

```sh
GEARBOX_PHYSICS=molla nix develop --impure -c oslo make run '--args=--name molla-tyres --ephemeral'
```

Use `-i molla-tyres` for its CLI controls; do not rely on the default instance
when the original Gearbox is also running.

```sh
gearbox -i INSTANCE machine tyre-pressure 1.0 --axle 1 --machine tractor
gearbox -i INSTANCE machine tyre-pressure 2.4 --axle 2 --machine tractor
gearbox -i INSTANCE machine tyre-pressure 1.8 --machine tractor
gearbox -i INSTANCE machine tyre-pressure 1.2 --wheel wheel_front_left --machine tractor
gearbox -i INSTANCE machine tyre-pressure 2.4 --machine tractor --reuse-session
gearbox -i INSTANCE --json machine state tractor
```

During a CLI drive, pass `--reuse-session` to borrow the active session held
by the same CLI identity. The pressure command neither claims nor releases
that session, including on command failure or acknowledgement timeout. It
refuses an idle session or a different reported holder, and cannot be combined
with `--take`. Without the flag, pressure commands claim and release their own
short session; a busy driving session is not automatically stolen. `--take`
explicitly steals ownership and can interrupt the drive. The reuse check is
CLI-side holder validation, not additional server-side authentication.

The command uses one grouped request and confirms accepted target telemetry
before reporting success. It does not wait for applied pressure to reach the
target. Invalid, unsupported or out-of-range edits cannot partially change
the selected group. Other command publishers can use the controller-command
property `tyre_pressure_scope=all|axle:N|wheel:LINK`, with `value` in gauge bar.
An ordinary command acknowledgement only confirms queueing, not application.

## Physical telemetry

Machine state exposes the solver readout as `link.LINK.tyre_*` properties:

| Suffix | Meaning / units |
|---|---|
| `in_contact`, `held_support` | Numeric booleans, 0 or 1 |
| `normal_load_n`, `grip_budget_n` | Mean normal load and available grip, N |
| `slip_ratio`, `slip_angle_rad` | Signed dimensionless ratio and angle in radians |
| `force_world_{x,y,z}_n` | Resultant tyre force, N |
| `aligning_moment_world_{x,y,z}_nm` | Aligning moment, N m |
| `rolling_moment_world_{x,y,z}_nm` | Rolling-resistance moment, N m |
| `rolling_moment_nm` | Magnitude of the rolling-resistance moment, N m |

Vectors use the physics world frame (Y up), not the robotics odometry frame.
Resultant force includes normal support and tangent traction; it excludes
joint motor forces and obstacle contacts. Aligning and rolling moments exclude
the contact-force lever arm about the wheel COM. Wrenches are taken from the
actual post-limiter solver samples, not recomputed from slip or friction.

Active force/moment readouts are duration-weighted over the last completed
physics step, including zero contributions when contact is absent. Slip and
contact-point readouts are weighted by normal impulse. Changing the configured
timestep does not rescale the previous step's output. Applied/target pressure
and hub geometry remain current-state readouts; footprint dimensions retain
the last contact sample. Paused target edits do not advance inflation.

`held_support=1` marks a sleeping tyre's retained reaction. It is not an
impulse applied during that step; sleeping bodies are not integrated. Loss of
contact or invalidation clears forces, moments and contact flags. Existing
`normal_force`, `slip` and `slip_angle` track properties remain available.

## Axle identity

`gearbox:value:tyre_axle` is an optional integer from 1 to 65535 on a wheel
link. The accepted id is exposed as `link.LINK.tyre_axle` in machine state.

Without metadata, discovery groups wheel-body centres within 0.25 m along
the chassis forward direction. Rows are numbered from front to rear using
unclaimed ids. A single authored id in a row applies to its unauthored members;
conflicting authored ids with unauthored members are rejected as ambiguous.
Grouping is cached until wheel membership or authored ids change, so steering
and driving do not renumber the axles. Closely spaced, unusual or articulated
axle layouts should author explicit ids instead of relying on inference.

## Model and limits

Defaults are inspectable, uncalibrated simulation values: 1.8 gauge bar,
0.5–4.0 bar bounds, and 1.0 bar per simulated second. Authored
`gearbox:value:tyre_pressure_rate_bar_s` overrides the default. These are not tyre
manufacturer specifications or real-machine inflation advice.

Molla uses Pa internally. Pressure-dependent brush support supplies normal
load, loaded radius, footprint and grip. Imported rubber deformation follows
that loaded radius using a compact contact-envelope approximation; hubs/rims
stay rigid. It is not a calibrated FEM carcass solution. Current visual
selection recognizes Kubota rubber names and Krampe BKT naming, not arbitrary
imported meshes. Nonplanar/camber agreement, general machine compatibility and
scene-scale performance gates remain incomplete.

At first wheel registration, available undeformed rubber meshes define the
outer support radius, including tread lugs, in the wheel body's frame. Rims,
hubs and collider helpers are excluded. The radius stays fixed as rubber
deforms; collider radius is the fallback when no matching mesh is available.
Inflation changes support stiffness and chassis height, not this reference
radius. The loaded tyre retains a contact patch even at maximum pressure.

## Rubber rendering

The viewer deforms eligible static rubber meshes in the vertex shader. Source
vertex buffers stay resident; per-mesh contact transforms, the loaded envelope
and ground plane occupy persistent 320-byte GPU buffers. Updating those buffers
does not rebuild the materials or their texture bindings. Forward, shadow and prepass vertices use the
same deformation. Normals and tangents follow the deformed surface, and bounds
conservatively include the displacement. Pressure physics and collision
support remain in the backend, not in the material.

Standard-material edits and reassignment propagate to the tyre material.
Removing wheel support restores the source material and bounds. Rims and hubs
remain on their original meshes/materials. Skinned, morphed or non-standard
materials retain the CPU path. `GEARBOX_TYRE_RENDER=cpu` selects that reference
path for comparison; `GEARBOX_TYRE_MESH_TRACE=1` logs GPU/CPU mesh counts and
CPU update costs.

The ignored Vulkan test `gpu_envelope_matches_cpu_pressure_and_ground_sweep`
executes the production deformation WGSL over 124,416 pressure/geometry/ground
cases and compares positions with the f64 CPU reference. This is geometric
parity, not validation of a physical tyre model or all render pipeline variants.

## Machine configuration persistence

Pause before exporting a standalone pressure-tyre machine:

```sh
gearbox -i INSTANCE scene pause
gearbox -i INSTANCE machine save tractor.toml --machine tractor
gearbox -i INSTANCE clear usd tractor
gearbox -i INSTANCE spawn from tractor.toml --wait-timeout 90
gearbox -i INSTANCE scene play
```

The version-1 TOML configuration stores the absolute source asset path,
variant selections, scene-root placement/yaw, and each wheel's applied and
target gauge pressure. Source files are not modified. Publication refuses to
overwrite an existing file, including a source asset or symlink. Restoring
requires a fresh machine id and leaves the simulation paused. Readback must
confirm both pressures before `spawn from` reports success. Play resumes the
bounded transition from the saved applied value toward its saved target.
Machine-agent ownership can take time to expire after unloading an ephemeral
machine; manifest loading allows 90 seconds by default, independently of the
per-request `--timeout`.

Pressure entries are validated together against the loaded source's wheel
links and supported ranges before inventory publication. Failed restores
report a rejection and remove the rejected instance without changing another
machine's pressure. Delete/reset also clear cached values for removed ids.
Pressure continues to survive unrelated runtime rebuilds with unchanged wheel
handles. Scene reset replays the recorded configuration files.

This is a machine configuration, not a full simulation checkpoint. Velocities,
joint positions, contact history, terrain and other scene objects are not
saved. Attached or multi-machine assets and runtime USD attribute overrides
are refused rather than silently dropped. Source files must remain available;
the export is not a self-contained asset package. A synchronized whole-scene
save and articulated solver warm-state restoration remain separate work.
