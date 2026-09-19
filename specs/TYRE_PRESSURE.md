# Pressure tyres in the isolated Molla backend

Run this worktree with `GEARBOX_PHYSICS=molla`. Rapier retains rigid tyres;
it does not implement these pressure mechanics.

## Controls

The Machine sidebar has all-tyre, axle and individual-wheel target sliders.
Group ranges are the intersection of member ranges. Mixed targets remain
unchanged until the group slider is edited. Applied pressure is separate
from the requested target: target edits work while paused; inflation/deflation
advances only during successful simulated steps.

```sh
gearbox -i INSTANCE machine tyre-pressure 1.0 --axle 1 --machine tractor
gearbox -i INSTANCE machine tyre-pressure 2.4 --axle 2 --machine tractor
gearbox -i INSTANCE machine tyre-pressure 1.8 --machine tractor
gearbox -i INSTANCE machine tyre-pressure 1.2 --wheel wheel_front_left --machine tractor
gearbox -i INSTANCE --json machine state tractor
```

The command uses one grouped request and confirms accepted target telemetry
before reporting success. It does not wait for applied pressure to reach the
target. Invalid, unsupported or out-of-range edits cannot partially change
the selected group. Other command publishers can use the controller-command
property `tyre_pressure_scope=all|axle:N|wheel:LINK`, with `value` in gauge bar.
An ordinary command acknowledgement only confirms queueing, not application.

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
0.5–4.0 bar bounds, and 0.2 bar per simulated second. These are not tyre
manufacturer specifications or real-machine inflation advice.

Molla uses Pa internally. Pressure-dependent brush support supplies normal
load, loaded radius, footprint and grip. Imported rubber deformation follows
that loaded radius using a compact contact-envelope approximation; hubs/rims
stay rigid. It is not a calibrated FEM carcass solution. Current visual
selection recognizes Kubota rubber names and Krampe BKT naming, not arbitrary
imported meshes. Nonplanar/camber agreement, GPU parity and performance gates
remain incomplete.

## Persistence status

Pressure survives unrelated runtime rebuilds while the wheel handles remain.
The current scene reset replays spawn manifests; it is not a live simulation
snapshot. Those manifests do not yet save applied/target tyre state. A full
supported save/reload cycle remains required work; editor UI persistence does
not satisfy it.
