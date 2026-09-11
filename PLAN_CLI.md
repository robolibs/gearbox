# PLAN — gearbox CLI on peerbus

Turn `gearbox` into a command-line front door for the simulator: launch a sim,
get an identity for it, and talk to it from any shell or script. Ten command
groups, each a tree underneath. Transport moves from zenoh to peerbus, wrapped
by agentio for identity, naming, and directory resolution.

Companion documents: `PLAN_COM.md` (the agentio bus this CLI talks to;
its Phases 0 to 3 replace Phases 0 and 1 below), `specs/CONTROLLER_SPEC.md`
(what a machine is), `specs/TOOLS_SPEC.md` (attachments),
`../peerbus/README.md`, `../agentio/README.md`, `../DEPS.md` (robolibs
version pinning).

`PLAN_COM.md` also changes one thing in this plan: every simulated machine
is its own agent with its own `did:key`, so per-machine topics are relative
to that agent (`/drive`, `/odom`) rather than `/gearbox/machines/<ns>/...`
on the host. The command tree in §3 is unaffected.

## 0. Decisions

These were settled while reading peerbus and agentio. Everything below builds
on them.

| Decision | Why |
|---|---|
| **agentio `Agent`, not raw peerbus `Node`.** | peerbus only takes a key and an id. agentio adds persistent key files, `did:key` text ids, a name table, a signed topic directory, and `by_id`. All of that is what a CLI needs and none of it belongs in gearbox. |
| **One `Agent` per process.** | Today the sim opens six zenoh sessions. One agent hosts every topic and appears as one id. |
| **The sim's identity is the "UUID".** | An ed25519 `EndpointId`, printed as `did:key:z6Mk…`. Persistent by default (`IdentitySource::Name("gearbox")`) so scripts can pin it. `--ephemeral` for throwaway runs. |
| **Exact-match topics, ids in payloads.** | peerbus has no wildcards. `gearbox/usd/load/<id>` becomes `/gearbox/usd/load` with `id` in the request. Per-machine topics are created when the machine is discovered. |
| **Requests are req/res, streams are pub/sub, ownership is pip.** | Commands must not be fire-and-forget over a transport that drops when nobody listens. A req/res returns a status. A machine's control session is a pip: whoever holds the pip open owns the machine, closing it releases. |
| **Payloads are `datapod` types in `gearbox-api`.** | peerbus carries `DataPod` only, no serde. Fixed fields ride the header, one `#[dp(bytes)]` field carries variable data. For open-ended fields the bytes field is a `datapod::Map` (string-keyed). |
| **Canonical datapod names on every wire type.** | `#[datapod(name = "gearbox.usd_load.v1")]` gives a toolchain-independent hash so Python scripts and C clients speak the same types through peerbus's `datapod_*` APIs. |
| **CLI and sim are two binaries in one workspace.** | The CLI must start in milliseconds and build without Bevy. `gearbox` is the CLI, `gearbox-sim` is the simulator, `gearbox run` launches the latter. |
| **A host-local instance registry.** | agentio resolves topics, not "which sims are running on this box". A JSON file per instance under `$XDG_RUNTIME_DIR/gearbox/` gives `gearbox instance list` without any network. |
| **The CLI's own key is auto-allowed.** | peerbus denies inbound peers by default. The sim reads the CLI's `did` from the CLI config on launch and allows it, so the local loop works with zero flags. Remote robots are added with `--allow` or `access grant`. |

## 1. Identity and discovery

### Sim side

```
gearbox run --name barn --load assets/world/flatland.usd
gearbox-sim: instance barn
  did      did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK
  addr     <EndpointAddr, direct addresses + relay>
  registry /run/user/1000/gearbox/barn.json
  allowed  did:key:z6Mk…cli (gearbox-cli), any: no
```

- Identity: `IdentitySource::Name(<name>)` under agentio's key dir, so the
  same `--name` gives the same `did` every launch. `--ephemeral` uses
  `IdentitySource::Random`.
