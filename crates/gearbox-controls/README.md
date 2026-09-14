# Gearbox controls

Dependency-free operator-input routing. This crate owns layers, dead zones,
dead-man arming and navigation intents. It does not know about Bevy, USD,
Rapier, machine identities or camera entities.

`bin/gearbox/src/viewer/drive/` adapts Bevy gamepad samples to this crate and
applies the resulting intents through Gearbox's existing control/camera paths.
Vehicle kinematics and torque remain in `bin/gearbox/src/controller/`.

## Layers

| Modifier | Sticks | D-pad |
|---|---|---|
| None | Right: orbit. Left Y: move forward/back; left X: strafe. | Left/right: previous/next machine and fly to it. Up: follow; down: unfollow. |
| R1 / right bumper held | Left Y: forward/reverse; left X: steering. Right stick unused. | Unused. |
| L1 / left bumper held | Reserved for machine internals; no actions yet. | Unused. |

L1 takes priority over R1. South/cross stops and requires re-arming. R1 must
be released after connection, focus loss, machine changes or entering L1.
In normal mode R2 raises the camera and L2 lowers it, with proportional trigger
pressure. Equal pressure cancels the vertical movement. Triggers do not change
layers and have no actions in the vehicle or machine layers yet.
Left-stick movement translates the camera and its focus together on the ground
plane, preserving height and orbit distance. It does not zoom or stop at a zoom
limit. The mouse wheel still zooms; R2/L2 control height independently.
Camera and vehicle stick outputs are mutually exclusive. A held D-pad button
does not repeatedly cycle machines; the adapter supplies press edges.

Normal camera controls work without selecting/enabling a machine. R1 acquires
the selected machine's enabled drive controller. Navigation also includes passive
machines, which can be viewed/followed but cannot be driven. Switching machines
stops/releases the old viewer drive and transfers an existing follow target.
The machine order matches the alphabetical Machines list and wraps at either end.

Releasing R1, disconnecting or losing focus issues a zero drive command without
the normal command slew. The physics brakes stop the vehicle; this is not an
instant velocity teleport or a safety-rated hardware emergency stop.
The adapter pins one gamepad until it disconnects. Mouse/keyboard bindings remain unchanged.
Follow pins the camera focus to the actual chassis. Gamepad translation and lift
are blocked until D-pad Down releases the lock; mouse-wheel zoom remains available
both in free mode and while following. It tracks body motion whether driven locally
or externally. R1 release sends one stop, then relinquishes the local override.

The bottom-left joystick icon opens Controller and keyboard settings, bindings and
events. Vertical right-stick inversion defaults on and can be disabled there;
preferences are saved in `$XDG_CONFIG_HOME/gearbox/controls.json` (or
`~/.config/gearbox/controls.json`). Inversion does not affect steering or mouse input.
The drive menu is always available: sliders submit commands only when changed,
and changing selection stops the previous local drive. R1 targets the current
selection only, after host/API/mouse selection updates; it never selects a machine
implicitly. D-pad Left/Right also selects when there is no current selection.
Focus is forwarded from Mara's egui host before each Bevy tick. The embedded
Bevy renderer deliberately has no `Window` entity; querying Bevy windows would
incorrectly disable every input. `GEARBOX_CONTROLS_DEBUG=1` logs camera intents.

## Checks

Regression cases cover exclusive layers, L1 priority, trigger lift, release-to-rearm, disconnected
or unfocused input and machine-list wrapping. Tests are authored but not run under
the current no-automated-tests constraint. Build the preview with `make build-bins`.

The live preview confirmed right-stick orbit, left-stick depth movement and R1
press/release layer transitions on both loaded machine roots. See
`/tmp/gearbox-controls-ready.log` and `/tmp/gearbox-controls-panel.png`.
The initial host adapter incorrectly queried Bevy windows; the Mara focus bridge
corrected the always-inactive state. This is interactive evidence, not a full
automated input-device or disconnect/focus regression run.
