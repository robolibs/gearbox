//! Gearbox tool API on agentio.
//!
//! One `HostBus` per simulator process hosts the scene topics; one
//! `MachineAgent` per simulated machine hosts that machine's control and
//! telemetry topics under its own identity. `Client` talks to both.
//! The `bevy` feature adds the plugins that wire the host into the app.

pub mod client;
pub mod fake;
pub mod host;
pub mod machine;
pub mod registry;
pub mod topics;
pub mod wire;

#[cfg(feature = "bevy")]
pub mod plugin;

pub use agentio;
pub use agentio::IdentitySource;
pub use client::{Client, MachineClient, next_sample};
pub use datapod;
pub use host::{HostBus, HostConfig};
pub use machine::{
    ControllerDesc, LinkDesc, MachineAgent, MachineConfig, ToolDesc, link_records_for,
};
pub use peerbus;
pub use wire::*;
pub mod tyres;

#[cfg(feature = "bevy")]
pub use plugin::{
    GearboxBus, GearboxBusPlugin, MachineDeleteQueue, MachineLoadQueue, PhysicsActive, SceneObjects,
    SelectionState, SimResetRequest, UsdAssetRoot, UsdLoaderPlugin, UsdMarkerPlugin,
};