- Registry entry, written after `Agent::build`, removed on clean exit:

```json
{ "name": "barn", "did": "did:key:z6Mk…", "addr": "…", "pid": 41231,
  "started": "2026-09-11T14:02:11Z", "version": "0.0.7", "world": "flatland.usd" }
```

- The CLI treats an entry whose `pid` is dead as stale and deletes it.
- ACL: `--allow <did>...`, `--allow-any` (WARNs), plus the CLI did from
  `$XDG_CONFIG_HOME/gearbox/cli.did` always. Robots on the LAN use
  `--allow-any` or are granted explicitly.
- `--no-relay` default on; `--relay` opts into iroh relays for off-LAN use.

### CLI side

- Identity `IdentitySource::Name("gearbox-cli")`, created on first run,
  `did` written to `cli.did` for the sim to read.
- Target instance resolution, in order: `--instance <name|did>`,
  `GEARBOX_INSTANCE`, the `instance` field in the context file, the only live
  registry entry, otherwise exit code 3 with the list of candidates.
- `bootstrap([sim did])` on the agent, then `by_id(sim)` for every call, so
  no directory round trip is needed for the common case. `agent.resolve_topic`
  is used only by `api topics` and for machines hosted by another agent later.
- Context file `$XDG_STATE_HOME/gearbox/context.toml`:

```toml
instance = "barn"
[selection]
kind = "machine"
id = "oxbo"
```

## 2. Topic layout

All absolute, agentio-normalized. `<ns>` is a machine namespace exactly as in
`CONTROLLER_SPEC.md`. Types are named by their canonical datapod name.

| Topic | Mode | Request → Response | Replaces |
|---|---|---|---|
| `/gearbox/info` | req/res | `Empty` → `InstanceInfo` | — |
| `/gearbox/scene/clock` | req/res | `ClockCommand` → `ClockState` | `sim/clock`, never wired |
| `/gearbox/scene/clock/state` | pub/sub | `ClockState` | — |
| `/gearbox/scene/clear` | req/res | `ClearRequest` → `Status` | `sim/clear`, `sim/reset` |
| `/gearbox/scene/list` | que/ans | `ListQuery` → `SceneObject` × N | — |
| `/gearbox/scene/events` | pub/sub | `SceneEvent` (loaded, pose, harvested, removed, attached) | `usd/loaded`, `usd/pose/*`, `usd/harvested/*` |
| `/gearbox/usd/load` | req/res | `UsdLoad` → `Status` | `usd/load/<id>` |
| `/gearbox/usd/delete` | req/res | `UsdRef` → `Status` | `usd/delete/<id>` |
| `/gearbox/marker/set` | req/res | `MarkerSet` → `Status` | `usd/mark/<uuid>/x/y/z` |
| `/gearbox/marker/delete` | req/res | `MarkerRef` → `Status` | `usd/mark/<uuid>/delete` |
| `/gearbox/select` | req/res | `Selection` → `Selection` | — |
| `/gearbox/machines/list` | que/ans | `Empty` → `MachineInfo` × N | — |
| `/gearbox/machines/<ns>/drive` | pip | `Twist` ↔ `MachineState` | `session` + `cmd_vel` |
| `/gearbox/machines/<ns>/state` | pub/sub | `MachineState` | `machines/<ns>/state` |
| `/gearbox/machines/<ns>/cmd` | req/res | `ControllerCommand` → `Status` | — (process data, tools) |
| `/gearbox/machines/<ns>/session` | req/res | `SessionQuery` → `SessionInfo` | — |
| `/gearbox/access/list` | req/res | `Empty` → `AccessInfo` | — |

Session semantics on the pip:

- Opening `/drive` succeeds only if no other pip is open on that machine.
  Otherwise `Busy` with the holder's did, unless the request header carries
  `take = 1` and the caller is allowed to steal (`access take`).
