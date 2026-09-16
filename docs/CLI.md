------------------------------------------
Project: gearbox-sim v0.0.6
Display: wayland backend
------------------------------------------
# gearbox CLI

Launch, inspect and drive the gearbox simulator

## `gearbox run`

Launch a simulator instance

```
run [OPTIONS] [-- <SIM_ARGS>...]
```

## `gearbox instance`

Find, inspect and stop instances; pick the default

| Leaf | Does |
|---|---|
| `list` | Registry entries with liveness; the default is marked |
| `info [ID]` | `/gearbox/info` of an instance |
| `use <ID>` | Make an instance the default for later commands |
| `stop [OPTIONS] [ID]` | Ask an instance to shut down, then signal it |
| `ping [OPTIONS] [ID]` | Round-trip time of `/gearbox/info` |
| `wait [OPTIONS] [ID]` | Block until the instance answers |
| `screenshot [OPTIONS] <OUT>` | Save a screenshot of the viewer window (panes included) to a PNG |
| `camera [OPTIONS] <ACTION> [MACHINE]` | Move the viewer camera: `fly MACHINE` flies behind a machine like a double-click in the Agents pane, `follow MACHINE` pins the camera to it, `unfollow` releases it |
| `ui [OPTIONS] [STEPS]...` | Drive the window with scripted input, one step per argument or per `;`-separated part: `move X Y`, `down X Y`, `up X Y`, `click X Y`, `scroll DX DY`, `text …`, `key NAME`, `wait MS` |
| `logs [OPTIONS] [ID]` | Show the instance's log file |

## `gearbox spawn`

Put things into the scene

| Leaf | Does |
|---|---|
| `usd [OPTIONS] <PATH>` | Load a USD as a prop (or another category) |
| `machine [OPTIONS] <PATH>` | Load a machine USD under a namespace and wait for its agent |
| `world <PATH>` | Load a world USD |
| `terrain <PATH>` | Load a terrain USD |
| `marker --at <M> <M>... <ID>` | Place a marker |
| `from <FILE>` | Apply a TOML manifest of spawns in order |
| `move [OPTIONS] <ID>` | Move a loaded object by re-issuing its load |

## `gearbox clear`

Take things out of the scene

| Leaf | Does |
|---|---|
| `all` | Everything: machines, props, markers (the UI button) |
| `machines` | Despawn every machine, keep props and markers |
| `props` | Props only |
| `markers` | Markers only |
| `usd <ID>` | One loaded USD by id |
| `marker <ID>` | One marker by id |
| `selection` | Drop the CLI selection and the UI highlight |

## `gearbox scene`

Clock, listing, poses, events

| Leaf | Does |
|---|---|
| `status` | Clock state, counts, world |
| `play` | Run the clock |
| `pause` | Pause the clock |
| `toggle` | Toggle the clock |
| `step [N]` | Advance N physics frames while paused |
| `speed <FACTOR>` | Time scale (not supported by the sim yet) |
| `list [OPTIONS]` | Objects in the scene |
| `pose [OPTIONS] <ID>` | Pose of one object; live for a machine |
| `events [OPTIONS]` | Tail scene events |
| `reset` | Clear, then reload the world and every recorded manifest |
| `tree` | One tf-style tree of the scene: machines with their links, props and markers as leaves |

## `gearbox select`

Choose the thing later commands act on

| Leaf | Does |
|---|---|
| `show` | Current selection |
| `machine <NS>` | Select a machine by namespace |
| `object <ID>` | Select a loaded object by id |
| `marker <ID>` | Select a marker by id |
| `next` | Select the next object in `scene list` order |
| `prev` | Select the previous object in `scene list` order |
| `none` | Clear the selection |

## `gearbox machine`

Inspect and drive machines

| Leaf | Does |
|---|---|
| `list` | Machines with their controllers and holders |
| `info [NS]` | Everything a machine reports about itself |
| `state [OPTIONS] [NS]` | Stream the machine's state |
| `move [OPTIONS] [NS]` | Drive with a constant twist for a while, then stop |
| `stop [OPTIONS] [NS]` | Send a zero twist and release |
| `drive [OPTIONS] [NS]` | Drive from the keyboard: WASD or arrows, space stops, q quits |
| `cmd [OPTIONS] <CONTROLLER> [KEY=VAL]...` | Generic controller command: KEY=VAL pairs, `value=` and `element=` are numeric |
| `controllers [NS]` | Controller table with types and interfaces |
| `links [OPTIONS] [NS]` | The link tree: names, roles, parents, static offsets |
| `tf [OPTIONS] [NS]` | Stream link poses: switches the machine's tf on, prints, switches it off |
| `sub [OPTIONS] <TOPIC>` | Stream any topic: a leaf under this machine (`imu`, `turn_radius`, `encoders`, `odom`, `tf`, `state`) or a full path starting with `/` |
| `set-value [OPTIONS] <LINK> <NAME> <VALUE>` | Set a named value on a link: LINK NAME VALUE (e.g. boom position 0.8) |
| `tools <COMMAND>` | Attachments: what hangs on a machine, attach and detach slaves |

## `gearbox access`

Identities, allowlists, sessions

| Leaf | Does |
|---|---|
| `whoami` | The CLI's did and key path |
| `list` | Allowed peers of the instance and who holds each machine |
| `grant [OPTIONS] <DID>` | Allow a peer on the next launch (live grants need agentio support) |
| `revoke <DID>` | Remove a peer from the launch allowlist |
| `take <NS>` | Steal a machine's session, then release it |
| `release <NS>` | Force-release whoever holds a machine |
| `identity <COMMAND>` | Manage CLI identities under agentio's key dir |

## `gearbox api`

Raw topic access, schemas, diagnostics

| Leaf | Does |
|---|---|
| `topics` | Topics of the target and its machines, with types |
| `call [OPTIONS] <TOPIC> [BODY]` | req/res call with a JSON body |
| `sub [OPTIONS] <TOPIC>` | Tail a pub/sub topic |
| `que [OPTIONS] <TOPIC> [BODY]` | que/ans query, one line per answer |
| `schema [TYPE_NAME]` | Schema of a wire type, or the list of all |
| `diag [DID]` | Transport path to the target (or a peer did) |
| `doctor` | Registry, key and version checks |

## `gearbox env`

CLI config, keys, completions, doctor

| Leaf | Does |
|---|---|
| `show` | Current config values |
| `set <KEY> [VALUE]` | Set a config key (instance, output, asset_root, sim); empty VALUE unsets |
| `path` | Paths of the config, context, registry and key files |
| `completions <SHELL>` | Shell completions |
| `keys` | Where identities live |
| `version` | CLI, sim and library versions |
| `docs` | The command tree as Markdown |

