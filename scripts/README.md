# gearbox control scripts

Python helpers for driving a running Gearbox over agentio/peerbus.

For one-off work from the shell use the `gearbox` CLI instead (`make
build-bins`, then `target/debug/gearbox --help`; the command tree is in
`CLI.md`). `gearbox run` launches `gearbox-sim`, and every script below works
against an instance started that way.

Every script goes through `gearbox_client.py`, which finds the running host
in the registry (`$XDG_RUNTIME_DIR/gearbox/<name>.json`), talks to the host
agent for scene work, and to each machine's own agent for control:

| what | topic (hosted by) |
|------|-------------------|
| host info, clock, clear, scene list, events | `/gearbox/...` (host agent) |
| load / delete USDs, markers | `/gearbox/usd/*`, `/gearbox/marker/*` (host agent) |
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
python scripts/bale_run_multi.py 2 10 60
```
