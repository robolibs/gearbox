//! Gearbox tool API on agentio.
//!
//! One `HostBus` per simulator process hosts the scene topics; one
//! `MachineAgent` per simulated machine hosts that machine's control and
//! telemetry topics under its own identity. `Client` talks to both.
//! The `bevy` feature adds the plugins that wire the host into the app.

pub mod client;
pub mod draw;
pub mod draw_data;
pub mod fake;
pub mod host;
pub mod machine;
pub mod registry;
pub mod topics;
pub mod view;
pub mod wire;

#[cfg(feature = "bevy")]
pub mod plugin;

pub use agentio;
pub use agentio::IdentitySource;
pub use client::{
    Client, DEFAULT_DATA_TIMEOUT, DataAdmission, DataCancelOutcome, DataPipelineConfig,
    DataPipelineError, DataPipelineStats, DataRequestCancellation, DataRequestTicket, DrawDataClient, DrawDataError,
    DrawDataPipeline, DataUpload, DataUploadCleanup, DataUploadError, DataUploadPage, DrawDataWriter, WaitVisibleError,
    MachineClient, next_sample,
};
pub use datapod;
pub use host::{ClockLane, DataLane, DrawLane, HostBus, HostConfig, MAX_PAYLOAD_BYTES};
pub use machine::{
    ControllerDesc, LinkDesc, MachineAgent, MachineConfig, MachineLane, ToolDesc, link_records_for,
};
pub use peerbus;
pub use view::{ViewPlace, set_home_place, set_view_place, view_place};
pub use wire::*;
pub mod tyres;

#[cfg(feature = "bevy")]
pub use plugin::{
    DrawClock, DrawServicePlugin, DrawStoreRes, DrawSystems, FieldsLayoutRequest, GearboxBus, GearboxBusPlugin, MachineDeleteQueue,
    MachineLoadQueue, PhysicsActive, PhysicsStepBudget, SceneObjects, SelectionState, SimResetRequest, UsdAssetRoot, UsdLoaderPlugin,
    UsdMarkerPlugin, WeatherReport, WeatherRequest, WorldClock,
};
