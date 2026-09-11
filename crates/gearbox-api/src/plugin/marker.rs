//! Marker meshes keyed by caller id, set and moved in place over the bus.

use std::collections::HashMap;

use bevy::prelude::*;

use super::{GearboxBus, SimResetRequest};
use crate::wire::*;

#[derive(Resource, Default)]
struct Markers(HashMap<String, Entity>);

#[derive(Resource, Clone)]
struct MarkerAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

pub struct UsdMarkerPlugin;

impl Plugin for UsdMarkerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Markers>()
            .add_systems(Startup, setup_marker_assets)
            .add_systems(Update, (serve_markers, clear_on_reset, refresh_markers));
    }
}

fn refresh_markers(
    markers: Res<Markers>,
    mut objects: ResMut<super::SceneObjects>,
    transforms: Query<&Transform>,
) {
    let mut out: Vec<SceneObject> = markers
        .0
        .iter()
        .filter_map(|(id, entity)| {
            let t = transforms.get(*entity).ok()?.translation;
            Some(SceneObject {
                x: t.x,
                y: t.y,
                z: t.z,
                yaw_deg: 0.0,
                kind: object_kind::MARKER,
                props: Props::from_pairs(&[("id", id.as_str())]).into_bytes(),
            })
        })
        .collect();
    out.sort_by_key(|o| o.props().get("id"));
    objects.markers = out;
}

fn setup_marker_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(MarkerAssets {
        mesh: meshes.add(Cuboid::new(0.64, 0.45, 0.64)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.0, 0.0),
            emissive: Color::srgb(1.0, 0.0, 0.0).into(),
            perceptual_roughness: 0.5,
            metallic: 0.0,
            ..default()
        }),
    });
}

fn serve_markers(
    mut commands: Commands,
    bus: Option<ResMut<GearboxBus>>,
    assets: Option<Res<MarkerAssets>>,
    mut markers: ResMut<Markers>,
    mut transforms: Query<&mut Transform>,
) {
    let (Some(mut bus), Some(assets)) = (bus, assets) else {
        return;
    };
    let markers = markers.as_mut();
    bus.host.serve_marker_set(|req| {
        let id = req.id();
        if id.is_empty() {
            return Status::err(code::USAGE, "marker needs an `id`");
        }
        let position = Vec3::new(req.x, req.y, req.z);
        if let Some(entity) = markers.0.get(&id).copied() {
            if let Ok(mut tr) = transforms.get_mut(entity) {
                tr.translation = position;
                return Status::ok();
            }
            markers.0.remove(&id);
        }
        let entity = commands
            .spawn((
                Name::new(format!("UsdMarker[{id}]")),
                Transform::from_translation(position),
                Mesh3d(assets.mesh.clone()),
                MeshMaterial3d(assets.material.clone()),
            ))
            .id();
        markers.0.insert(id, entity);
        Status::ok()
    });
    bus.host.serve_marker_delete(|req| {
        if let Some(entity) = markers.0.remove(&req.id()) {
            commands.entity(entity).try_despawn();
        }
        Status::ok()
    });
}

fn clear_on_reset(
    mut messages: MessageReader<SimResetRequest>,
    mut commands: Commands,
    mut markers: ResMut<Markers>,
) {
    let scopes: Vec<u32> = messages.read().map(|m| m.scope).collect();
    if !scopes
        .iter()
        .any(|s| matches!(*s, clear_scope::ALL | clear_scope::MARKERS))
    {
        return;
    }
    for (_id, entity) in markers.0.drain() {
        commands.entity(entity).try_despawn();
    }
}
