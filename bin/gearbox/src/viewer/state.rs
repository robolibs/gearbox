//! Viewer-side state resources ported from bevy_openusd. Adapted to
//! the gearbox simulator: the active stage is whichever LoadedAsset
//! entity is currently focused (`ActiveStage`), instead of a single
//! global `StageHandle<UsdAsset>`. Selection has a separate
//! `SelectedPrim` for prim-tree clicks vs. gearbox's existing
//! top-level entity selection.

use bevy::prelude::{Entity, Resource, Vec3};
use mara::ui::modules::bevy::ChaseCamera;
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

/// Scripted "fly to behind the machine" move, started from the agent tree.
/// Phase A pins the camera in place and turns it toward the machine; phase
/// B pulls back to the apex, orbits to behind the machine and settles at
/// `distance` with the elevation the user had.
#[derive(Clone, Copy, Debug)]
pub struct FlyTarget {
    pub root: Entity,
    pub body: Entity,
    pub distance: f32,
    pub duration: f32,
    pub elapsed: f32,
    pub start_focus: Vec3,
    pub start_cam_world: Vec3,
    pub start_elevation: f32,
    pub apex_distance: f32,
    pub last_target_pos: Option<Vec3>,
}

impl FlyTarget {
    pub const APEX_DISTANCE: f32 = 45.0;
    pub const PHASE_A_END: f32 = 0.30;
    pub const FINAL_DISTANCE: f32 = 18.0;
    pub const DURATION: f32 = 3.0;

    pub fn new(root: Entity, body: Entity, cam: &ChaseCamera) -> Self {
        let horizontal = cam.distance * cam.elevation.cos();
        let vertical = cam.distance * cam.elevation.sin();
        let offset = Vec3::new(
            horizontal * cam.yaw.sin(),
            vertical,
            horizontal * cam.yaw.cos(),
        );
        Self {
            root,
            body,
            distance: Self::FINAL_DISTANCE,
            duration: Self::DURATION,
            elapsed: 0.0,
            start_focus: cam.focus,
            start_cam_world: cam.focus + offset,
            start_elevation: cam.elevation,
            apex_distance: Self::APEX_DISTANCE,
            last_target_pos: None,
        }
    }
}

/// The fly in flight, if any.
#[derive(Resource, Default, Debug)]
pub struct ChaseCameraFly {
    pub target: Option<FlyTarget>,
}

/// The loaded asset the chase camera translates with. Position only: the
/// camera keeps its yaw and distance and moves by the asset's frame-to-frame
/// delta, so looking away still works.
#[derive(Resource, Default, Debug)]
pub struct FollowTarget {
    pub entity: Option<Entity>,
    pub last_pos: Option<Vec3>,
}

impl FollowTarget {
    pub fn set(&mut self, entity: Option<Entity>) {
        self.entity = entity;
        self.last_pos = None;
    }

    pub fn toggle(&mut self, entity: Entity) {
        if self.entity == Some(entity) {
            self.set(None);
        } else {
            self.set(Some(entity));
        }
    }
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
    /// `(prim, set, selection)` triples in the shape `UsdInstanceOverrides`
    /// takes.
    pub fn to_variant_selections(&self) -> Vec<(String, String, String)> {
        self.variants
            .iter()
            .map(|((prim_path, set_name), option)| {
                (prim_path.clone(), set_name.clone(), option.clone())
            })
            .collect()
    }
}

/// One variant set on one prim of the active stage.
#[derive(Debug, Clone, Default)]
pub struct VariantEntry {
    pub prim: String,
    pub name: String,
    pub selection: Option<String>,
    pub options: Vec<String>,
}

/// Variant sets of the active stage, read when the active stage changes.
#[derive(Resource, Debug, Clone, Default)]
pub struct ActiveVariants {
    pub root: Option<Entity>,
    pub entries: Vec<VariantEntry>,
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
