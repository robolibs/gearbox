# gearbox USD control scripts

Python helpers for the USD-only simulator path.

The command surface is:

- load USDs: `gearbox/usd/load/<id>`
- machine state: `gearbox/machines/<namespace>/state`
- machine commands: `gearbox/machines/<namespace>/cmd_vel`
- command ownership: `gearbox/machines/<namespace>/session`

## Setup

Use the repo dev shell:

```bash
nix develop --impure
```

## Scripts

| script | what it does |
|--------|-------------|
| `oxbo_flatland.py` | Load flatland + one Oxbo USD machine. |
| `oxbo_follow_points.py` | Load flatland + Oxbo, then drive it around waypoint points with the USD controller. |
| `oxbo_joystick.py` | Load flatland + Oxbo, claim the `oxbo` session, then drive it from `/dev/input/warpout0`. |
| `bale_run.py` | Load USD terrain + USD tractor + USD bales, then collect bales with the USD controller. |
| `bale_run_multi.py` | Same as `bale_run.py`, but with multiple USD tractor instances. |
| `stop.py <namespace>` | Claim a USD machine session and send zero `cmd_vel`. Default namespace: `oxbo`. |

The old preset/device scripts using `<robot_name>_<id>/cmd_vel`,
`gearbox/sim/spawn`, `odom`, and `fix` have been removed. New examples should
load USD assets and command the controller namespace authored/discovered from
the USD machine.