- Every client message is a `Twist`; the server streams `MachineState` back
  at the controller rate. Silence for longer than `hold_ms` (default 500)
  zeroes the command. Closing the pip zeroes the command and frees the
  machine.
- `/state` is read-only fan-out for watchers that do not want ownership.

### Wire types

Defined once in `crates/gearbox-api/src/wire/*.rs`, `#[datapod(name = …)]`,
no serde. Fixed fields first, one `Map` for the open-ended rest.

```rust
#[datapod::datapod(name = "gearbox.usd_load.v1")]
pub struct UsdLoad {
    pub x: f32, pub y: f32, pub z: f32, pub yaw_deg: f32,
    pub category: u32,          // Category enum: machine, static, variant, world, terrain
    pub flags: u32,             // bit 0 remove, bit 1 teleport …
    #[dp(bytes)]
    pub props: Vec<u8>,         // datapod::Map: id, path, namespace, label, variants
}

#[datapod::datapod(name = "gearbox.status.v1")]
pub struct Status {
    pub code: u32,              // 0 ok, 1 error, 4 refused, 5 unsupported, 6 busy
    #[dp(bytes)]
    pub detail: Vec<u8>,        // datapod::Map: message, id, holder …
}
```

`Twist` and `Odom` come from `datapod::robot`. `MachineState` wraps `Odom`
plus roll, pitch, and a `Map` of controller states. `MachineInfo` carries the
namespace, kind, machine id, and a `Map` listing controllers as
`controller.<n>.type`, `controller.<n>.command_interface`,
`controller.<n>.state_interfaces` so the CLI can answer "does it implement
`cmd_vel`" without a second round trip.

## 3. Command tree

Ten top-level groups. New functionality goes into a subgroup, never a new
top-level. Every leaf accepts `--json`; every group accepts `--instance`.

```
gearbox
├── run        launch a simulator instance
├── instance   find, inspect, stop instances; pick the default
├── spawn      put things into the scene
├── clear      take things out of the scene
├── scene      clock, listing, poses, events
├── select     choose the thing later commands act on
├── machine    inspect and drive machines
├── access     identities, allowlists, sessions
├── api        raw topic access, schemas, diagnostics
└── env        CLI config, keys, completions, doctor
```

### `run`

```
gearbox run [--name N] [--ephemeral] [--allow DID]... [--allow-any] [--relay]
            [--world USD] [--load USD]... [--detach] [--sim PATH] [-- sim-args]
```

Finds `gearbox-sim` next to itself, or `--sim`, or `GEARBOX_SIM`. Foreground
by default with the sim's log on stderr; `--detach` daemonizes and prints the
registry entry as JSON. `--world` loads with category `world`, `--load` with
category `machine`. Exits with the sim's code.

### `instance`

| Leaf | Does |
|---|---|
| `list` | registry entries with liveness, marks the default |
| `info [ID]` | `/gearbox/info`: did, addr, version, uptime, clock, machine count, world |
| `use ID` | writes `instance` into the context file |
| `stop [ID] [--force]` | `ClockCommand::Shutdown`, then SIGTERM after 3 s, SIGKILL with `--force` |
| `ping [ID]` | round-trip time of `/gearbox/info` |
| `wait [ID] [--timeout]` | blocks until the instance answers, for scripts |
| `logs [ID] [-f]` | tails the sim's log file from the registry entry |

### `spawn`

| Leaf | Does |
|---|---|
| `usd PATH [--id] [--at X Y Z] [--yaw DEG] [--category C] [--variant PRIM SET OPT]...` | `/gearbox/usd/load`. Category defaults from discovery: machine metadata present → `machine`, else `static`. |
| `machine PATH --ns NS [--at] [--yaw]` | same with `category = machine`, namespace required, waits for the machine to appear in `machines/list` unless `--no-wait` |
| `world PATH` | `category = world` |
| `terrain PATH` | `category = terrain` |
| `marker ID --at X Y Z` | `/gearbox/marker/set` |
| `from FILE` | a TOML manifest of the above, applied in order, for repeatable scenes |
| `move ID --at X Y Z [--yaw]` | re-issues the load with the same path, which the sim treats as a move |

