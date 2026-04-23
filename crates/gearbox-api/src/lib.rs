//! # Tool API — external integration surface (zenoh).
//!
//! This crate is the **tool API**: the network boundary gearbox
//! exposes to *external tools* — a real robot publishing telemetry,
//! a scripting agent issuing commands, a CLI pausing the sim, a
//! second editor mirroring state. It speaks zenoh because zenoh is
//! the lingua franca for robot / scene comms in the ecosystem we
//! plug into.
//!
//! ## What this crate is NOT
//!
//! This is **not** the simulator ↔ renderer link. The simulator and
//! renderer currently live in the same process (`bin/gearbox`) and
//! share a `Sim` via a Bevy resource — no transport needed. When we
//! later split them (headless sim server + wasm browser renderer),
//! the transport for *that* split will be a separate crate (likely
//! `aeronet` over WebSocket / WebTransport) because zenoh doesn't
//! target `wasm32-unknown-unknown` and because the sim↔renderer
//! traffic profile (60 Hz scene deltas) is very different from the
//! tool-API traffic profile (low-rate commands + status).
//!
//! Architecturally:
//!
//! ```text
//!     ┌──────────────────────────────────────────────────────┐
//!     │                 bin/gearbox (one binary)             │
//!     │                                                      │
//!     │  ┌──────────┐   in-process   ┌─────────────────┐     │
//!     │  │Simulator │ <────────────> │    Renderer     │     │
//!     │  │  (Sim)   │    (same       │  (Bevy + egui)  │     │
//!     │  └──────────┘    resource)   └─────────────────┘     │
//!     │        ▲                                             │
//!     │        │                                             │
//!     │  ┌─────┴────────┐   ← THIS CRATE                     │
//!     │  │ Tool API     │                                    │
//!     │  │ (zenoh pub/  │                                    │
//!     │  │  sub/query)  │                                    │
//!     │  └───────┬──────┘                                    │
//!     └──────────┼──────────────────────────────────────────-┘
//!                │
//!                ▼    external tools:
//!                     real robots, CLIs, scripting agents, other
//!                     editors — server/client terminology lives
//!                     here, not in the sim↔renderer split.
//! ```
//!
//! ## Module layout
//!
//! * [`broker`] — pure-Rust [`ApiBroker`] that owns the zenoh
//!   session. No Bevy.
//! * [`wire`] — CBOR-encoded message types.
//! * `plugin` — Bevy `GearboxApiPlugin` (feature-gated), wiring the
//!   broker into the editor's `SimClock` resource.

pub mod broker;
pub mod wire;

#[cfg(feature = "bevy")]
mod plugin;

pub use broker::ApiBroker;
pub use wire::{ClockCommand, ClockWire};

#[cfg(feature = "bevy")]
pub use plugin::{ApiSession, GearboxApiPlugin};
