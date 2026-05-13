# Gearbox USD machine/controller metadata

This is **not** an OpenUSD generated schema/plugin.

This file documents the plain USD attributes and relationships that Gearbox
currently scans directly from assets like `bin/gearbox/assets/tractor.usd`.
The intent is simple:

> The machine USD declares what controllers it wants, and Gearbox attaches them.

No TOML/YAML sidecar is required.

## Design direction: Gearbox Robot Description API

The current metadata is intentionally small, but the long-term API should be
closer to a robot-description IR than a single controller tag. The model to
follow is:

- **LinkForge-style core**: keep a host-independent robot description that can
  be authored from plugins/editors and exported or imported without losing
  semantics.
- **Isaac Robot Wizard-style authoring**: users should be able to pick links,
  visuals, colliders, joints, drives, sensors, and controller interfaces in a
  UI, then save those choices into USD.
- **Gearbox runtime binding**: Gearbox reads the USD description, validates it,
  normalizes it into runtime specs, then attaches supported controllers.

So the Gearbox USD API should have four layers:

```text
robot root
├── structure     links, bodies, joints, articulation root
├── geometry      visuals, colliders, materials, physics approximations
├── semantics     wheel roles, steering roles, sensors, tools, implements
└── control       controller instances, command/state interfaces, plugins
```

This means the schema should describe **what the machine is**, not only
**which controller binary to run**.

### Authoring plugin workflow

A future Blender/Isaac/Gearbox plugin should behave like a wizard:

1. Select or create the robot root.
2. Mark child prims as links/bodies.
3. Assign visual and collision geometry.
4. Configure mass, inertia, center of mass, collision approximation.
5. Create joints between links.
6. Assign joint roles:
   - steering
   - powered wheel
   - passive/free wheel
   - brake
   - lift/arm/tool
7. Add sensors and implements.
8. Add one or more controller profiles.
9. Run validation before saving.

The plugin should write plain USD attributes/relationships. Gearbox can later
offer formal UI panels, validators, and import/export adapters, but the USD
asset stays the source of truth.

### Canonical naming direction

The current names are controller-specific:

```usda
rel gearbox:controller:drive:driveWheelJoints = [...]
rel gearbox:controller:drive:passiveWheelJoints = [...]
```

For a better API, keep durable physical structure in normal USD/UsdPhysics and
move only robot semantics to machine-level roles:

```usda
rel gearbox:machine:visuals = [...]
rel gearbox:machine:colliders = [...]
rel gearbox:machine:sensors = [...]

rel gearbox:machine:role:poweredWheelJoints = [...]
rel gearbox:machine:role:passiveWheelJoints = [...]
rel gearbox:machine:role:steeringJoints = [...]
rel gearbox:machine:role:brakeJoints = [...]
rel gearbox:machine:role:toolJoints = [...]
```

Controllers should then reference roles instead of repeating every joint list:

```usda
token gearbox:controller:drive:type = "builtin:ackermann_cmd_vel"
token[] gearbox:controller:drive:usesRoles = [
    "poweredWheelJoints",
    "steeringJoints"
]
```

Controller-specific relationships remain allowed as overrides, but the default
should be role-driven. This scales better for six-axle machines, implements,
robot arms, and future plugin-generated assets.

Do **not** duplicate the real kinematic tree as `gearbox:machine:links` or
`gearbox:machine:joints`. Links/bodies are regular USD prims with
`PhysicsRigidBodyAPI`, and joints are regular `UsdPhysics` joint prims with
`physics:body0` / `physics:body1`. Gearbox roles point at those normal USD
physics joints.

### Validation model

A LinkForge-like validator should run before attaching controllers:

- exactly one articulation/root body for a mobile base;
- all relationship targets exist after USD composition;
- every controlled joint has a valid body0/body1 pair;
- steering joints are not also powered wheel joints unless explicitly allowed;
- passive wheel joints are not motorized by a drive controller;
- mass and inertia are finite and positive;
- collision shapes do not obviously overlap at spawn;
- controller command/state interfaces are supported by the runtime;
- plugin/external process controllers pass allowlist policy.

Validation errors should be visible in the UI, not discovered only after the
simulation explodes.

## Marking a machine

Apply `GearboxMachineAPI` to the machine root prim and author machine metadata
directly on that prim:

```usda
def Xform "robot" (
    prepend apiSchemas = ["GearboxMachineAPI"]
)
{
    token gearbox:machine:kind = "tractor"
    token gearbox:machine:idPolicy = "prim_path"

    rel gearbox:machine:body = </robot/chassis>
}
```

