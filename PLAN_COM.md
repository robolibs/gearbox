# PLAN — gearbox communication on agentio

Replace zenoh with agentio over peerbus. Every entity that can be talked to
gets its own agentio `Agent`, so it has its own ed25519 identity, its own
`did:key`, and its own hosted topics: the simulator host, every simulated
machine, the CLI, and later tools. Things talk to each other only through
agentio, and anything agentio cannot do yet is either covered by a small piece
in `gearbox-api` or listed as an upstream ask, never re-implemented here.

This plan is the prerequisite of `PLAN_CLI.md`. Its Phases 0 and 1 are
superseded by this document; the CLI plan picks up at its Phase 2 once the
bus below exists.

## 1. What agentio is today

Audited on 2026-09-11 at agentio working tree (HEAD `bea4cba` plus
uncommitted work), peerbus `9a142c3`. Both build in the nix shell; agentio's
56 tests pass single-threaded, including forced-QUIC referral and directory
reconciliation.

### Real and usable now

| Capability | Where | Notes |
|---|---|---|
| One `Agent` = one key = one peerbus `Node` | `agent/core.rs`, `agent/builder.rs` | `secret_key` handed to peerbus; two `std` threads per agent plus one per directory seed. No tokio use despite the dependency. |
| Identity persistence | `identity/source.rs` | `Name`, `DidKey`, `File`, `Key`, `Random`, `Ephemeral`. Keys under `$AGENTIO_KEYS_DIR`, `$XDG_DATA_HOME/agentio/keys`, or `~/.local/share/agentio/keys`, mode 0600, rejected if group/other readable. `Ephemeral` is misnamed: it persists to `ephemeral.key`. Every persisting source also writes a second copy under `keys/did/`. |
| `did:key` text form | `identity/mod.rs` via authbox | Encode, decode, DID document JSON. |
| All five exchange families | `agent/core.rs` | `publish`, `subscribe`, `req`, `que`, `put`, `pip`, each server registered in the directory as a signed, leased record; dropping the `Registered` handle sends a signed withdrawal. |
| Signed directory | `directory/*` | Ed25519 over postcard; one live owner per `(topic, exchange)`; revisions, 30 s leases, renewal every lease/3, paginated snapshots with generation, typed conflicts. |
| Referral resolution | `__agentio_resolve` req/res | Typed clients resolve locally, then ask seeds (`Replicated`) or the front door (`FrontDoor(id)`), validate exchange and type hashes, then dial the owner directly. |
| `by_id` escape hatch | `escape/by_id.rs` | Client-side only. Bypasses the directory, not the inbound ACL. |
| Inbound ACL | builder only | Deny by default; `allow_peer`, `allow_peers`, `allow_any_peer`. |
| Health counters | `directory/health.rs` | Announcement, resolver, rejection, stale-seed, conflict counts. |

### Missing, and what gearbox does about it

| Gap | Consequence | Gearbox answer |
|---|---|---|
| **No host-local discovery.** Nothing finds an agent on the same machine without already knowing its id; peerbus has none either. | The CLI cannot "just find" the sim. | A registry file per running host under `$XDG_RUNTIME_DIR/gearbox/` (§4). Upstream ask U1. |
| **No runtime allowlist mutation.** ACL is fixed at `build()`. | Granting a new robot access means restarting the host. | Host allows the CLI's did at launch and takes `--allow`/`--allow-any`; `access grant` edits config for the next launch. Upstream ask U2. |
| **No wildcard or prefix topics.** Exact `(topic, exchange)` only. | `gearbox/usd/load/**` style keys are impossible. | Fixed topic names, ids inside payloads (§5). |
| **No transitive replication.** Records reach direct seeds only. | A three-hop chain does not converge. | Every agent in a gearbox deployment bootstraps to the host, so the host is one hop from everyone. `FrontDoor(host)` for the CLI. |
| **`resolve_topic` is local-only.** Typed clients do referral; the bare lookup does not. | Listing "what exists" needs the directory snapshot, not `resolve_topic`. | `api topics` uses `agent.directory()` after `reconcile_now()`. |
| **`hosted_topics()` from peerbus is never used.** Anything published through `agent.node()` is invisible to the directory. | Raw peerbus publishing bypasses naming. | Gearbox never touches `agent.node()` for hosting. `by_id` only for clients. |
| **No `publish_in`, no participant context.** Only `subscribe_in`. | Relative topic names must be qualified by hand on the host side. | `gearbox-api` builds absolute topics through one `topics.rs` module. Upstream ask U3. |
| **No Python or C bindings.** peerbus has both; agentio re-exports neither. | Scripts cannot use identity or the directory. | Scripts use peerbus Python by id; the CLI hands them dids (§7). Upstream ask U4, the most valuable one. |
| **Static bootstrap set.** Seeds learned after `build()` get no worker. | An agent cannot adopt a new seed at runtime. | Machine agents are built with the host as their only seed, which exists before they do. |
| **One bad answer aborts a query.** `query_directory` uses `?` per candidate. | A conflicting seed poisons resolution. | Single seed per agent avoids it; upstream ask U5. |

