# gearbox — state of play

Where we are, what we have, and what's still needed to finish the
split-deployment + browser-renderer story.

Scan the **Status** section near the bottom first if you're trying to
remember what to pick up next.

## The three layers

```
    ┌──────────────────────────────────────────────────────────┐
    │                 bin/gearbox (one binary)                 │
    │                                                          │
    │   ┌───────────┐                 ┌────────────────────┐    │
    │   │ Simulator │ ─► SceneState ─► │     Renderer       │    │
    │   │   Sim     │     (mirror)    │  Bevy + egui UI    │    │
    │   └─────▲─────┘                 └─────────▲──────────┘    │
    │         │                                 │              │
    │  ┌──────┴───────┐                 ┌───────┴────────┐     │
    │  │  Tool API    │                 │  Sim↔Renderer  │     │
    │  │  gearbox-api │                 │  gearbox-link  │     │
    │  │  (zenoh)     │                 │  (aeronet WT)  │     │
    │  └──────┬───────┘                 └───────┬────────┘     │
    └─────────┼─────────────────────────────────┼──────────────┘
              │                                 │
              ▼ external tools                  ▼ renderer clients
                (real robots, CLIs,               (native, or later
                 agents, other editors)            wasm in a browser)
```

| Layer | Role | "Server/Client" terminology? |
|---|---|---|
| **Simulator** | Owns `Sim` (rapier-f64 physics world) and steps it. **Rapier lives strictly here.** | no |
| **Renderer** | Draws the state. Reads from `SceneState` (a rapier-free mirror) — **never** touches `Sim` / rapier directly. | no |
| **Tool API** | Network surface for *external tools* | **yes** — zenoh server lives here |

Between Simulator and Renderer sits a renderer-facing state mirror,
[`SceneState`](crates/gearbox-viz/src/scene.rs). A
`mirror_scene_state_system` copies authoritative server data into it
every `PostUpdate`. The renderer reads from `SceneState`; the
split-deployment story (native sim host + wasm browser renderer) is
just "serialize `SceneState` onto the wire on one side, deserialize
back into `SceneState` on the other".

The word "server/client" belongs to the **Tool API axis only**. The
**Simulator ↔ Renderer axis** calls the two halves by their right
names — one side is always the simulator, the other always the
renderer.

## Crates

| Crate | Role | Key types | Bevy? |
|---|---|---|---|
| `gearbox-core` | Shared spec / data types (no physics, no Bevy) | `VehicleSpec`, `ControlInput`, `DriveMode`, `PartSpec` | no |
| `gearbox-physics` | Rapier-f64 wrapper + drive controllers | `Sim` | no |
| `gearbox-viz` | Bevy visualization layer; owns `GearboxSim` + `SceneState` | `GearboxSim`, `SceneState`, `ChaseCamera`, `SimClock`, `FollowTarget` | yes |
| `gearbox-editor` | Egui panels on top of the renderer | `Selection`, `HeadingArrows`, `EditorPlugin` | yes |
| `gearbox-api` | **Tool API** over zenoh | `ApiBroker`, `ClockWire`, `ClockCommand` | optional feature |
| `gearbox-link` | **Simulator ↔ Renderer** transport via aeronet / WebTransport | `SimToRenderer`, `RendererToSim`, plugins | yes |
| `bin/gearbox` | The one binary | — | yes |

Dependency direction is one-way: `core → physics → viz → editor → bin`.
`gearbox-api` and `gearbox-link` sit aside and are pulled by the bin.

## The two network boundaries

| | **Tool API** | **Sim ↔ Renderer link** |
|---|---|---|
| **Crate** | `gearbox-api` | `gearbox-link` |
| **Transport** | zenoh 1.x (pub/sub/query/reply) | aeronet 0.20 + `aeronet_webtransport` (QUIC) |
| **Wasm-capable?** | **no** — zenoh Rust doesn't target wasm | **yes** — WebTransport is wasm-first |
| **Encoding** | CBOR (`ciborium`) | CBOR (`ciborium`) |
| **Purpose** | Expose the sim to *other tools* | Split sim off a Bevy renderer process |
| **Who speaks it** | Server-side only (native). External robot teleop, CLIs, scripting agents | Both sides; either can be native or wasm |
| **Bevy integration** | Plugin wraps the broker | Plugin feeds Bevy `Message` events in/out |
| **Currently active?** | ✅ Running in `bin/gearbox` | 🟡 Crate built; not wired into `bin/gearbox` yet |

### Why two layers, not one?

- **zenoh** is the ecosystem standard for robot / scene comms. We want
  real robots and agents to treat the sim as just another zenoh peer.
  Zenoh Rust pulls `tokio`/`rustls`/QUIC and does not target
  `wasm32-unknown-unknown`.
- **aeronet / WebTransport** gives us the opposite: pure Rust, wasm
  works today, 60 Hz bidirectional, binary payloads. Useless for
  talking to a real robot on the network.

## Tool API (`gearbox-api`) — topic surface