Paths resolve as the sim does: absolute, cwd-relative, then the sim's asset
root. `spawn` prints the resolved id.

### `clear`

| Leaf | Does |
|---|---|
| `all` | `ClearRequest { scope: all }` — same as the UI button |
| `machines` | despawn every machine, keep props and markers |
| `markers` | markers only |
| `usd ID` | `/gearbox/usd/delete` |
| `selection` | drops the CLI selection and the UI highlight |

`gearbox clear` with no leaf is `clear all` after a confirmation, `-y` skips it.

### `scene`

| Leaf | Does |
|---|---|
| `status` | clock state, counts, world |
| `play` / `pause` / `toggle` | `ClockCommand` |
| `step [N]` | advance N physics steps while paused |
| `speed X` | time scale |
| `list [--kind machine\|prop\|marker\|terrain]` | `/gearbox/scene/list` |
| `pose ID [--watch]` | settled pose from events, or live if a machine |
| `events [--kind]` | tail `/gearbox/scene/events` |
| `reset` | clear plus reload the world and every `spawn from` manifest recorded in context |

### `select`

Selection is CLI context plus a UI highlight, so a script and a person at the
viewer see the same thing.

| Leaf | Does |
|---|---|
| `show` | current selection |
| `machine NS` / `object ID` / `marker ID` | sets both |
| `next` / `prev` | cycles through `scene list` in order |
| `none` | clears both |

Every `machine` leaf that takes `[NS]` falls back to the selected machine.

### `machine`

| Leaf | Does |
|---|---|
| `list` | `/gearbox/machines/list` as a table: ns, kind, controllers, held-by |
| `info [NS]` | full `MachineInfo` including link tree and capabilities |
| `state [NS] [--watch] [--rate HZ]` | subscribe `/state` |
| `move [NS] --forward M/S [--turn RAD/S] [--for DURATION] [--hold]` | opens the pip, streams the twist at 20 Hz for the duration, sends zero, closes. `--hold` keeps streaming until Ctrl-C. Exit 5 if no controller has `commandInterface = cmd_vel`. |
| `stop [NS]` | opens the pip (or steals with `--take`), sends zero, closes |
| `drive [NS]` | interactive: WASD or arrow keys on the terminal, live state line |
| `cmd [NS] CONTROLLER KEY=VAL...` | `/cmd` generic controller command; `element=4 ddi=SetpointWorkState value=1` for tools |
| `controllers [NS]` | the controller table with types and interfaces |
| `links [NS]` | the link tree from `CONTROLLER_SPEC.md` §7 once implemented |
| `tools ...` | reserved for `TOOLS_SPEC.md`: `attach`, `detach`, `list` |

"If it implements" is answered from `MachineInfo`: `move` refuses with a
message listing the interfaces the machine does expose.

### `access`

| Leaf | Does |
|---|---|
| `whoami` | the CLI's did and key path |
| `list` | `/gearbox/access/list`: allowed peers, `allow_any`, active sessions with holder and age |
| `grant DID [--persist]` | adds to the sim's allowlist file for next launch; live if agentio grows a runtime allowlist API, reported otherwise |
| `revoke DID` | the inverse |
| `take NS` | steal the drive session of a machine, then release |
| `release NS` | force-close the session held by anyone, for a stuck script |
| `identity new NAME` / `identity list` | manage CLI identities under agentio's key dir |

### `api`

The escape hatch and the introspection surface.

| Leaf | Does |
|---|---|
| `topics` | `agent.directory()` of the target: topic, mode, type name, type hash |
| `call TOPIC [JSON]` | req/res with a JSON body mapped onto the wire type by canonical name |
| `sub TOPIC` | pub/sub tail, decoded through `datapod::dynamic::view_message` |
| `que TOPIC [JSON]` | que/ans, one line per answer |
| `schema [TYPE]` | the datapod schema of a wire type, from `datapod::schema` |
| `diag [DID]` | `peer_path_diagnostics`: direct or relay, RTT, MTU |
| `doctor` | registry consistency, key permissions, stale SHM segments, version match |