### Pins

| Crate | agentio wants | gearbox has | Action |
|---|---|---|---|
| peerbus | rev `f4b31c6` | none | add at the same rev; do not use head `9a142c3` alone, it bumps authbox to 0.1.1 and splits the graph |
| authbox | tag 0.1.0 (via that peerbus rev) | none | inherit |
| datapod | tag 0.4.1 | unpinned git, locked at 0.4.0, used by zero files | pin to 0.4.1 |
| agentio | — | none | git rev of the audited tree once committed; path dep for development only |

## 2. Entity model

Every box below is one agentio `Agent`, one key, one `did:key`.

```
            ┌────────────────────────────────────────────────────────┐
            │ gearbox-sim process                                    │
            │                                                        │
            │  host agent   "gearbox/<instance>"                     │
            │    /gearbox/info  /scene/*  /usd/*  /marker/*  /select │
            │    /machines/list                                      │
            │    seed for every agent below and for the CLI          │
            │                                                        │
            │  machine agent "gearbox/<instance>/<ns>"   one per     │
            │    /cmd_vel  /odom  /state  /drive  /cmd   machine     │
            │    /links  /info                                       │
            │                                                        │
            │  machine agent "gearbox/<instance>/<ns2>"              │
            └────────────────────────────────────────────────────────┘
                         ▲                     ▲
        CLI agent        │                     │   robot stack / script
        "gearbox-cli"────┘                     └── peerbus by id, or
        bootstrap = host, FrontDoor(host)          its own agentio agent
```

### Why one agent per machine

A simulated tractor with its own identity is indistinguishable, to a client,
from a real one running agentio. The topics are relative to the machine
(`/cmd_vel`, `/odom`) exactly as they would be on hardware, the did is stable
across runs because the key is persisted under the machine's namespace, and
an allowlist entry for "the tractor" means the same thing in the field and in
the sim. Ownership of a machine becomes ownership of a pip on that machine's
agent, not a session id inside a payload.

The cost per machine agent: two threads plus one seed worker, one iroh UDP
socket, one shared-memory arena, a few milliseconds at build. Ten machines is
fine. A hundred is not; §9 records the fallback.

### Identities

| Entity | `IdentitySource` | Stable across runs |
|---|---|---|
| host | `Name("gearbox/<instance>")`; `Random` with `--ephemeral` | yes, per `--name` |
| machine | `Name("gearbox/<instance>/<ns>")` | yes, per instance and namespace; `hunter_1` in `barn` always has the same did |
| CLI | `Name("gearbox-cli")` | yes |
| script | whatever peerbus Python is given; recommended a persisted key file | caller's choice |

`agentio` replaces `/` in names with `_` in the key filename, so
`gearbox/barn/hunter_1` is stored as `name/gearbox_barn_hunter_1.key`. Fine.

### Bootstrap and ACL

- Machine agents: `bootstrap([host])`, `DirectoryMode::FrontDoor(host)`,
  `allow_peers(host allow set)`. The host's allow set is the CLI did, every
  `--allow`, and `allow_any_peer` when `--allow-any`.
- Host: `Replicated` with no seeds. It is the authority; it needs nobody.
- CLI: `bootstrap([host])`, `FrontDoor(host)`, `allow_peer(host)` so the host
  can answer. Machine ids come back through referral.
