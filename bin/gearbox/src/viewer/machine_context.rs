//! Live machine cards, coupling candidates and terrain-following selection bands.

use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology, prelude::*};
use usd_bevy::UsdPrimRef;

use super::systems::{Selection, SelectionRing};
use crate::{
    attach::{
        Attachments, LocalAttachmentAction, SNAP_ANGLE_RAD, SNAP_DISTANCE_M, body_link, would_loop,
    },
    controller::{ControllerInventory, MachineAgentKeys, find_prim_entity, is_descendant_of},
    links::CouplingSide,
    physics::PhysicsWorld,
};

#[derive(Resource, Default)]
pub(crate) struct MachineHover {
    pub hit: Option<Entity>,
    pub retained: Option<Entity>,
    pub captures_pointer: bool,
}

#[derive(Clone)]
pub(crate) struct CouplingAction {
    pub action: LocalAttachmentAction,
    pub endpoints: [Vec3; 2],
    pub label: String,
    pub detail: String,
    pub enabled: bool,
}

#[derive(Clone)]
pub(crate) struct MachineCard {
    pub ns: String,
    pub root: Entity,
    pub ground_center: Vec3,
    pub ground_right: Vec3,
    pub ground_down: Vec3,
    pub center: Vec3,
    pub radius: f32,
    pub stopped: bool,
    pub available: bool,
    pub actions: Vec<CouplingAction>,
}

#[derive(Resource, Default)]
pub(crate) struct MachineCards(pub Vec<MachineCard>);

struct Port {
    machine: usize,
    name: String,
    kind: String,
    side: CouplingSide,
    position: Vec3,
    rotation: Quat,
}

pub(super) struct MachineContextPlugin;

impl Plugin for MachineContextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MachineHover>()
            .init_resource::<MachineCards>()
            .add_systems(Startup, spawn_band)
            .add_systems(
                PostUpdate,
                (publish_cards, update_band)
                    .chain()
                    .after(bevy::transform::TransformSystems::Propagate),
            );
    }
}

