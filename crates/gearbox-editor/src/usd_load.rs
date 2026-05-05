//! Editor-owned queue for "Load USD…" ribbon button events.
//!
//! The button click in `dock_ribbons` opens a native file dialog
//! (rfd) and pushes the picked absolute paths into [`LoadUsdQueue`].
//! The host binary (`bin/gearbox`) is the side that knows what to do
//! with the files — it drains the queue each frame and spawns the
//! USDs as `SceneRoot`s.
//!
//! This crate intentionally doesn't depend on `usd_bevy`; the queue
//! is just `PathBuf`s. That keeps the editor crate the same shape it
//! was before USD existed (it has no opinion on what a USD is).

use std::path::PathBuf;

use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct LoadUsdQueue(pub Vec<PathBuf>);

/// Marker: this entity is a SceneRoot of a USD that the user loaded
/// via the editor's `📂 Load USD…` button (or `--usd <path>` CLI).
/// The picker queries this to find pickable USD roots; the gizmo
/// systems use it to switch the proxy between vehicle-pose and
/// entity-Transform sources.
///
/// `pick_radius` is a coarse sphere around the entity's position used
/// for ray picking. Real per-prim AABB picking is a follow-up.
#[derive(Component, Debug, Clone, Copy)]
pub struct UsdSelectable {
    pub pick_radius: f32,
}

impl Default for UsdSelectable {
    fn default() -> Self {
        Self { pick_radius: 1.5 }
    }
}
