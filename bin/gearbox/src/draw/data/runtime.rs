use std::sync::Arc;

use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::tonemapping::TonemappingLuts;
use bevy::ecs::query::QueryItem;
use bevy::math::{DQuat, DVec3};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::render::{
    extract_component::ExtractComponent,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    sync_component::SyncComponent,
};
use gearbox_api::draw_data::{Charge, DrawingBackend};
use gearbox_api::wire::draw_data::{DataFrame, Placement, RigidPose};
use gearbox_api::{DrawFrame, DrawStoreRes};

use super::frontier::{Offered, Retention};
use super::owned::Map;
use super::worker::{Dataset, Page, Scene, Snapshot, Worker, charge};

#[derive(Resource, Clone, ExtractResource)]
pub(in crate::draw) struct Shared {
    pub backend: DrawingBackend,
    pub mesh: Handle<Mesh>,
    pub layout: MeshVertexBufferLayoutRef,
    pub lut: Handle<Image>,
    /// GPU bytes the compact instance renderer needs for every placed page, uploaded
    /// or not; cloud residency plans around it.
    pub compact_demand: Arc<std::sync::atomic::AtomicU64>,
    _charge: Arc<Charge>,
    #[cfg(feature = "draw-probes")]
    pub(super) probe: Option<Arc<super::probe::Probe>>,
}

#[cfg(test)]
impl Shared {
    pub(in crate::draw) fn for_tests(backend: DrawingBackend, lut: Handle<Image>) -> Self {
        let metadata = charge(backend.budget(), 2048).unwrap();
        Self {
            backend,
            mesh: Handle::default(),
            layout: Mesh::from(Rectangle::default())
                .get_mesh_vertex_buffer_layout(&mut Default::default()),
            lut,
            compact_demand: Arc::default(),
            _charge: Arc::new(metadata),
            #[cfg(feature = "draw-probes")]
            probe: None,
        }
    }
}

#[derive(Component, Clone)]
pub(in crate::draw) struct Node {
    pub(super) dataset: Arc<Dataset>,
    pub(super) page: Arc<Page>,
    pub placed: bool,
    pub world_from_local: Mat4,
    pub simulation_s: f64,
    _charge: Arc<Charge>,
}

#[cfg(test)]
impl Node {
    pub(super) fn for_tests(
        budget: &gearbox_api::draw_data::CpuBudget,
        dataset: Arc<Dataset>,
        page: Arc<Page>,
    ) -> Self {
        let world_from_local = Mat4::from_translation(Vec3::from_array(
            page.appearance.geometry.rebase_m.map(|v| v as f32),
        ));
        Self {
            dataset,
            page,
            placed: true,
            world_from_local,
            simulation_s: 0.0,
            _charge: Arc::new(charge(budget, size_of::<Self>() + 16).unwrap()),
        }
    }
}

impl SyncComponent for Node {
    type Target = Self;
}
impl ExtractComponent for Node {
    type QueryData = (&'static Self, &'static GlobalTransform);
    type QueryFilter = ();
    type Out = Self;
    fn extract_component((node, transform): QueryItem<'_, '_, Self::QueryData>) -> Option<Self> {
        let mut result = node.clone();
        result.world_from_local = transform.to_matrix();
        Some(result)
    }
}

impl super::extract::Budgeted for Node {
    fn denied((node, _): QueryItem<'_, '_, Self::QueryData>, backend: &DrawingBackend) {
        super::admission::Issue::CpuBudget.report(backend, &node.dataset);
    }
}

#[derive(Resource)]
struct Runtime {
    worker: Worker,
    seen: u64,
    sequence: u64,
    snapshot: Option<Arc<Snapshot>>,
    pending: Option<Arc<Scene>>,
    demand: u64,
    nodes: Map<(usize, usize), Entity>,
}

pub(super) fn install(app: &mut App) {
    app.add_plugins((
        ExtractResourcePlugin::<Shared>::default(),
        ExtractResourcePlugin::<Offered>::default(),
    ))
    .add_systems(Startup, start)
    .add_systems(
        Update,
        (sync, place)
            .chain()
            .in_set(super::DataSystems)
            .after(gearbox_api::DrawSystems::Serve),
    );
    super::extract::install::<Node>(app);
}