### `env`

| Leaf | Does |
|---|---|
| `show` / `set KEY VAL` / `path` | `$XDG_CONFIG_HOME/gearbox/config.toml`: default instance, output format, asset root, sim path |
| `completions SHELL` | clap-generated |
| `keys` | where identities live |
| `version` | CLI, sim if reachable, peerbus, agentio, datapod |

### Extensions

`gearbox <name> …` where `<name>` is not a core group runs `gearbox-<name>`
from `PATH` with `GEARBOX_INSTANCE`, `GEARBOX_DID`, `GEARBOX_ADDR`, and
`GEARBOX_SELECTION` in its environment, git-style. Extensions are listed under
their own heading in `gearbox --help` only when found on `PATH`. They never
become core groups.

## 4. Output and errors

- Human output by default: tables for lists, key/value blocks for `info`,
  one line for actions. `--json` on any leaf prints the wire response as JSON
  through the datapod dynamic view. `--quiet` prints ids only.
- Exit codes: 0 ok, 1 error, 2 usage, 3 no instance, 4 refused by ACL or
  session, 5 machine does not implement, 6 busy, 7 timeout.
- Every error names the topic and the did it was talking to.
- `--timeout` defaults to 5 s for req/res, none for streams.

## 5. Crate and binary layout

```
crates/gearbox-api        wire types (datapod), topic constants, GearboxBus
                          (Agent wrapper), Bevy plugins behind `bevy` feature
crates/gearbox-cli        the `gearbox` binary: clap tree, context, registry,
                          client helpers. Depends on gearbox-api without `bevy`.
bin/gearbox               package `gearbox-sim`, binary `gearbox-sim`
```

- `gearbox-api` drops zenoh, ciborium, serde. Adds peerbus, agentio, datapod.
- `gearbox-cli/src/cli/<group>.rs`, one module per top-level group, each
  exposing `Args` and `run(ctx, args)`. `main.rs` only dispatches.
- `gearbox-cli/src/client.rs`: `Client::connect(target) -> Client` holding the
  agent and `by_id` handle; typed helpers `info()`, `load()`, `drive()`.
- Makefile: `run` becomes `cargo run -p gearbox-cli -- run $(RUN_ARGS)`;
  `build-release-bin` builds both; the release workflow packs both binaries.
- `gearbox-link` and `gearbox-world` are removed from the workspace in the
  same change; nothing depends on them.

## 6. Sim-side changes

- One `GearboxBus` resource built at startup from `run` flags passed through
  the environment (`GEARBOX_NAME`, `GEARBOX_ALLOW`, `GEARBOX_ALLOW_ANY`,
  `GEARBOX_EPHEMERAL`, `GEARBOX_RELAY`).
- Every former zenoh callback becomes a polling system: `server.take()` in
  `Update`, respond in place. No mutex inboxes.
- `ControllerDiscoveryPlugin` creates `/drive`, `/state`, `/cmd`, `/session`
  for each discovered machine and drops them on despawn or clear.
- `ClockCommand` is finally wired: pause, play, step, speed, shutdown.
- `/gearbox/scene/list` and `/gearbox/machines/list` are new que/ans servers
  over `LoadedAsset` roots and `ControllerInventory`.
- `/gearbox/select` writes the UI `Selection` and reads it back.
- Registry file written after bind, removed in an `AppExit` observer.
- `LocalConfig` for the bus: `max_subscribers` raised for `/state` fan-out,
  `history_depth` 4 on state topics, `TopicQos::latest()` on state and
  events, reliable on everything else.

## 7. Scripts

