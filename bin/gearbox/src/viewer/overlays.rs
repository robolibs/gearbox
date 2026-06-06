//! Overlay state + scene-extent recompute. Ported from bevy_openusd.
//! The hand-rolled grid has long been replaced by `bevy_glacial`'s
//! `GroundGridPlugin` (which the gearbox `WorldPlugin` already wires);
//! this module just owns the
//! `DisplayToggles` resource that the Overlays panel mutates and the
//! light-intensity / wireframe glue.

use bevy::prelude::*;
use bevy_glacial::GroundGrid;
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
                    apply_light_intensity_scale,
                    apply_wireframe_toggle,
                    sync_ground_grid_visibility,
                    sync_collider_debug_visibility,
                )
                    .chain(),
            );
    }
}

#[derive(Component, Debug, Copy, Clone)]
pub struct OriginalIlluminance(pub f32);

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
    toggles: Res<DisplayToggles>,
    mut cfg: ResMut<bevy::pbr::wireframe::WireframeConfig>,
) {
    if cfg.global != toggles.wireframe {
        cfg.global = toggles.wireframe;
    }
}

fn sync_ground_grid_visibility(toggles: Res<DisplayToggles>, mut grid: ResMut<GroundGrid>) {
    if grid.visible != toggles.show_world_grid {
        grid.visible = toggles.show_world_grid;
    }
}

fn sync_collider_debug_visibility(
    toggles: Res<DisplayToggles>,
    mut enabled: ResMut<usd_bevy::physics::ColliderDebugEnabled>,
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
    pub light_intensity_scale: f32,
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
            light_intensity_scale: 1.0,
        }
    }
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
    prims: Query<
        (
            &GlobalTransform,
            Option<&usd_bevy::UsdLocalExtent>,
            Option<&bevy::camera::primitives::Aabb>,
        ),
        With<UsdPrimRef>,
    >,
    mut extent: ResMut<SceneExtent>,
) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut count = 0u32;
    for (gt, local, aabb) in prims.iter() {
        if let Some(le) = local {
            let m = gt.to_matrix();
            for i in 0..8 {
                let c = Vec3::new(
                    if i & 1 == 0 { le.min[0] } else { le.max[0] },
                    if i & 2 == 0 { le.min[1] } else { le.max[1] },
                    if i & 4 == 0 { le.min[2] } else { le.max[2] },
                );
                let w = m.transform_point3(c);
                min = min.min(w);
                max = max.max(w);
            }
        } else if let Some(aabb) = aabb {
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