fn publish_cards(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<Res<gearbox_api::GearboxBus>>,
    attachments: Res<Attachments>,
    physics: Res<PhysicsWorld>,
    prims: Query<(Entity, &UsdPrimRef)>,
    parents: Query<&ChildOf>,
    transforms: Query<&GlobalTransform>,
    meshes: Query<
        (
            Entity,
            &GlobalTransform,
            &bevy::camera::primitives::Aabb,
            Option<&InheritedVisibility>,
        ),
        With<Mesh3d>,
    >,
    mut cards: ResMut<MachineCards>,
) {
    cards.0.clear();
    let mut ports = Vec::new();
    let mut agents: Vec<_> = keys.0.iter().collect();
    agents.sort_by_key(|(ns, _)| *ns);
    for (ns, key) in agents {
        let Some(machine) = inventory
            .machines
            .iter()
            .find(|m| m.scene_root == Some(key.scene_root) && m.id == key.machine_id)
        else {
            continue;
        };
        let entity_of = |p: &str| find_prim_entity(key.scene_root, p, &prims, &parents);
        let machine_entity = entity_of(&machine.prim_path).unwrap_or(key.scene_root);
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for (entity, gt, aabb, visibility) in &meshes {
            if visibility.is_some_and(|v| !v.get())
                || !is_descendant_of(entity, machine_entity, &parents)
            {
                continue;
            }
            let center = gt.transform_point(Vec3::from(aabb.center));
            let matrix = gt.affine().matrix3;
            let half = matrix.x_axis.abs() * aabb.half_extents.x
                + matrix.y_axis.abs() * aabb.half_extents.y
                + matrix.z_axis.abs() * aabb.half_extents.z;
            min = min.min(center - Vec3::from(half));
            max = max.max(center + Vec3::from(half));
        }
        if !min.is_finite() {
            continue;
        }
        let center = (min + max) * 0.5;
        let body = machine
            .body
            .as_deref()
            .and_then(entity_of)
            .and_then(|e| physics.entity_to_body.get(&e))
            .and_then(|h| physics.bodies.get(*h));
        let index = cards.0.len();
        for (link, coupling) in machine.links.couplings() {
            let Some(gt) = entity_of(&link.prim_path).and_then(|e| transforms.get(e).ok()) else {
                continue;
            };
            let Some(body_gt) = body_link(&machine.links, link)
                .and_then(|l| l.body_prim.as_deref())
                .and_then(entity_of)
                .and_then(|e| transforms.get(e).ok())
            else {
                continue;
            };
            ports.push(Port {
                machine: index,
                name: coupling.name.clone(),
                kind: coupling.kind.clone(),
                side: coupling.side,
                position: gt.translation(),
                rotation: body_gt.rotation(),
            });
        }
        let right = body
            .map(|b| {
                let q = b.rotation();
                Quat::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32) * Vec3::X
            })
            .map(|v| Vec3::new(v.x, 0.0, v.z).normalize_or(Vec3::X))
            .unwrap_or(Vec3::X);
        let radius = ((max - min).xz().length() * 0.5 + 0.35).max(1.0);
        cards.0.push(MachineCard {
            ns: ns.clone(),
            root: key.scene_root,
            ground_center: center + right * (radius + 0.55),
            ground_right: right.cross(Vec3::Y),
            ground_down: -right,
            center,
            radius,
            stopped: body.is_some_and(|b| b.linvel().length() < 0.3 && b.angvel().length() < 0.2),
            available: bus
                .as_ref()
                .and_then(|b| b.machines.get(ns))
                .is_some_and(|a| a.session_id() == 0),
            actions: Vec::new(),
        });
    }
    for attachment in &attachments.0 {
        let Some(mi) = cards.0.iter().position(|c| c.ns == attachment.master_ns) else {
            continue;
        };
        let Some(si) = cards.0.iter().position(|c| c.ns == attachment.slave_ns) else {
            continue;
        };
        let enabled = [mi, si]
            .into_iter()
            .all(|i| cards.0[i].stopped && cards.0[i].available);
        let Some(hitch) = ports.iter().find(|p| {
            p.machine == mi && p.side == CouplingSide::Hitch && p.name == attachment.hitch
        }) else {
            continue;
        };
        let Some(coupler) = ports.iter().find(|p| {
            p.machine == si && p.side == CouplingSide::Coupler && p.name == attachment.coupler
        }) else {
            continue;
        };
        let action = CouplingAction {
            action: LocalAttachmentAction::Disconnect {
                master: attachment.master_ns.clone(),
                slave: attachment.slave_ns.clone(),
            },
            endpoints: [hitch.position, coupler.position],
            label: "Disconnect".into(),
            detail: format!(
                "{} ↔ {} · {}{}",
                attachment.master_ns,
                attachment.slave_ns,
                attachment.kind,
                if enabled {
                    ""
                } else {
                    " · stop / release control first"
                }
            ),
            enabled,
        };
        cards.0[mi].actions.push(action.clone());
        cards.0[si].actions.push(action);
    }
    for hitch in ports.iter().filter(|p| p.side == CouplingSide::Hitch) {
        for coupler in ports.iter().filter(|p| {
            p.side == CouplingSide::Coupler && p.kind == hitch.kind && p.machine != hitch.machine
        }) {
            let (master, slave) = (&cards.0[hitch.machine], &cards.0[coupler.machine]);
            let gap = hitch.position.distance(coupler.position) as f64;
            if gap > 3.0
                || attachments.0.iter().any(|a| {
                    a.slave_ns == slave.ns || (a.master_ns == master.ns && a.hitch == hitch.name)
                })
                || would_loop(&attachments.0, &master.ns, &slave.ns)
            {
                continue;
            }
            let angle = hitch.rotation.angle_between(coupler.rotation) as f64;
            let reason = if !master.available || !slave.available {
                "Release external control"
            } else if !master.stopped || !slave.stopped {
                "Stop both machines"
            } else if gap > SNAP_DISTANCE_M {
                "Move closer"
            } else if angle > SNAP_ANGLE_RAD {
                "Align machines"
            } else {
                "Ready to connect"
            };
            let action = CouplingAction {
                action: LocalAttachmentAction::Connect {
                    master: master.ns.clone(),
                    slave: slave.ns.clone(),
                    hitch: hitch.name.clone(),
                    coupler: coupler.name.clone(),
                },
                endpoints: [hitch.position, coupler.position],
                label: "Connect".into(),
                detail: format!(
                    "{} ↔ {}\n{} / {} · {:.0} cm · {:.0}°\n{}",
                    master.ns,
                    slave.ns,
                    hitch.name,
                    coupler.name,
                    gap * 100.0,
                    angle.to_degrees(),
                    reason
                ),
                enabled: reason == "Ready to connect",
            };
            cards.0[hitch.machine].actions.push(action.clone());
            cards.0[coupler.machine].actions.push(action);
        }
    }
}