- Whether peerbus applies the ACL on the shared-memory path is not stated in
  its docs; the audit found the check on the QUIC connection only. Phase 1
  verifies this with a test before anything relies on it.

## 3. Topics

Absolute, agentio-normalized, exact-match. Each row is one hosted exchange
with a signed directory record. `<ns>` topics are hosted by that machine's
agent, everything else by the host.

| Host | Mode | Req → Res / payload |
|---|---|---|
| `/gearbox/info` | req/res | `Empty` → `HostInfo` |
| `/gearbox/scene/clock` | req/res | `ClockCommand` → `ClockState` |
| `/gearbox/scene/clock/state` | pub/sub | `ClockState`, latest |
| `/gearbox/scene/clear` | req/res | `ClearRequest` → `Status` |
| `/gearbox/scene/list` | que/ans | `ListQuery` → `SceneObject` × N |
| `/gearbox/scene/events` | pub/sub | `SceneEvent`, reliable, history 8 |
| `/gearbox/usd/load` | req/res | `UsdLoad` → `Status` |
| `/gearbox/usd/delete` | req/res | `UsdRef` → `Status` |
| `/gearbox/marker/set`, `/gearbox/marker/delete` | req/res | `MarkerSet` / `MarkerRef` → `Status` |
| `/gearbox/select` | req/res | `Selection` → `Selection` |
| `/gearbox/machines/list` | que/ans | `Empty` → `MachineRef` × N, each with the machine's did |

| Machine | Mode | Payload |
|---|---|---|
| `/info` | req/res | `Empty` → `MachineInfo` (kind, controllers, interfaces, links) |
| `/drive` | pip | `Twist` in, `MachineState` out; open = own, close = release |
| `/cmd_vel` | put/ack | `Twist` × N → `Status`; for clients that cannot hold a pip, accepted only while no pip is open |
| `/odom` | pub/sub | `datapod::robot::Odom`, latest |
| `/state` | pub/sub | `MachineState`, latest |
| `/cmd` | req/res | `ControllerCommand` → `Status`; process-data style, see `TOOLS_SPEC.md` |
| `/session` | req/res | `Empty` → `SessionInfo` (holder did, age) |

The `/drive` pip carries the session rules from `PLAN_CLI.md` §2: one open
pip per machine, `Busy` with the holder's did otherwise, `take` flag for an
allowed steal, silence longer than `hold_ms` zeroes the command, close zeroes
and frees.

## 4. Host-local registry

Written by the host after `Agent::build`, removed on `AppExit`, pruned by
readers when the pid is dead:

```
$XDG_RUNTIME_DIR/gearbox/<instance>.json
{ "name": "barn", "did": "did:key:z6Mk…", "addr": "<EndpointAddr>",
  "pid": 41231, "started": "…", "version": "0.0.7", "log": "…/barn.log" }
```

Machines are not in the file. They are found through the host's directory.
This is the only discovery mechanism gearbox owns, and it goes away the day
agentio grows one (U1).

## 5. Wire types

`crates/gearbox-api/src/wire/*.rs`, datapod only, every type with a
canonical name so Python and C get the same hash:

```rust
#[datapod::datapod(name = "gearbox.usd_load.v1")]
pub struct UsdLoad {
    pub x: f32, pub y: f32, pub z: f32, pub yaw_deg: f32,
    pub category: u32,
    pub flags: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,          // datapod::Map: id, path, namespace, label, variants
}
```

Rules:

- fixed numeric fields in the header, one `Map` in the bytes field for
  strings and optional extras; never two heap fields;
- `Twist` and `Odom` from `datapod::robot`, not redefined;
- `Status { code, detail: Map }` is the universal response; codes match the
  CLI exit codes;
- every type name ends in `.v1`; a breaking change is `.v2` beside it.

CLI and sim compile the same crate, so peerbus's Rust type hash matches.
Cross-language clients use the canonical name through peerbus's
`datapod_*` APIs.

## 6. Inside the simulator

- `GearboxBus` resource: the host `Agent`, its registry entry, every
  `Registered` server handle (dropping one withdraws it), and a
  `HashMap<ns, MachineAgent>`.