Everything CBOR. Currently scene-agnostic; scene topics deferred
until OpenUSD lands (they'll be keyed by `sdf::Path`).

| Key | Direction | Payload |
|---|---|---|
| `gearbox/sim/clock` | pub | `ClockWire { paused: bool, speed: f32 }` |
| `gearbox/sim/clock/command` | sub | `ClockCommand::{ SetPaused(bool), SetSpeed(f32) }` |

```bash
# Watch the clock
z_sub -k 'gearbox/sim/**'
# Resume the sim from anywhere on the network
z_put -k 'gearbox/sim/clock/command' -v '{"SetPaused":false}'
# Double speed
z_put -k 'gearbox/sim/clock/command' -v '{"SetSpeed":2.0}'
```

### `ApiBroker`

- Pure Rust, no Bevy (feature-gated).
- Owns the zenoh session + cached publishers + subscribers. Drop it
  and the network surface goes.
- `publish_clock(&ClockWire)` — uses a cached `Publisher`.
- `drain_clock_commands()` — main-thread drain of the internal
  `Mutex<Vec<ClockCommand>>` the subscriber callback fills from a
  zenoh worker thread.

### Bevy plugin (`gearbox-api` with `bevy` feature)

- `GearboxApiPlugin` inserts `ApiSession` as a `Resource`.
- `apply_clock_commands_system` (`PostUpdate`) drains network
  commands and writes them to the `SimClock` resource.
- `publish_clock_system` (`PostUpdate`) reads `SimClock`, pushes
  `ClockWire` out.

## Sim ↔ Renderer link (`gearbox-link`) — topic surface

Also CBOR. Deliberately narrow — it's a boundary between two halves of
our own stack, not a public protocol.

```rust
enum SimToRenderer {
    Tick { time_s: f64, frame: u64 },
    ClockState { paused: bool, speed: f32 },
    // Future: scene snapshot variants keyed by whatever the
    // scene layer ends up addressing prims by (SdfPath).
}

enum RendererToSim {
    Ping,
    SetPaused(bool),
    Input { throttle, steer, brake, yaw, lift: f32 },
}
```

### `LinkServerPlugin` (feature `server`, native only)

- `Startup`: spawn entity, queue
  `WebTransportServer::open(config)` with a self-signed
  `wtransport::Identity` for `localhost` / `127.0.0.1` / `::1`;
  UDP port 7824 by default.
- Observer on `SessionRequest` auto-`Accepted` (dev posture).
- `PreUpdate.after(IoSystems::Poll)`: drains every client's
  `Session::recv`, CBOR-decodes into `IncomingInput` messages.
- `PostUpdate`: reads `OutgoingFrame` events, encodes once, pushes
  the shared `Bytes` into every connected session's `Session::send`.

### `LinkClientPlugin` (feature `client`, native or wasm)

- Native config: `ClientConfig::with_no_cert_validation()` — native
  dev trusts the self-signed server cert.
- Wasm config: placeholder `ClientConfig::default()` — browser trust
  will use `serverCertificateHashes` when the wasm build
  materialises.
- Symmetric `drain_incoming_packets_system` /
  `send_outgoing_inputs_system`.

### Feature matrix

| Build target | gearbox-link features | Why |
|---|---|---|
| Native simulator host | `server` | Needs wtransport server stack |
| Native renderer (dev) | `client` | Needs `with_no_cert_validation` |
| Wasm renderer | `client` (`xwt-web` picks up wasm target automatically) | Browser WebTransport |

## Terminology discipline

- **Server / Client** → exclusively about the **Tool API**. The
  simulator is a zenoh "server" (owns state); external things
  (robots, CLIs, agents) are "clients". `gearbox-api`.
- **Simulator / Renderer** → the two halves of gearbox itself,
  regardless of whether they share a process or speak over the
  link. `gearbox-viz` / `gearbox-editor` / `gearbox-link`.

If you find yourself calling the renderer "a client" or the sim "a
server", the Tool API axis has leaked into the wrong conversation.

---

# Status

## ✅ Where we are (working today)

- **One desktop binary** (`bin/gearbox`). Simulator, renderer, tool
  API — all in one process.
- **Simulator**: rapier-f64 physics world, fixed-dt 60 Hz, pause /
  speed control from the transport bar, monotonic `sim_frame` +
  `sim_time_s` counters on `SimClock`.
- **Renderer**: Bevy 0.18 + egui, chase camera, ground grid, sky +
  clouds, heading-arrow shader, selection ring, editor UI, WASD
  teleop on the selected vehicle.
- **Tool API**: zenoh session live; external clients can watch the
  sim clock and command pause / speed.
- **`SceneState` abstraction**: declared, registered, mirrored
  from `SimClock` every `PostUpdate`. Renderer has an explicit
  rapier-free reading surface.
- **`gearbox-link` crate**: aeronet WebTransport client + server
  plugins compile (both features). Wire types (`SimToRenderer` /
  `RendererToSim`) live. Session open, accept, CBOR
  encode/decode, Bevy `Message`-based in/out queues — all real
  code, no stubs.

## 🟡 Written but latent (compiled, not wired into `bin/gearbox`)

- `LinkServerPlugin` and `LinkClientPlugin` are *not* added to the
  binary's `App`. Adding them is a one-line change each; deferred
  until the split actually happens so dead plumbing doesn't bit-rot
  against the one-binary setup.

## ⏳ To split the simulator off into its own process

Everything here is pure plumbing — no renderer refactoring needed:

- **Wire `LinkServerPlugin`** into `bin/gearbox` behind a `--serve`
  flag (or whichever CLI-mode shape you prefer).
- **Scene snapshot wire variants**. Add variants to `SimToRenderer`
  that ship `SceneState` (today: clock; later: scene prims). CBOR
  encode the resource contents straight onto the wire.
- **Publisher system**: reads `SceneState`, emits
  `OutgoingFrame(SimToRenderer::Snapshot(..))` on change or at N Hz.
- **Consumer system on the client side**: reads `IncomingFrame`,
  writes into the client's local `SceneState`. Renderer unchanged.
- **`LinkClientPlugin`** wired into the same binary behind
  `--connect <url>` (or a compile-time "wasm renderer" target).

## ⏳ To run the renderer in a browser

All of the split-process work above, **plus**:

- **`std::fs` → browser storage.** Two files use `std::fs` directly:
  - `crates/gearbox-editor/src/persist.rs`
  - `crates/gearbox-viz/src/window_settings.rs`
  - Both need `#[cfg(target_family = "wasm")]` branches that use
    `web_sys::Storage` (localStorage).
- **Renderer decoupled from `Sim`.** The abstraction (`SceneState`)
  is already live, but ~79 renderer-side callsites (`sim.0.vehicle_pose`,
  `sim.0.vehicle_linvel`, `sim.0.vehicles()`, …) still read `Sim`
  directly. These need to migrate onto `SceneState`. **Intentionally
  deferred until OpenUSD replaces the vehicle model** — migrating
  to a vehicle-shaped `SceneState` now just means rewriting twice.
- **Cert-hash propagation.** Server generates the self-signed cert's
  SHA-256 at startup; wasm client puts it in
  `ClientConfig::serverCertificateHashes` or the browser refuses
  the connection. Options: static JSON endpoint on the sim host,
  or compile-time embed if the cert is fixed.
- **Static file server** in `bin/gearbox` to serve the wasm bundle.
  A dozen lines of `axum` / `hyper` behind the same `--serve` flag.
- **Canvas mount + winit wasm setup** in `bin/gearbox` main.
  Bevy 0.18 pattern — `#[wasm_bindgen(start)]`, canvas selector
  in `WindowPlugin`.
- **Wasm32 build verification.** `cargo check
  --target wasm32-unknown-unknown` has *not* been run end-to-end
  against this workspace yet (the current dev env can't materialise
  the wasm std). Expect surprise breakage from one or more
  transitive deps; plan for a short debug loop.

## ⏳ OpenUSD integration (separate track)

- Pull in `bevy_openusd` from the sibling project.
- Extend `SceneState` with `HashMap<SdfPath, Pose>` (or similar)
  for live prim xforms.
- Server-side: stage → `SceneState` populator (replaces the
  vehicle-from-`Sim` path).
- Retire vehicle-preset-based rendering; vehicles become USD prims
  authored by `bevy_openusd`.
- Scene topics on the **tool API** side:
  `gearbox/scene/prim/<path>/…` with `sdf::Path` as the address.

## Track dependency graph

```
  split-sim-process ─► browser-renderer
                              ▲
  openusd-integration ────────┘
```

The three tracks overlap at `SceneState`: split-process decides the
*wire shape* of it, OpenUSD decides the *data shape* of it, the
browser renderer consumes whatever it looks like on both axes.
There's no strict ordering, but split-process first means you can
test the link path without any browser setup yet, and OpenUSD first
means the renderer-migration happens only once.

---

## Why these choices

- **rapier3d-f64**: double-precision physics so planet-scale
  simulations don't lose precision far from origin. Propagates all
  the way through — spec types in `gearbox-core` are `f64`, sibling
  `datapod` crate ships f64 spatial types.
- **No `gilrs`**: gamepad / joystick input is a robot-API concern,
  not an engine concern. Anything external rides zenoh.
- **One binary**: `bin/gearbox` is the only binary. A future
  headless or split mode goes behind a flag on this same binary
  rather than a second crate.
- **Bevy 0.18 rename**: `Event`/`add_event` → `Message`/`add_message`.
  `EventReader`/`Writer` → `MessageReader`/`Writer`. Older Bevy
  docs need translating.
- **CBOR over JSON**: compact binary, `serde`-compatible,
  debuggable, pure-Rust decode with three deps (`serde`, `ciborium`,
  the wire types). JSON was fine for scaffold, CBOR is fine for the
  long run.

## Running / testing

```bash
# Desktop editor — simulator + renderer + tool API, one process
cargo run --bin gearbox

# Build the link crate on its own
cargo build -p gearbox-link --features server,client

# Pause the live sim from another shell
z_put -k 'gearbox/sim/clock/command' -v '{"SetPaused":true}'
```
