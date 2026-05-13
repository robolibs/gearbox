//! # Simulator ↔ Renderer link (aeronet / WebSocket).
//!
//! Transport for the case where the simulator and the renderer are
//! split across two processes — classic example: a headless native
//! sim on a server machine, a wasm Bevy renderer running in a
//! browser tab.
//!
//! When the two layers live in the *same* process (the default
//! `bin/gearbox` today), this crate is **unused**. The sim and the
//! renderer share a Bevy resource directly; there is no network
//! between them and no reason to introduce one.
//!
//! ## Not to be confused with…
//!
//! …the **tool API** (`gearbox_api`, zenoh). That crate is the
//! network boundary gearbox exposes to *external tools* — real
//! robots, CLIs, scripting agents. Zenoh cannot target wasm, so it
//! stays server-side and speaks only to native processes. This
//! crate is explicitly the *wasm-friendly* boundary.
//!
//! ## Shape
//!
//! * [`wire`]   — CBOR-encoded [`SimToRenderer`] / [`RendererToSim`]
//!                messages. The only thing both sides must agree on.
//! * [`server`] (feature `server`) — Bevy plugin that opens an
//!                aeronet WebSocket listener on the simulator host
//!                and bridges it to in-process sim state.
//! * [`client`] (feature `client`) — Bevy plugin that opens an
//!                aeronet WebSocket client on the renderer side and
//!                bridges it to the renderer's resources.
//!
//! The two features are mutually independent so you can build
//! server-only (native, linux/mac) or client-only (wasm) without
//! pulling the other side's dependencies.

pub mod wire;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "client")]
pub mod client;

pub use wire::{CodecError, RendererToSim, SimToRenderer, decode, encode};

#[cfg(feature = "server")]
pub use server::LinkServerPlugin;

#[cfg(feature = "client")]
pub use client::LinkClientPlugin;