- `MachineAgent`: the machine's `Agent`, its handles, the open pip token if
  any, and the `ControllerKey` it maps to. Built by
  `ControllerDiscoveryPlugin` when a machine is discovered, dropped on
  despawn or clear. Dropping withdraws every record and closes the pip.
- All servers are polled in `Update` with `take()` or `recv_timeout(0)`.
  No callbacks, no mutex inboxes, no blocking calls on the main thread.
- `LocalConfig` set once before the first host topic: `max_subscribers` 32,
  `history_depth` 4, `max_payload_bytes` 1 MiB. `TopicQos::latest()` for
  `/odom`, `/state`, `/clock/state`; reliable elsewhere.
- The Clear button and any future UI action go through the same
  `ClearRequest` handler as the wire, so UI and API cannot drift.

## 7. Scripts and external clients

Until U4 lands there is no agentio in Python. Scripts use peerbus's Python
module by id:

1. `gearbox machine list --json` prints each machine's did and addr.
2. The script opens a peerbus `Node`, dials that did, and uses
   `datapod_pip_client` on `/drive` or `datapod_publisher`/`subscriber` on
   `/odom` with the canonical type names.
3. `scripts/gearbox_client.py` wraps steps 1 and 2 so a script is
   `connect("oxbo").drive(v, w)`.

A robot stack that already runs agentio simply bootstraps to the host and
subscribes by name; nothing in gearbox is special-cased for it.

## 8. Phases

### Phase 0 — pins and removal

- Pin datapod 0.4.1, add peerbus at `f4b31c6`, add agentio, remove zenoh,
  ciborium, serde from `gearbox-api` and `bin/gearbox`. Remove `gearbox-link`
  and `gearbox-world` from the workspace.

Accept: `cargo tree -i datapod`, `-i authbox`, `-i peerbus` each show one
version; the workspace builds with zenoh gone, even if nothing works yet.

### Phase 1 — bus and types

- `gearbox-api`: wire types, `topics.rs`, `GearboxBus`, registry file.
- A `fake-host` test binary in `gearbox-api` hosting every host topic with
  canned answers, no Bevy.
- Test: SHM-path ACL behaviour, recorded in this file under §2 once known.

Accept: an integration test builds a client agent, resolves `/gearbox/info`
via `FrontDoor`, loads a USD against `fake-host`, and reads a `SceneEvent`.

### Phase 2 — host agent in the sim

- Port loader, marker, reset, world events, clock, select, scene list.
- Registry entry, startup banner with did and addr, `AppExit` cleanup.

Accept: `gearbox-sim` prints its did; the Phase 1 test passes against the
real sim; the Clear button works through `ClearRequest`.

### Phase 3 — machine agents

- `MachineAgent` lifecycle in `ControllerDiscoveryPlugin`.
- `/info`, `/drive` pip with session rules, `/cmd_vel`, `/odom`, `/state`,
  `/session`; `MachineInfo` built from the discovered `ControllerSpec`s.
- `/gearbox/machines/list` returns dids; referral from the CLI agent resolves
  a machine's `/drive` without the CLI knowing the machine did beforehand.

Accept: two agents in one test process drive `husky` and `oxbo` at once;
a second `/drive` open is refused with the holder's did; killing the client
process zeroes the command within `hold_ms`; despawn withdraws the records
and the directory forgets the machine within one lease.

### Phase 4 — scripts

- `scripts/gearbox_client.py` on peerbus Python; port every script;
  `xtra/` clients get the same wrapper.

Accept: `oxbo_follow_points.py` and `hunter_drive.py` behave as before;
`grep -r zenoh` over the repo is empty.

### Phase 5 — upstream asks, in priority order

Filed against agentio, each with a gearbox test that proves the need. Until
they land, gearbox keeps the workaround named in §1.

- **U4** Python bindings for `Agent` (identity, `FrontDoor` resolution,
  typed `datapod_*` clients). Unblocks scripts talking by name.
- **U1** Host-local rendezvous: enumerate agents on this host without their
  ids. Removes §4.
- **U2** Runtime `allow_peer` on a live agent, if peerbus can add it.
- **U3** `publish_in` and a per-agent participant prefix.
- **U5** `query_directory` continues past a bad candidate.
- **U6** Feed the directory from `peerbus::Node::hosted_topics()` so raw
  node use is not invisible.