Reusable machine assets should usually **not** author a fixed
`gearbox:machine:id`, because the same asset may be referenced many times in a
world. Use:

```usda
token gearbox:machine:idPolicy = "prim_path"
```

Then Gearbox derives an id from the composed prim path:

```text
/World/Tractor_01 -> world_tractor_01
/World/Tractor_02 -> world_tractor_02
```

A world layer may override this for a specific instance:

```usda
over "/World/Tractor_03"
{
    string gearbox:machine:id = "leader"
}
```

## Adding a controller

Apply a controller token to the same machine prim:

```usda
def Xform "robot" (
    prepend apiSchemas = [
        "GearboxMachineAPI",
        "GearboxControllerAPI:drive"
    ]
)
{
    token gearbox:controller:drive:type = "builtin:ackermann_cmd_vel"
    token gearbox:controller:drive:commandInterface = "cmd_vel"
}
```

`drive` is the controller instance name. Multiple controllers can live on the
same machine:

```usda
prepend apiSchemas = [
    "GearboxMachineAPI",
    "GearboxControllerAPI:drive",
    "GearboxControllerAPI:arm",
    "GearboxControllerAPI:implement"
]
```

Each controller uses its own namespaced properties:

```usda
token gearbox:controller:drive:type = "builtin:ackermann_cmd_vel"
token gearbox:controller:arm:type = "builtin:joint_position"
```

## Controller namespaces

By default, controller command/state topics use the resolved machine id:

```usda
token gearbox:controller:drive:namespacePolicy = "machine_id"
```

You can override the namespace on a specific composed instance:

```usda
over "/World/Tractor_03"
{
    string gearbox:controller:drive:namespace = "leader"
}
```

## Gearbox-authored Ackermann drive example

A Gearbox-native asset may use a builtin Ackermann `cmd_vel` controller:

```usda
rel gearbox:machine:body = </robot/chassis>
rel gearbox:machine:role:poweredWheelJoints = [
    </robot/Joints/rev_back_left>,
    </robot/Joints/rev_back_right>
]
rel gearbox:machine:role:passiveWheelJoints = [
    </robot/Joints/rev_front_left>,
    </robot/Joints/rev_front_right>
]
rel gearbox:machine:role:steeringJoints = [
    </robot/Joints/steer_front_left>,
    </robot/Joints/steer_front_right>
]

token gearbox:controller:drive:type = "builtin:ackermann_cmd_vel"
bool gearbox:controller:drive:enabled = true
float gearbox:controller:drive:updateRateHz = 60
token gearbox:controller:drive:commandInterface = "cmd_vel"
token[] gearbox:controller:drive:stateInterfaces = ["pose", "velocity", "joint_state"]

float gearbox:controller:drive:wheelBase = 2.37
float gearbox:controller:drive:trackWidth = 1.685
float gearbox:controller:drive:maxSteerDeg = 45
token gearbox:controller:drive:steeringGeometry = "ackermann"
token[] gearbox:controller:drive:usesRoles = [
    "poweredWheelJoints",
    "steeringJoints"
]
rel gearbox:controller:drive:body = </robot/chassis>
```

Gearbox scans those relationships, resolves them against the loaded composed USD
instance, and attaches the runtime controller to that specific machine.

Wheel-joint roles are intentionally separate:

- `gearbox:machine:role:poweredWheelJoints` = joints that may receive wheel
  motor velocity/force from drive controllers.
- `gearbox:machine:role:passiveWheelJoints` = rolling joints that controllers
  must leave free.
- `gearbox:machine:role:steeringJoints` = joints that may receive steering
  position targets.

For a normal rear-wheel-drive tractor, put the rear axle wheel joints in
`poweredWheelJoints` and the front wheel rolling joints in
`passiveWheelJoints`. The front wheels can still steer through
`steeringJoints`, but they do not receive drive motor force. For a future
six-axle machine, author all real joints as normal `UsdPhysics` joints, then put
only the powered axle joints in `poweredWheelJoints`, only the steering axle
joints in `steeringJoints`, and leave balance/free-rolling axles out of
`poweredWheelJoints`.

Controller-specific relationships such as
`gearbox:controller:drive:driveWheelJoints` are still supported as overrides for
older assets or unusual controllers, but the role-driven machine-level form is
preferred for new assets and authoring plugins.