#[derive(Component)]
struct SelectionBand;

pub(crate) fn ring_color(selected: bool) -> LinearRgba {
    if selected {
        LinearRgba::new(0.12, 0.8, 0.65, 0.65)
    } else {
        LinearRgba::new(0.55, 0.8, 0.95, 0.5)
    }
}

fn spawn_band(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        SelectionBand,
        Mesh3d(meshes.add(band_mesh(&[], None))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            double_sided: true,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
        bevy::light::NotShadowCaster,
        bevy::light::NotShadowReceiver,
        bevy::camera::visibility::NoFrustumCulling,
    ));
}

fn band_mesh(
    bands: &[(Vec3, f32, bool)],
    terrain: Option<&crate::terrain::ProceduralTerrain>,
) -> Mesh {
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    for &(center, radius, selected) in bands {
        let segments = ((std::f32::consts::TAU * radius / 0.10).ceil() as usize).clamp(128, 4096);
        for segment in 0..segments {
            let a = segment as f32 * std::f32::consts::TAU / segments as f32;
            let b = (segment + 1) as f32 * std::f32::consts::TAU / segments as f32;
            let strong = (segment as f32 * 4.0 / segments as f32).fract() < 0.75;
            let width = if strong { 0.16 } else { 0.055 };
            let mut color = ring_color(selected).to_f32_array();
            if !strong {
                color[3] *= 0.3;
            }
            let point = |angle: f32, r: f32| {
                let x = center.x + angle.cos() * r;
                let z = center.z + angle.sin() * r;
                let ground = terrain
                    .and_then(|t| t.surface_height_m(x, z))
                    .unwrap_or_else(|| crate::world::terrain_height_m(x, z));
                [x, ground + 0.10, z]
            };
            let quad = [
                point(a, radius),
                point(b, radius),
                point(b, radius - width),
                point(a, radius - width),
            ];
            for i in [0, 1, 2, 0, 2, 3] {
                positions.push(quad[i]);
                colors.push(color);
            }
        }
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
}

fn update_band(
    cards: Res<MachineCards>,
    selection: Res<Selection>,
    hover: Res<MachineHover>,
    active: Res<gearbox_api::PhysicsActive>,
    mut ring: ResMut<SelectionRing>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut band: Query<(&Mesh3d, &mut Visibility), With<SelectionBand>>,
    terrain: Option<Res<crate::terrain::ProceduralTerrain>>,
) {
    let Ok((mesh, mut visibility)) = band.single_mut() else {
        return;
    };
    let mut bands = Vec::new();
    for card in &cards.0 {
        let selected = selection.0 == Some(card.root);
        if selected {
            ring.outer_radius = card.radius;
        }
        if active.0 && (selected || hover.retained.or(hover.hit) == Some(card.root)) {
            bands.push((card.center, card.radius, selected));
        }
    }
    *visibility = if bands.is_empty() {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    if !bands.is_empty()
        && let Some(mut mesh) = meshes.get_mut(&mesh.0)
    {
        *mesh = band_mesh(&bands, terrain.as_deref());
    }
}