fn start(
    mut commands: Commands,
    store: Res<DrawStoreRes>,
    luts: Res<TonemappingLuts>,
    #[cfg(feature = "draw-probes")] probe: Option<Res<super::probe::ProbeState>>,
) {
    let backend = store.backend();
    if let Err(error) = backend.enable_renderer_reports() {
        error!("cloud status: {error}");
        return;
    }
    let worker = match Worker::new(backend.clone()) {
        Ok(worker) => worker,
        Err(error) => {
            error!("cloud worker: {error}");
            return;
        }
    };
    let Ok(metadata) = charge(backend.budget(), 2048) else {
        return;
    };
    let layout =
        Mesh::from(Rectangle::new(1.0, 1.0)).get_mesh_vertex_buffer_layout(&mut Default::default());
    let Ok(retention) = Retention::new(backend.budget()) else {
        return;
    };
    commands.insert_resource(Offered {
        snapshot: None,
        scene: None,
        retention,
    });
    commands.insert_resource(Shared {
        backend,
        mesh: Handle::default(),
        layout,
        lut: luts.agx.clone(),
        compact_demand: Arc::default(),
        _charge: Arc::new(metadata),
        #[cfg(feature = "draw-probes")]
        probe: probe.map(|state| state.0.clone()),
    });
    commands.insert_resource(Runtime {
        worker,
        seen: u64::MAX,
        sequence: 0,
        snapshot: None,
        pending: None,
        nodes: Map::default(),
        demand: 0,
    });
}

fn sync(
    mut commands: Commands,
    shared: Option<Res<Shared>>,
    runtime: Option<ResMut<Runtime>>,
    offered: Option<ResMut<Offered>>,
) {
    let (Some(shared), Some(mut runtime), Some(mut offered)) = (shared, runtime, offered) else {
        return;
    };
    if let Some(snapshot) = &runtime.snapshot {
        snapshot.entries.prune();
    }
    if let Some(scene) = &runtime.pending {
        scene.prune();
    }
    offered.prune();
    runtime.worker.prune();
    #[cfg(feature = "draw-probes")]
    if let Some(probe) = &shared.probe {
        probe.cpu.apply(shared.backend.budget());
    }
    match Snapshot::capture(&shared.backend, &mut runtime.seen) {
        Ok(Some(snapshot))
            if !runtime
                .snapshot
                .as_ref()
                .is_some_and(|old| old.same(&snapshot)) =>
        {
            runtime.sequence = runtime.worker.submit(snapshot.clone());
            runtime.pending = None;
            runtime.snapshot = Some(snapshot);
        }
        Err(error) => {
            #[cfg(feature = "draw-probes")]
            if let Some(probe) = &shared.probe {
                probe.cpu.failed_snapshot();
            }
            warn!("cloud snapshot: {error}");
        }
        _ => {}
    }
    if let Some(scene) = runtime.worker.take()
        && scene.sequence == runtime.sequence
    {
        for failure in scene.failures.read().iter() {
            let key = failure.key();
            warn!("cloud {}/{}: {}", key.namespace, key.id, failure.message);
        }
        runtime.pending = Some(scene);
    }
    offered.snapshot = runtime.snapshot.clone();
    offered.scene = runtime.pending.clone();
    let held = offered.retention.read();
    if let Some((stamp, demand)) = &held.demand
        && *stamp != runtime.demand
    {
        runtime.demand = *stamp;
        runtime.sequence = runtime.worker.demand(demand.clone());
    }
    runtime.nodes.retain_keys_by(
        |mark| {
            offered.visit_needed(&held, |data| {
                for (index, page) in data.pages.iter().enumerate() {
                    if page.appearance.geometry.len() != 0 {
                        mark(&(Arc::as_ptr(data) as usize, index));
                    }
                }
            });
        },
        |_, entity| {
            commands.entity(*entity).try_despawn();
        },
    );
    let _ = runtime.nodes.compact(shared.backend.budget());
    offered.visit_needed(&held, |dataset| {
        for (index, page) in dataset.pages.iter().enumerate() {
            if page.appearance.geometry.len() == 0 {
                continue;
            }
            let key = (Arc::as_ptr(dataset) as usize, index);
            if runtime.nodes.contains_key(&key) {
                continue;
            }
            if runtime.nodes.reserve(shared.backend.budget(), 1).is_err() {
                super::admission::Issue::CpuBudget.report(&shared.backend, dataset);
                continue;
            }
            let Ok(metadata) = charge(shared.backend.budget(), 4096) else {
                super::admission::Issue::CpuBudget.report(&shared.backend, dataset);
                continue;
            };
            let entity = commands
                .spawn((
                    Node {
                        dataset: dataset.clone(),
                        page: page.clone(),
                        placed: false,
                        world_from_local: Mat4::IDENTITY,
                        simulation_s: 0.0,
                        _charge: Arc::new(metadata),
                    },
                    Transform::IDENTITY,
                    GlobalTransform::IDENTITY,
                    Visibility::Inherited,
                    RenderLayers::layer(super::super::DRAW_LAYER),
                ))
                .id();
            runtime
                .nodes
                .insert(shared.backend.budget(), key, entity)
                .expect("reserved cloud node slot");
        }
    });
}

