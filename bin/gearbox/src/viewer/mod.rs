//! The Bevy side of the viewer: state resources, per-frame systems and
//! overlays. The panes that show and drive this state are drawn by the mara
//! host (`crate::host`), which reaches the world directly and through
//! `commands::HostCommands`.

pub(crate) mod camera;
pub mod camera_requests;
pub mod commands;
pub mod drive;
pub mod log;
pub mod machine_context;
pub mod overlays;
pub mod physics_overlay;
pub mod recorder;
pub mod screenshot;
pub mod state;
pub mod systems;
pub mod tf_overlay;
mod tyre_mesh;
mod tyre_gpu;
#[cfg(test)]
mod tyre_gpu_test;