`scripts/*.py` all use zenoh and cbor2. They move to peerbus's Python module
plus datapod's Python bindings, through the `datapod_*` client APIs and
canonical type names. A shared `scripts/gearbox_client.py` wraps connect,
load, drive, and state so each script shrinks to its logic. The xtra ROS
workspace and MultiBaleCollection use their own `gearbox_api.py`; those get
the same wrapper.

## 8. Phases

Each phase ends with something runnable. Acceptance criteria are what a
reviewer checks, not what the author hopes.

### Phase 0 — pins and skeleton

- Pin `datapod` to the tag agentio and peerbus use; `DEPS.md` says gearbox is
  unpinned today and that is what breaks the build first.
- Add `peerbus` and `agentio` as git deps in the workspace manifest.
- Rename `bin/gearbox`'s binary to `gearbox-sim`; add `crates/gearbox-cli`
  with a `gearbox` binary that prints `--help` and `env version`.
- Makefile and release workflow updated for two binaries.

Accept: `make build` builds both; `cargo tree -i datapod` shows one version;
`gearbox --help` lists the ten groups.

### Phase 1 — bus in the sim

- `gearbox-api`: wire types with canonical names, topic constants,
  `GearboxBus`. Remove zenoh.
- Port loader, marker, reset, world events, and controller API to polling
  systems on the bus. Port the Clear button and the clock.
- Registry file and startup banner.
- A `fake-sim` test fixture in `gearbox-api` (no Bevy) that hosts the same
  topics with canned answers, for CLI tests.

Accept: `gearbox-sim` starts, prints did and registry path; a Rust test
opens an agent, calls `/gearbox/info`, loads a USD, receives the `loaded`
event; no zenoh symbol remains in the workspace.

### Phase 2 — `run`, `instance`, `api`, `env`

- Context file, target resolution, `Client::connect`.
- `run` with launch, banner passthrough, `--detach`.
- `instance list/info/use/stop/ping/wait/logs`.
- `api topics/call/sub/que/schema/diag/doctor`, `env *`, completions.

Accept: `gearbox run --name t --detach && gearbox instance use t && gearbox
instance info` works from a fresh shell; `gearbox api call /gearbox/info`
prints JSON; killing the sim makes `instance list` mark it dead and clean up.

### Phase 3 — `spawn`, `clear`, `scene`

- All leaves in §3 for the three groups, `spawn from` manifests.
- `scene events` tail with kind filter.

Accept: the `oxbo_flatland.py` flow reproduced as
`gearbox spawn world … && gearbox spawn machine … --ns oxbo`, then
`gearbox scene list` shows both, `gearbox clear all` empties it, all against
the real sim and against `fake-sim` in CI.

### Phase 4 — `machine`, `select`, `access`

- `/drive` pip server in the sim with the session rules of §2.
- `machine list/info/state/move/stop/drive/cmd/controllers`.
- `select` with UI highlight sync.
- `access whoami/list/take/release/grant/revoke/identity`.

Accept: `gearbox select machine oxbo && gearbox machine move --forward 1
--for 3s` drives the harvester and exits 0; the same against `husky` with
`--turn` turns on the spot; against a static prop it exits 5 naming the
missing interface; a second shell's `move` exits 6 until the first releases;
`access take` then succeeds.

### Phase 5 — scripts and docs

- `scripts/gearbox_client.py` on peerbus Python; port every script.
- `scripts/README.md`, `specs/CONTROLLER_SPEC.md` §6 rewritten for the new
  topics, `CLI.md` generated from clap with `--markdown-help`.

Accept: `oxbo_follow_points.py` and `hunter_drive.py` run unchanged in
behaviour; `grep -r zenoh` over the repo is empty.

### Phase 6 — later

- `machine tools attach/detach` per `TOOLS_SPEC.md`.
- `machine links` and a `tf`-style stream once the link tree exists.
- `gearbox shell`: a REPL that keeps one agent open, for interactive work.
- `run --headless` once the sim can run without a window.
- Runtime allowlist changes if agentio exposes them.

## 9. Risks

