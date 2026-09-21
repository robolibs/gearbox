//! Overlay state + scene-extent recompute. Ported from bevy_openusd.
//! The hand-rolled grid has long been replaced by Mara's
//! `GroundGridPlugin` (which the gearbox `WorldPlugin` already wires);
//! this module just owns the
//! `DisplayToggles` resource that the Overlays panel mutates and the
//! light-intensity / wireframe glue.

use bevy::prelude::*;
use mara::ui::modules::bevy::GroundGrid;
use usd_bevy::UsdPrimRef;

pub struct OverlaysPlugin;

impl Plugin for OverlaysPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DisplayToggles>()
            .init_resource::<SceneExtent>()
            .add_systems(
                Update,
                (
                    compute_extent,
                    capture_original_light_levels,
                    apply_light_intensity_scale.after(crate::environment::DaylightUpdate),
                    apply_wireframe_toggle,
                    sync_ground_grid_visibility,
                    sync_collider_debug_visibility,
                )
                    .chain(),
            );
    }
}

pub use bevy_weather::OriginalIlluminance;

#[derive(Component, Debug, Copy, Clone)]
pub struct OriginalLightIntensity(pub f32);

fn capture_original_light_levels(
    mut cmds: Commands,
    dir: Query<
        (Entity, &DirectionalLight),
        (Added<DirectionalLight>, Without<OriginalIlluminance>),
    >,
    pt: Query<(Entity, &PointLight), (Added<PointLight>, Without<OriginalLightIntensity>)>,
    sp: Query<(Entity, &SpotLight), (Added<SpotLight>, Without<OriginalLightIntensity>)>,
) {
    for (e, l) in &dir {
        cmds.entity(e).insert(OriginalIlluminance(l.illuminance));
    }
    for (e, l) in &pt {
        cmds.entity(e).insert(OriginalLightIntensity(l.intensity));
    }
    for (e, l) in &sp {
        cmds.entity(e).insert(OriginalLightIntensity(l.intensity));
    }
}

fn apply_light_intensity_scale(
    toggles: Res<DisplayToggles>,
    mut dir: Query<(&mut DirectionalLight, &OriginalIlluminance)>,
    mut pt: Query<(&mut PointLight, &OriginalLightIntensity)>,
    mut sp: Query<(&mut SpotLight, &OriginalLightIntensity)>,
) {
    let s = toggles.light_intensity_scale;
    for (mut l, o) in &mut dir {
        l.illuminance = o.0 * s;
    }
    for (mut l, o) in &mut pt {
        l.intensity = o.0 * s;
    }
    for (mut l, o) in &mut sp {
        l.intensity = o.0 * s;
    }
}

fn apply_wireframe_toggle(
    mut toggles: ResMut<DisplayToggles>,
    available: Res<WireframeAvailable>,
    mut cfg: ResMut<bevy::pbr::wireframe::WireframeConfig>,
) {
    if !available.0 { toggles.wireframe = false; }
    if cfg.global != toggles.wireframe {
        cfg.global = toggles.wireframe;
    }
}

#[derive(Resource)]
pub(crate) struct WireframeAvailable(pub bool);

fn sync_ground_grid_visibility(toggles: Res<DisplayToggles>, mut grid: ResMut<GroundGrid>) {
    if grid.visible != toggles.show_world_grid {
        grid.visible = toggles.show_world_grid;
    }
}

fn sync_collider_debug_visibility(
    toggles: Res<DisplayToggles>,
    mut enabled: ResMut<crate::physics::ColliderDebugEnabled>,
) {
    if enabled.0 != toggles.show_colliders {
        enabled.0 = toggles.show_colliders;
    }
}

#[derive(Resource, Debug, Clone)]
pub struct DisplayToggles {
    pub show_world_grid: bool,
    pub show_world_axes: bool,
    pub show_prim_markers: bool,
    pub prim_marker_bias: f32,
    pub show_skeleton: bool,
    pub show_physics: bool,
    pub wireframe: bool,
    pub show_colliders: bool,
    /// Lifts the camera's 5 km zoom ceiling, out to where the whole planet fits.
    pub unlimited_zoom: bool,
    /// Following a machine swings the view around behind it. Off, the view
    /// keeps whatever angle it was on and only travels with the machine.
    pub follow_from_behind: bool,
    pub light_intensity_scale: f32,
    pub show_tf_frames: bool,
    pub show_tf_names: bool,
    pub show_tf_links: bool,
    pub tf_wheels_only: bool,
}

impl Default for DisplayToggles {
    fn default() -> Self {
        Self {
            show_world_grid: false,
            show_world_axes: false,
            show_prim_markers: false,
            prim_marker_bias: 1.0,
            show_skeleton: false,
            show_physics: false,
            wireframe: false,
            show_colliders: false,
            unlimited_zoom: std::env::var("GEARBOX_UNLIMITED_ZOOM").is_ok_and(|v| v == "1"),
            follow_from_behind: true,
            light_intensity_scale: 1.0,
            show_tf_frames: tf_env("frames"),
            show_tf_names: tf_env("names"),
            show_tf_links: tf_env("links"),
            tf_wheels_only: std::env::var("GEARBOX_TF_OVERLAY")
                .is_ok_and(|v| v.split(',').any(|t| t.trim() == "wheels")),
        }
    }
}

/// `GEARBOX_TF_OVERLAY=frames,names,links` (or `all`) switches TF overlay
/// toggles on at start, for scripted runs and screenshots.
fn tf_env(which: &str) -> bool {
    std::env::var("GEARBOX_TF_OVERLAY")
        .map(|v| v.split(',').any(|t| t.trim() == which || t.trim() == "all"))
        .unwrap_or(false)
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct SceneExtent {
    pub min: Vec3,
    pub max: Vec3,
    pub count: u32,
}

impl Default for SceneExtent {
    fn default() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
            count: 0,
        }
    }
}

impl SceneExtent {
    pub fn diag(&self) -> f32 {
        if self.count == 0 {
            1.0
        } else {
            (self.max - self.min).length().max(0.01)
        }
    }

    pub fn centre(&self) -> Vec3 {
        if self.count == 0 {
            Vec3::ZERO
        } else {
            (self.min + self.max) * 0.5
        }
    }
}

fn compute_extent(
    prims: Query<(&GlobalTransform, Option<&bevy::camera::primitives::Aabb>), With<UsdPrimRef>>,
    mut extent: ResMut<SceneExtent>,
) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut count = 0u32;
    for (gt, aabb) in prims.iter() {
        if let Some(aabb) = aabb {
            let m = gt.to_matrix();
            let center = Vec3::from(aabb.center);
            let half = Vec3::from(aabb.half_extents);
            for i in 0..8 {
                let local = Vec3::new(
                    if i & 1 == 0 {
                        center.x - half.x
                    } else {
                        center.x + half.x
                    },
                    if i & 2 == 0 {
                        center.y - half.y
                    } else {
                        center.y + half.y
                    },
                    if i & 4 == 0 {
                        center.z - half.z
                    } else {
                        center.z + half.z
                    },
                );
                let w = m.transform_point3(local);
                min = min.min(w);
                max = max.max(w);
            }
        } else {
            let p = gt.translation();
            min = min.min(p);
            max = max.max(p);
        }
        count += 1;
    }
    *extent = SceneExtent { min, max, count };
}
