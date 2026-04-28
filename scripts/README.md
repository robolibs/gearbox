# gearbox zenoh control scripts

Python helpers for poking at the running gearbox simulator's
`<robot_name>_<instance>/cmd_vel` / `odom` / `fix` topics.

## Setup (once)

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install eclipse-zenoh cbor2
```

(Or install globally — `pip install --user eclipse-zenoh cbor2`.)

## Scripts

| script | what it does |
|--------|-------------|
| `discover.py` | Wildcard-subscribes to `**/odom` and prints every key it sees — fastest way to confirm what's alive. |
| `watch.py <vehicle>` | Tails one vehicle's `odom` + `fix` topics. Default vehicle: `tractor_0`. |
| `drive.py <vehicle> [linear] [angular]` | Spams a constant `cmd_vel` (m/s, rad/s) at 10 Hz until Ctrl-C. |
| `stop.py <vehicle>` | One-shot zero `cmd_vel`. |
| `square.py <vehicle>` | Drives in a 4-second square. |

`<vehicle>` is `<robot_name>_<instance>` — e.g. `tractor_0`, `husky_2`.
The starter tractor is always `tractor_0`. Spawn more from the
Library panel and ids increment.