## 9. Risks and fallbacks

- **Per-machine agent cost.** If a scene needs more than roughly twenty
  machines, fall back to hosting machine topics on the host agent under
  `/gearbox/machines/<ns>/...`. The wire types and the CLI do not change;
  only which agent hosts them and whether a machine has a did.
- **peerbus SHM limits.** `max_subscribers` and `history_depth` are pinned by
  the first creator of a segment. The host sets `LocalConfig` before hosting
  anything; machine agents inherit the same config.
- **Type hash coupling.** Rust type hashes include the type path and vary
  with toolchain. The CLI and sim ship together; anything else uses canonical
  names.
- **Two private-key copies on disk** per persisted identity is agentio's
  behaviour, not ours, but `gearbox env keys` should say so.
- **`Ephemeral` persists.** Gearbox uses `Random` for `--ephemeral` and
  `Name` otherwise, never `Ephemeral`.
- **agentio is unreleased and its tree is dirty.** Pin a commit the moment
  one is made; carry a path override in `.cargo/config.toml` for local work
  only, per agentio's own dependency decision.

## 10. Implementation notes (2026-09-11)

Phases 0 to 4 are implemented. Where the code differs from the plan above,
the code wins and the difference is recorded here.

- **Machine control is req/res, not pip.** peerbus's pip server blocks up to
  five seconds on every read, so it cannot be polled from Bevy's main
  thread. A machine is owned through `/machines/<ns>/claim`, driven with
  `/machines/<ns>/cmd_vel` requests carrying the session id, and released
  with `/machines/<ns>/release`. Silence longer than the claim's hold time
  zeroes the twist, a minute of silence releases. `/session` reports the
  holder. Telemetry stays pub/sub on `/machines/<ns>/state` and `/odom`.
- **Machine topics are `/machines/<ns>/...`, not `/cmd_vel`.** The host
  directory keys records by topic and exchange, so two machines hosting
  `/cmd_vel` would conflict. Each machine agent hosts its topics under its
  namespace and the CLI resolves them through the host as front door.
- **Every topic carries `peerbus::DatapodMsg`.** The typed request and
  response ride inside the envelope by canonical datapod name, so
  peerbus's Python binding, which only speaks envelopes, talks to the Rust
  servers unchanged. Rust callers use `pack` and `unpack` in `wire::common`.
- **The registry and machine refs carry the hex endpoint address** in the
  form peerbus's Python binding dials, next to the `did:key`.
- **agentio gained `AgentBuilder::local_config`.** peerbus pins a
  shared-memory ring's producer and subscriber limits at creation, and its
  default of two producers per service refused the third local client.
  Gearbox sets 8 producers, 16 subscribers, 64 KiB payloads, history 1.
  Larger histories let que/ans clients read a previous query's answer.
- **peerbus gained per-handle request-id seeds** (`local::seed_id`). Clients
  numbering requests from 1 on a shared ring picked up each other's retained
  responses. The workspace patches `peerbus` to the local checkout for this;
  agentio's authbox pin moved to 0.1.1 to match.
- **The allowlist applies to QUIC only.** peerbus checks it in the accept
  path; shared-memory peers are never filtered. Machine agents therefore
  announce to the host without pre-registered keys, and only remote peers
  need `GEARBOX_ALLOW` or `GEARBOX_ALLOW_ANY`.
- **Fake host.** `gearbox_api::fake::FakeHost` and the `fake_host` example
  host every topic without Bevy, create a kinematic machine agent for each
  machine load, report bale poses at once, and harvest bales a machine
  drives within three metres of. The Rust tests and the Python scripts run
  against it.
- **Python.** `scripts/gearbox_client.py` on peerbus's Python module and
  datapod's declarative schemas; every script under `scripts/` and the
  MultiBaleCollection experiment use it. Wheels are built from the sibling
  checkouts into `.python-packages`. No agentio in Python yet (U4): scripts
  read the registry file for the host and the machine list for machines.
- **Not verified in this session:** a windowed run of the simulator. The
  binary builds and links; the sandbox had no render node to present to.
