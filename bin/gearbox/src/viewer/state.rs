//! Viewer-side state resources ported from bevy_openusd. Adapted to
//! the gearbox simulator: the active stage is whichever LoadedAsset
//! entity is currently focused (`ActiveStage`), instead of a single
//! global `StageHandle<UsdAsset>`. Selection has a separate
//! `SelectedPrim` for prim-tree clicks vs. gearbox's existing
//! top-level entity selection.

use bevy::prelude::{Entity, Resource, Vec3};
use std::path::PathBuf;

/// The currently-focused loaded USD entity. Drives every panel that
/// needs a stage handle (Tree, Info, Variants, Cameras, Materials,
/// Timeline, Selection). When `None` the panels render an empty /
/// "no stage" state.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct ActiveStage(pub Option<Entity>);

/// Per-active-stage metadata snapshot. Captured by
/// `capture_active_stage_info` once an asset finishes loading. Best-
/// effort: only the fields that are easy to read off `UsdAsset`.
#[derive(Resource, Default, Debug, Clone)]
pub struct StageInfo {
    pub path: String,
    pub default_prim: Option<String>,
    pub layer_count: usize,
    pub variant_count: usize,
    pub lights_directional: usize,
    pub lights_point: usize,
    pub lights_spot: usize,
    pub lights_dome: usize,
    pub instance_prim_count: usize,
    pub instance_prototype_reuses: usize,
    pub animated_prim_count: usize,
    pub skeleton_count: usize,
    pub skel_root_count: usize,
    pub skel_binding_count: usize,
    pub render_settings_count: usize,
    pub render_product_count: usize,
    pub render_var_count: usize,
    pub render_primary_resolution: Option<[i32; 2]>,
    pub render_primary_path: Option<String>,
    pub rigid_body_count: usize,
    pub physics_scene_count: usize,
    pub joint_count: usize,
    pub custom_attr_prim_count: usize,
    pub custom_layer_data_entries: usize,
    pub subdivision_prim_count: usize,
    pub light_linked_count: usize,
    pub clip_prim_count: usize,
}

/// Hot-reload request for the active stage. R key + UI button set
/// `requested = true`; `apply_reload_request` despawns the active
/// LoadedAsset and re-pushes its path through `LoadQueue`.
#[derive(Resource, Default, Debug, Clone)]
pub struct ReloadRequest {
    pub requested: bool,
}

/// Single-USD browse request. The original viewer re-exec'd the binary
/// for an asset swap; in gearbox we simply push the picked path onto
/// the `LoadQueue`, which adds it to the scene without dropping
/// anything else.
#[derive(Resource, Default, Debug, Clone)]
pub struct LoadRequest {
    pub path: Option<PathBuf>,
}

/// Currently-selected prim in the active stage's hierarchy.
/// The highlight system reads this; the fly-to system watches changes.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct SelectedPrim(pub Option<Entity>);

/// In-flight camera tween. Identical semantics to the viewer.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct FlyTo {
    pub target_focus: Vec3,
    pub target_distance: f32,
    pub remaining: f32,
    pub duration: f32,
    pub start_focus: Vec3,
    pub start_distance: f32,
    pub start_yaw: Option<f32>,
    pub target_yaw: Option<f32>,
    pub start_elevation: Option<f32>,
    pub target_elevation: Option<f32>,
}

/// Saved camera viewpoints — `Cameras` panel.
#[derive(Resource, Default, Debug, Clone)]
pub struct CameraBookmarks {
    pub items: Vec<CameraBookmark>,
    pub next_seq: u32,
}

#[derive(Debug, Clone)]
pub struct CameraBookmark {
    pub name: String,
    pub focus: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub elevation: f32,
}

/// Camera mount mode. Mounted-USD-camera support is currently a stub
/// in the simulator (the viewer's full follow-mounted-camera system
/// hasn't been ported yet). The Cameras panel still surfaces the
/// option so the UI is consistent.
#[derive(Resource, Debug, Clone, Default)]
pub enum CameraMount {
    #[default]
    Arcball,
    Mounted {
        prim_path: String,
    },
}

/// Curve / point rendering knobs + variant overrides — same struct as
/// the viewer. The variants map is consulted by `apply_reload_request`
/// when re-pushing an asset to `LoadQueue`.
#[derive(Resource, Debug, Clone, Default)]
pub struct LoaderTuning {
    pub curves: CurveTuning,
    pub variants: std::collections::HashMap<(String, String), String>,
}

impl LoaderTuning {
    pub fn to_variant_selections(&self) -> Vec<usd_bevy::VariantSelection> {
        self.variants
            .iter()
            .map(
                |((prim_path, set_name), option)| usd_bevy::VariantSelection {
                    prim_path: prim_path.clone(),
                    set_name: set_name.clone(),
                    option: option.clone(),
                },
            )
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CurveTuning {
    pub default_radius: f32,
    pub ring_segments: u32,
    pub point_scale: f32,
}

impl Default for CurveTuning {
    fn default() -> Self {
        Self {
            default_radius: 0.02,
            ring_segments: 6,
            point_scale: 1.0,
        }
    }
}

/// Animation playback clock — driven by `tick_stage_time`.
#[derive(Resource, Debug, Clone, Copy)]
pub struct UsdStageTime {
    pub seconds: f64,
    pub playing: bool,
    pub start_time_code: f64,
    pub end_time_code: f64,
    pub time_codes_per_second: f64,
    pub initialized: bool,
}

impl Default for UsdStageTime {
    fn default() -> Self {
        Self {
            seconds: 0.0,
            playing: false,
            start_time_code: 0.0,
            end_time_code: 1.0,
            time_codes_per_second: 24.0,
            initialized: false,
        }
    }
}

impl UsdStageTime {
    pub fn current_time_code(&self) -> f64 {
        self.start_time_code + self.seconds * self.time_codes_per_second
    }
    pub fn duration_seconds(&self) -> f64 {
        (self.end_time_code - self.start_time_code).max(0.0) / self.time_codes_per_second
    }
}
