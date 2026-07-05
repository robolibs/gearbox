//! Viewer UI ported from `bevy_openusd::*`. Provides the full panel
//! set (Selection / Tree / Info / Variants / Cameras / Materials /
//! Overlays / Timeline / Keys / Log) on top of the gearbox simulator's
//! existing world / multi-USD loader / transform-gizmo / play button.

pub mod keyboard;
pub mod log_panel;
pub mod mara_ui;
pub mod overlays;
pub mod physics_overlay;
pub mod state;
pub mod ui;