`steeringGeometry = "ackermann"` follows the Isaac Sim Leatherback pattern: the
controller computes individual left/right steering position targets. In true
Ackermann, the front steering wheels are **not** the same angle; the inner wheel
steers sharper than the outer wheel. Wheel velocity targets are only applied to
`poweredWheelJoints`; passive rolling joints are not motorized by the
controller.

`steeringGeometry = "parallel"` is still supported for machines that
intentionally want both steering joints to receive the same steering position
target, but that is not the Isaac Sim Ackermann pattern.

## Adding a new machine type

1. Put `GearboxMachineAPI` on the machine root.
2. Set `gearbox:machine:kind`.
3. Set `gearbox:machine:idPolicy = "prim_path"` for reusable assets.
4. Author normal USD physics bodies/joints, then add Gearbox role relationships
   only for semantic controller roles.
5. Add one or more `GearboxControllerAPI:<name>` tokens.
6. Add controller-specific properties and relationships.

Example skeleton:

```usda
def Xform "robot" (
    prepend apiSchemas = [
        "GearboxMachineAPI",
        "GearboxControllerAPI:drive"
    ]
)
{
    token gearbox:machine:kind = "my_machine"
    token gearbox:machine:idPolicy = "prim_path"
    rel gearbox:machine:body = </robot/base>

    bool gearbox:controller:drive:enabled = true
    token gearbox:controller:drive:type = "builtin:my_controller"
}
```

## Adding a new controller type

1. Pick a controller type string, for example:

   ```usda
   token gearbox:controller:arm:type = "builtin:joint_position"
   ```

2. Add a runtime implementation in Gearbox that matches that string.
3. Define which USD relationships that controller reads.
4. Document the expected properties here.
5. Add tests that load a USD asset and verify discovery/attachment.

The USD should declare intent and targets. Gearbox runtime decides whether the
controller is supported and safe to run.

## Isaac Sim compatibility

Gearbox metadata is the preferred explicit format for Gearbox-authored assets
like `bin/gearbox/assets/tractor.usd`, but it is **not required** for imported
Isaac Sim robots.

When no `GearboxMachineAPI` metadata exists, Gearbox runs an Isaac compatibility
discovery pass:

1. Find prims with `PhysicsArticulationRootAPI`.
2. Scan Isaac/OmniGraph-style controller nodes for authored `jointNames` /
   `dofNames` arrays.
3. Treat position/steering joint-name groups as Gearbox `steerJoints`.
4. Treat velocity/wheel/drive joint-name groups as Gearbox
   `driveWheelJoints`.
5. Treat rolling wheel joints that are not powered as `passiveWheelJoints`.

This mirrors Isaac Sim's controller wiring:

- an Ackermann node computes steering angles and wheel velocities;
- an Articulation Controller applies position targets to the listed steering
  joints;
- another Articulation Controller applies velocity targets only to the listed
  wheel joints.

So an imported Isaac USD remains compatible as long as the USD/action graph
authors the same intent Isaac uses: which articulation DOFs receive steering
position commands, and which DOFs receive wheel velocity commands.

Gearbox's normalized internal mapping is:

| Isaac Sim concept | Gearbox normalized field |
| --- | --- |
| articulation root | machine root |
| steering `jointNames` receiving position targets | `steerJoints` |
| wheel `jointNames` receiving velocity targets | `driveWheelJoints` |
| rolling wheel joints not receiving velocity targets | `passiveWheelJoints` |

If an Isaac USD has only physics joints and no readable action graph/controller
joint-name lists, Gearbox falls back to name-based inference from the physics
joint paths. That fallback is intentionally best-effort; controller graphs or
explicit Gearbox metadata are more reliable.

## External process controllers

External process controllers are allowed only by runtime policy. USD may request
one like this:

```usda
token gearbox:controller:drive:type = "external:process"
string gearbox:controller:drive:executable = "/path/to/controller"
string[] gearbox:controller:drive:args = ["--some-arg"]
token gearbox:controller:drive:transport = "zenoh"
```

But Gearbox will not execute it unless the runtime explicitly allows it:

```text
GEARBOX_ALLOW_USD_CONTROLLER_PROCESS=1
GEARBOX_CONTROLLER_ALLOWLIST=/allowed/controller/dir
```

This keeps USD-authored binaries deny-by-default.

## What this is not

This is not a generated OpenUSD C++ schema plugin. We may eventually create one
for editor validation/autocomplete, but the current Gearbox implementation
intentionally reads these plain USD attributes/relationships directly.