fn pose(value: RigidPose) -> (DQuat, DVec3) {
    let [w, x, y, z] = value.rotation_wxyz;
    (
        DQuat::from_xyzw(x, y, z, w),
        DVec3::from_array(value.translation_m),
    )
}

/// The highest terrain on a 5 x 5 grid within `radius` of `anchor`, site metres.
fn ground_under(anchor: DVec3, radius: f64) -> f64 {
    let mut high = f64::NEG_INFINITY;
    for j in -2..=2 {
        for i in -2..=2 {
            let (dx, dz) = (f64::from(i) * radius / 2.0, f64::from(j) * radius / 2.0);
            if dx.hypot(dz) <= radius + 1e-9 {
                high = high.max(crate::globe::ground_height_at_physics(anchor.x + dx, anchor.z + dz));
            }
        }
    }
    high
}

pub(super) fn site_pose(
    frame: &DataFrame,
    page_origin: [f64; 3],
    rebase: [f64; 3],
    home: &crate::globe::SiteEntry,
) -> Option<(DVec3, DQuat)> {
    let (rotation, translation) = pose(frame.pose);
    let local = rotation
        * (DVec3::from_array(frame.origin_m)
            + DVec3::from_array(page_origin)
            + DVec3::from_array(rebase))
        + translation;
    let enu = super::super::enu_to_site().rotation.as_dquat();
    if frame.placement == Placement::Snapshot {
        let capture = frame.capture_transform.as_ref()?;
        let datum = capture.datum.as_ref()?;
        let (capture_rotation, capture_translation) = pose(capture.pose);
        let [w, x, y, z] = datum.axes_to_ecef_wxyz;
        let datum_rotation = DQuat::from_xyzw(x, y, z, w);
        let world = DVec3::from_array(datum.origin_ecef_m)
            + datum_rotation * (capture_rotation * local + capture_translation);
        let inverse = home.frame.rotation.inverse();
        return Some((
            inverse * (world - home.frame.origin),
            inverse * datum_rotation * capture_rotation * rotation,
        ));
    }
    match DrawFrame::parse(&frame.frame) {
        DrawFrame::World => {
            let mut position = enu * local;
            if let Some(radius) = frame.ground_radius_m {
                let anchor = enu * (rotation * DVec3::from_array(frame.origin_m) + translation);
                position.y += ground_under(anchor, radius);
            }
            Some((position, enu * rotation))
        }
        DrawFrame::Odom(machine) => {
            let datum = crate::globe::machine_datum(&machine)?;
            let turn = home.frame.rotation.inverse() * datum.rotation;
            Some((
                home.frame.rotation.inverse() * (datum.origin - home.frame.origin)
                    + turn * enu * local,
                turn * enu * rotation,
            ))
        }
        DrawFrame::Link(..) => None,
    }
}

fn place(
    mut commands: Commands,
    sites: Option<Res<crate::globe::Sites>>,
    grids: Query<&big_space::prelude::Grid>,
    inventory: Res<crate::controller::ControllerInventory>,
    index: Res<crate::prim_index::PrimIndex>,
    globals: Query<&GlobalTransform>,
    clock: Res<gearbox_api::DrawClock>,
    time: Res<Time>,
    toggles: Res<crate::viewer::overlays::DisplayToggles>,
    mut nodes: Query<(Entity, &mut Node, &mut Transform, Option<&ChildOf>)>,
) {
    let Some(sites) = sites else {
        return;
    };
    let home = sites.home();
    for (entity, mut node, mut transform, parent) in &mut nodes {
        node.simulation_s = clock.0.unwrap_or_else(|| time.elapsed_secs_f64());
        node.placed = false;
        if !toggles.show_draws {
            continue;
        }
        let frame = &node.page.scan.value().frame;
        let geometry = &node.page.appearance.geometry;
        if let Some((position, rotation)) = site_pose(
            frame,
            geometry.descriptor().origin_m,
            geometry.rebase_m,
            home,
        ) {
            let Ok(grid) = grids.get(home.entity) else {
                continue;
            };
            if !position.is_finite() {
                continue;
            }
            let (cell, local) = grid.translation_to_grid(position);
            *transform = Transform::from_translation(local).with_rotation(rotation.as_quat());
            commands.entity(entity).insert(cell);
            if parent.is_none_or(|parent| parent.parent() != home.entity) {
                commands.entity(entity).insert(ChildOf(home.entity));
            }
            node.placed = true;
        } else if let Some((target, local)) = link_placement(
            frame,
            geometry.descriptor().origin_m,
            geometry.rebase_m,
            &inventory,
            &index,
            &globals,
        ) {
            *transform = local;
            commands
                .entity(entity)
                .remove::<big_space::prelude::CellCoord>();
            if parent.is_none_or(|parent| parent.parent() != target) {
                commands.entity(entity).insert(ChildOf(target));
            }
            node.placed = true;
        }
    }
}