- **Type hashes include the Rust type path.** CLI and sim must be built from
  the same `gearbox-api`; a mismatched pair fails the handshake loudly. Fine in
  one workspace, a footgun for a separately built CLI. The canonical datapod
  name is what cross-language clients use; document both.
- **SHM defaults.** `max_subscribers = 8` and `history_depth = 1` are pinned
  by the first creator. The sim must set `LocalConfig` before hosting any
  topic, and the CLI must not create SHM services of its own for sim topics.
- **Blocking calls in Bevy.** `take()` is non-blocking; `call()` on a client
  blocks. The sim never acts as a client. The CLI never runs a Bevy app.
- **iroh startup.** Binding an endpoint takes tens of milliseconds and may
  wait for direct addresses. The CLI uses `no_relay` and does not call
  `wait_for_direct_addresses` for local targets.
- **agentio is pre-1.0 and its own plan is mid-flight.** Keep every agentio
  call behind `GearboxBus` and `Client` so an API change touches two files.
- **Python.** peerbus's Python module exists; datapod's Python schema path
  is what makes canonical names work. Verify both build in the nix shell
  before Phase 5 starts.

## 10. Implementation notes (2026-09-11)

Phases 0 to 5 are implemented in `crates/gearbox-cli` (binary `gearbox`)
and `bin/gearbox` (package and binary `gearbox-sim`, library still named
`gearbox` for the C and Python bindings). Where the code differs from the
plan above, the code wins and the difference is recorded here.

- **Machine control is claim / cmd_vel / release**, as `PLAN_COM.md` §10
  records; there is no `/drive` pip. `machine move`, `stop`, `drive` and
  `cmd` claim first, stream at 20 Hz, send zero, release. `--take` steals.
- **Machine topics live under `/machines/<ns>/…`** on the machine's own
  agent and are resolved through the host as front door.
- **`--json` goes through `gearbox_api::wire::json`**, a typed JSON view of
  every wire type keyed by canonical name; datapod's dynamic view knows only
  its own types. `api call` and `api que` build requests from JSON the same
  way; keys that are not fixed fields become string props.
- **`api topics` lists the known topic table**, resolving owners through the
  directory, rather than dumping the directory; a front-door client holds no
  replica to dump.
- **`scene step N`** is a new `clock_op::STEP` that runs N frames and pauses.
  `scene speed` exits 5: the sim has no time scale.
- **`access list`** reads allowed peers and `allow_any` from `/gearbox/info`
  props and each machine's `/session`; no `/gearbox/access/list` topic.
  `grant` and `revoke` edit `$XDG_CONFIG_HOME/gearbox/allow`, which `run`
  passes through `GEARBOX_ALLOW`; agentio has no runtime allowlist yet.
- **`run --detach`** redirects the sim to `$XDG_STATE_HOME/gearbox/logs/<name>.log`,
  passes it as `GEARBOX_LOG` so the registry entry carries it, and `setsid`s
  the child. `--world` and `--load` are applied over the bus once the sim
  registers.
- **Context is TOML** at `$XDG_STATE_HOME/gearbox/context.toml` with the
  instance, the selection, the last world and every `spawn from` manifest,
  which `scene reset` replays. Config is `$XDG_CONFIG_HOME/gearbox/config.toml`.
- **`spawn usd` defaults to `static`**; discovery-based category defaults
  would need the CLI to parse USD, so `spawn machine` is the explicit form.
- **`machine links` and `machine tools` exit 5** until the link tree and
  attachments exist.
- **`make run`** builds both binaries and launches the sim through the CLI;
  `make sim` runs `gearbox-sim` directly; `make cli RUN_ARGS='…'` runs the CLI.
- **Tests** run the built `gearbox` binary against an in-process fake host
  with every file it touches redirected into a temp directory
  (`crates/gearbox-cli/tests/cli.rs`). `gearbox run --sim target/debug/examples/fake_host`
  exercises the launch path without a GPU.
- **`CLI.md`** is generated by `gearbox env docs`.
