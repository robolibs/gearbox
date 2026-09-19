# Pressure tyres in the isolated Molla backend

Run this worktree with `GEARBOX_PHYSICS=molla`. Rapier retains rigid tyres;
it does not implement these pressure mechanics.

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