/// A live-link frame's target entity and the local transform under it.
pub(super) fn link_placement(
    frame: &DataFrame,
    page_origin: [f64; 3],
    rebase: [f64; 3],
    inventory: &crate::controller::ControllerInventory,
    index: &crate::prim_index::PrimIndex,
    globals: &Query<&GlobalTransform>,
) -> Option<(Entity, Transform)> {
    if frame.placement != Placement::Live {
        return None;
    }
    let DrawFrame::Link(machine, link) = DrawFrame::parse(&frame.frame) else {
        return None;
    };
    let spec = inventory.machines.iter().find(|m| m.id == machine)?;
    let root = spec.scene_root?;
    let link = if link.is_empty() {
        spec.links.base().or(spec.links.links.first())
    } else {
        spec.links.get(&link)
    }?;
    let target = index.find(root, &link.prim_path)?;
    let scale = globals.get(target).ok()?.to_scale_rotation_translation().0.x;
    if !scale.is_finite() || scale.abs() < 1e-6 {
        return None;
    }
    let (rotation, translation) = pose(frame.pose);
    let position = rotation
        * (DVec3::from_array(frame.origin_m) + DVec3::from_array(page_origin) + DVec3::from_array(rebase))
        + translation;
    if !position.is_finite()
        || (position.as_vec3().as_dvec3() - position).length() > super::cloud::MAX_LOCAL_ERROR_M
    {
        return None;
    }
    Some((
        target,
        Transform {
            translation: position.as_vec3() / scale,
            rotation: rotation.as_quat(),
            scale: Vec3::splat(1.0 / scale),
        },
    ))
}

pub(in crate::draw) fn uses_frost(nodes: &Query<&Node>) -> bool {
    nodes
        .iter()
        .any(|node| node.placed && node.dataset.generation.descriptor().presentation.blur > 0.0)
}

pub(in crate::draw) fn frost_admission(
    backend: &DrawingBackend,
    nodes: &Query<&Node>,
    failure: Option<(u32, &str)>,
    samples: u32,
) {
    use gearbox_api::draw_data::RenderDiagnostic;
    use gearbox_api::wire::draw_data::DataPhase;

    for node in nodes {
        let data = &node.dataset;
        let current = backend.renderer_diagnostic(data.key(), data.revision);
        if let Some((code, reason)) = failure.filter(|_| {
            [1, 2, 4, 8]
                .into_iter()
                .any(|count| samples & count != 0 && data.needs_frost(count))
        }) {
            if let Ok(failure) = RenderDiagnostic::new(DataPhase::Rendering, code, reason)
                && current != Some(failure)
            {
                let _ = backend.report_renderer_diagnostic(data.key(), data.revision, failure);
            }
        } else if current.is_some_and(|error| {
            error.phase() == DataPhase::Rendering && error.reason().starts_with("drawing frost ")
        }) {
            backend.clear_render_failure(data.key(), data.revision, DataPhase::Rendering);
        }
    }
}

#[cfg(feature = "draw-probes")]
pub(in crate::draw) fn frost_observation(
    shared: Option<&Shared>,
    view: bevy::render::view::RetainedViewEntity,
    texture: Option<&bevy::render::render_resource::Texture>,
) {
    if let Some(probe) = shared.and_then(|shared| shared.probe.as_ref()) {
        let value = texture
            .map(|texture| {
                let stride = texture.format().block_copy_size(None).unwrap_or(0);
                super::probe::Backdrop {
                    bytes: u64::from(texture.width())
                        * u64::from(texture.height())
                        * u64::from(stride),
                    size: [texture.width(), texture.height()],
                    stride,
                }
            })
            .unwrap_or_default();
        probe.backdrop(view, value);
    }
}
