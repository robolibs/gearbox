# gearbox control scripts

Python helpers for driving a running Gearbox over agentio/peerbus.

For one-off work from the shell use the `gearbox` CLI instead (`make
build-bins`, then `target/debug/gearbox --help`; the command tree is in
`CLI.md`). `gearbox run` launches the sim, and every script below works
against an instance started that way.

Every script goes through `gearbox_client.py`, which finds the running host
in the registry (`$XDG_RUNTIME_DIR/gearbox/<name>.json`), talks to the host
agent for scene work, and to each machine's own agent for control:

| what | topic (hosted by) |
|------|-------------------|
| host info, clock, clear, scene list, events | `/gearbox/...` (host agent) |
| load / delete USDs, markers | `/gearbox/usd/*`, `/gearbox/marker/*` (host agent) |
| debug draws (`gb.draw`) | `/gearbox/draw`, `/gearbox/draw/list` (host agent) |
| machine list with each machine's did and address | `/gearbox/machines/list` (host agent) |
| claim, cmd_vel, release, session, info | `/machines/<ns>/...` (that machine's agent) |
| state, odom | `/machines/<ns>/state`, `/machines/<ns>/odom` (that machine's agent) |
| link tree, link poses | `/machines/<ns>/links`, `/machines/<ns>/tf` (`Machine.links()`, `Machine.tf()`) |
| attach, detach, attachments | `/machines/<master>/tools/*` (`Machine.attach()`, `.detach()`, `.tools()`) |
| link values (move a link, set a setpoint) | `/machines/<ns>/cmd` with `link` + `name` (`Machine.set_value(link, name, value)`); values ride `/links` props `value.<name>` |

A machine is driven by whoever holds its session: `claim()` first, then
`cmd_vel(v, w)` at your own rate. Silence longer than the claim's hold time
zeroes the command; a minute of silence releases the machine, and a client
whose session lapsed claims again on its next `cmd_vel`.

## Debug draws

`gb.draw` keeps shapes in the viewer per `(ns, id)` until they are deleted,
cleared, or their `lifetime` (sim seconds, 0 keeps them) runs out; sending an
entity again replaces it. Big entities are split into chunks and swapped in
whole once the last one lands. The `gearbox draw` CLI sends the same.

| frame | coordinates |
|-------|-------------|
| `"world"` (default) | home site, Z-up: x east, y north, z up, the axes machine states report in |
| `"<machine>/odom"` | that machine's own datum, the origin its reported pose is measured from: draw at `state.x, state.y, state.z` |
| `"<machine>/<link>"`, `"<machine>"` | the link's own axes (the base link for a bare machine id), following it |

Points are `(x, y)` or `(x, y, z)`; `pose` places the whole entity in its
frame as `(x, y, z)`, `(x, y, z, yaw)` or `(x, y, z, qw, qx, qy, qz)`. Widths
and radii: > 0 metres, < 0 pixels. Colours: `"#rrggbb[aa]"`, a name, `(r, g,
b[, a])` in 0..1 when any is a float, else 0..255; per-item `colors` beat
`color`.

Every entity has a `style` (default `"auto"`). Auto picks one from the kind
and role words in the ns or id, and with no colour given draws in the role's
colour (`ROLE`, Okabe-Ito):

| words in ns/id | kind | style | look |
|----------------|------|-------|------|
| path, plan, route | lines | `ribbon_flow` | flat ribbon 0.35 m wide, bright rims, chevrons flowing at 1.5 m/s, sky blue |
| horizon, predict, mpc | lines | `tube_pulse` | tube fading to 15 % at its tail, one pulse a second running along, green |
| rollout, sample | lines | `hairline` | 1.5 px lines added as faint light, overlaps build a density map, green |
| trace, history, trail | lines | `light_wall` | 0.25 m wall of light fading towards the start, white |
| footprint | lines, shapes | `decal` | outline on the ground with a fill fading inward |
| goal | points, arrows, shapes | `beacon` | ground ring with ticks and ripples, 12 m beam of light, yellow |
| waypoint | shapes | `holo` | glowing edges, faint faces, scanline, short beam, sky blue |
| obstacle, pred | shapes | `shield` | fresnel dome or ellipsoid with ripples and a ground ring, purple, turning vermillion with `urgency` |
| lookahead | points | `orb` | white-hot core and pulsing halo, orange |
| alert, collision | lines, shapes | `alert` | hazard stripes, flashing, on top, vermillion |
| (others) | points / arrows / text / rest | `point_glow` / `arrow_glow` / `label` / `plain` | |

Effects `glow`, `pulse_hz`, `flow_speed`, `spacing`, `beam_height`, `fade`
and `urgency` take the style's default when left out (or 0) and switch off
below 0; `flat=True` lays lines and points on the frame's XY plane and
`additive=True` adds them as light.

```python
d = gb.draw
F = "husky/odom"                          # draw in the frame the state is in
d.lines("lab/husky", "path", path, frame=F, labels=["path"])                   # ribbon_flow
d.extend("lab/husky", "trace", [(s.x, s.y, s.z)], frame=F, max_points=3000)   # light_wall
d.lines("lab/husky", "horizon", horizon, frame=F, lifetime=0.3)                # tube_pulse
d.lines("lab/husky/rollouts", "dwa", strips=rollouts, frame=F, lifetime=0.3)    # hairline
d.points("lab/husky", "lookahead", [carrot], frame=F, lifetime=0.3)             # orb
d.shapes("lab/obstacles", "pred", "ellipse", centers, sizes=diameters, yaws=yaws,
         urgency=0.6, lifetime=0.3)                                            # shield
d.arrows("lab/husky", "goal", poses=[(gx, gy, 0.1, gyaw)], length=1.2)         # beacon
d.lines("lab/husky", "footprint", corners, loop=True, frame="husky/base_link")  # decal
d.lines("lab/husky", "plan", plan, style="plain", color="#ff9f1c", width=-3)   # opt out
d.text("lab", "title", [(0, 0, 2)], text="pure pursuit", size=16)
d.clear("lab/husky")          # the path and everything below it
```

```bash
gearbox draw lines lab path --frame husky/odom --label path -- 0,0 5,0 5,5
gearbox draw shapes lab obstacle --shape ellipse --size 1.2,0.6 --urgency 0.8 --lifetime 2 -- 3,2,0.05,0.4
gearbox draw arrows lab goal --style beacon --beam 8 -- 6,4 7,4.5
gearbox draw list lab
gearbox draw clear lab
```

Kinds: `lines` (strips, `loop=True`, `segments=True`, `strips=[...]` for many
in one entity, `dashed=True`), `points` (spheres, or flat discs with `disc=True`),
`arrows` (`origins` + `vectors`, `tips`, or `poses`), `shapes` (`box`,
`ellipsoid`, `cylinder`, `cone`, `capsule`, `ellipse`; `sizes` are full
extents, `half_sizes` half; `fill="solid"`, `"wire"` or `"both"`),
`triangles`, `text`, `axes`. Every kind takes `frame`, `pose`, `color`,
`width`, `lifetime`, `labels`, `on_top` (drawn over everything) and
`snapshot` (keep the frame's pose at arrival instead of following it).

Each call waits for the host's answer and returns its `Status`. A control
loop that draws every tick sets `gb.draw.wait = False` (or passes
`wait=False`): sends then go through a background thread and return at once,
a newer send of an entity replaces an older one still queued, `gb.draw.flush()`
waits for the queue and `gb.draw.errors` lists what the host refused.
`with gb.draw.batch():` does the same for a block and flushes on leaving
(`every=n` flushes every n sends).

More kinds and widgets (widgets live under `ns/id`):

| group | calls |
|---|---|
| values | `values=` on lines, points, arrows and shapes, coloured through `colormap=` (`turbo`, `viridis`, `rdylgn`, `costmap`, `map`, `raw`, `okabe`, `rainbow`, `grey`) over `vmin..vmax` |
| polygons, grids | `polygon(rings, mode=, z_offset=, color_mode=)`, `zone(rings, kind=keepout\|slow\|stop)`, `grid(cells, cols, rows, cell=, colormap=, unknown=, heights=, contours=)`, `grid_patch(x, y, w, h, cells)` |
| frames, paths | `frame(name, pose, parent=)`, `pose_array(poses, glyph=)`, `waypoints(points, current=)`, `path(points, pose_markers=)`, `ee_path`, `rollouts(strips, costs, collisions)`, `band(points, sigma)`, `odometry(pose, velocity=, cov=)` |
| uncertainty | `covariance(mean, cov)` (2×2, 3×3, 6×6), `particles(poses, weights)` |
| sensors | `cloud(points, values=, axis=, quant=, decay_time=)`, `scan(ranges, angle_min=, angle_increment=)`, `sweep`, `range_cone`, `camera_frustum(pose, hfov=, vfov= or K=)`, `detection3d(dets)`, `detection2d(camera_pose, boxes, ...)`, `wrench`, `contact` |
| field | `field(boundary)`, `headland(outer, inner)`, `swaths(lines, width=, done=)`, `turns`, `ab_line(a, b)`, `coverage(counts, cols, rows)`, `implement(width, sections_on)` |
| robots, charts | `ghost(machine, poses)`, `ghost_trail`, `badge(machine, text)`, `sparkline`, `bars`, `gauge`, `chip`, `table`, `legend`, `tf_tree(machine)` |
| annotations | `ruler(a, b)`, `scale_bar`, `compass`, `callout`, `angle`, `graph(nodes, edges)`, `voxels(centers)`, `mesh(asset \| vertices, faces)` |
| header only | `update(ns, id, pose=, color=, style=, ...)` moves or restyles a live entity without resending it; `patch(ns, id, offset, points=, ...)` |
| interaction | `step(caption)` waits for the Next button, `pick_point()` and `pick_pose()` return what is clicked on the ground, `handle(ns, id, pose)` gives a 6-DoF gizmo whose `poll()` returns `pose_update`, `mouse_down`, `menu_select` events |

`gb.draw.tools(frame=, ns=, global_scale=10)` is the rviz_visual_tools
facade: `viz.sphere(p, Color.BLUE, Scale.MEDIUM)`, `viz.arrow`, `viz.axis`,
`viz.cuboid`, `viz.cylinder`, `viz.cone`, `viz.plane`, `viz.line_strip`,
`viz.path`, `viz.polygon`, `viz.text`, `viz.mesh`, `viz.delete_all`, with
ids numbered per ns, `Color` (rviz names on Okabe-Ito, `Color.RAND`,
`Color.scale(v)`) and `Scale` (`Scale.px(n)` for pixels).

## Setup

Use the repo dev shell, which puts `.python-packages` on `PYTHONPATH`:

```bash
nix develop --impure
```

The `peerbus` and `datapod` wheels are built from the sibling checkouts:

```bash
(cd ../datapod && maturin build --release --features python -o ../gearbox/target/wheels)
(cd ../peerbus && maturin build --release --features python -o ../gearbox/target/wheels)
pip install --no-deps --target .python-packages --upgrade target/wheels/*.whl
```

## Scripts

| script | what it does |
|--------|-------------|
| `gearbox_client.py` | The client library: `Gearbox()`, `gb.load(...)`, `gb.machine(ns)`, `m.claim()`, `m.cmd_vel()`, `m.state()`. |
| `oxbo_flatland.py` | Load flatland + one Oxbo USD machine. |
| `oxbo_follow_points.py` | Load the pea field + Oxbo, then drive it around waypoint points. |
| `oxbo_follow_points_multi.py` | One Oxbo per `--route "x z x z ..."` (default two), each following its own route. |
| `oxbo_maptrax_field.py` | Plan a GPS field with maptrax, draw the lines in Gearbox and rerun, drive one Oxbo per machine. |
| `oxbo_joystick.py` | Load flatland + Oxbo, claim it, drive it from `/dev/input/warpout0`. |
| `hunter_spawn.py` / `hunter_drive.py` / `barn_spawn.py` | Spawn AgileX machines on flatland or in the de Marke barn, drive them in circles. |
| `bale_run.py` | One USD tractor collects USD bales on the sim's own ground (meadow, wheat); no USD terrain is loaded. |
| `bale_run_multi.py` | Same with several tractors, each under its own namespace and agent. |
| `stop.py <namespace>` | Take over a machine and send zero `cmd_vel`. Default namespace: `oxbo`. |

Without a GPU, the same scripts run against the fake host, which spawns a
kinematic machine per machine load and reports bale poses and harvests:

```bash
cargo run -p gearbox-api --no-default-features --example fake_host -- fake
python scripts/control/bale_run_multi.py 2 10 60
```
