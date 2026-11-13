use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::{
    core_3d::main_transparent_pass_3d,
    schedule::{Core3d, Core3dSystems},
};
use bevy::image::ToExtents;
use bevy::pbr::{
    Transmissive3d, ViewTransmissionTexture, main_transmissive_pass_3d,
    prepare_mesh_view_bind_groups,
};
use bevy::prelude::*;
use bevy::render::{
    Render, RenderApp, RenderSystems,
    camera::ExtractedCamera,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    render_phase::ViewSortedRenderPhases,
    render_resource::TextureFormat,
    renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
    view::{ExtractedView, ViewTarget},
};
use gearbox_api::draw_data::MemoryClass;
use gearbox_api::wire::draw_data::data_code;
use std::sync::Arc;

#[path = "frost_targets.rs"]
mod targets;
use targets::{Failure, Target};

#[derive(Resource, Default, Clone, ExtractResource)]
struct Active(bool);

#[derive(Component)]
pub(super) struct FrostView {
    target: Arc<Target>,
}

impl FrostView {
    pub fn ready(&self, size: Option<UVec2>, format: TextureFormat) -> bool {
        self.target.matches(size, format)
    }
}

pub(super) struct FrostPlugin;

impl Plugin for FrostPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Active>()
            .add_plugins(ExtractResourcePlugin::<Active>::default())
            .add_systems(
                Update,
                update_active
                    .after(super::sync_draws)
                    .after(super::data::DataSystems),
            );
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render
            .add_systems(
                Render,
                prepare
                    .in_set(RenderSystems::PrepareBindGroups)
                    .before(prepare_mesh_view_bind_groups),
            )
            .add_systems(
                Core3d,
                copy_scene
                    .after(main_transmissive_pass_3d)
                    .before(main_transparent_pass_3d)
                    .in_set(Core3dSystems::MainPass),
            );
    }
}

fn update_active(
    store: Res<gearbox_api::DrawStoreRes>,
    nodes: Query<&super::data::runtime::Node>,
    mut active: ResMut<Active>,
) {
    active.0 = store.lock().iter().any(|(_, item)| {
        !matches!(
            item.entity.kind,
            gearbox_api::draw_kind::PANEL
                | gearbox_api::draw_kind::TEXT
                | gearbox_api::draw_kind::INTERACT
        ) && super::visual::blur(&item.entity, 0.0) > 0.0
    }) || super::data::runtime::uses_frost(&nodes);
}

fn prepare(
    mut commands: Commands,
    active: Res<Active>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    shared: Option<Res<super::data::runtime::Shared>>,
    nodes: Query<&super::data::runtime::Node>,
    phases: Res<ViewSortedRenderPhases<Transmissive3d>>,
    views: Query<(
        Entity,
        &ExtractedCamera,
        &ExtractedView,
        Option<&RenderLayers>,
        Option<&FrostView>,
        Option<&ViewTransmissionTexture>,
        &Msaa,
    )>,
) {
    let backend = shared.as_ref().map(|shared| &shared.backend);
    if !active.0 {
        for (entity, _, view, _, previous, _, _) in &views {
            let has_transmission = phases
                .get(&view.retained_view_entity)
                .is_some_and(|phase| !phase.items.is_empty());
            if previous.is_some_and(|old| old.target.owned) && !has_transmission {
                commands.entity(entity).remove::<ViewTransmissionTexture>();
            }
            commands.entity(entity).remove::<FrostView>();
            #[cfg(feature = "draw-probes")]
            super::data::runtime::frost_observation(
                shared.as_deref(),
                view.retained_view_entity,
                None,
            );
        }
        report(backend, &nodes, None, 0);
        return;
    }
    let scratch = backend
        .map(|backend| {
            let bytes = (views.iter().len() * size_of::<Arc<Target>>()) as u64;
            backend
                .budget()
                .reserve(bytes)
                .and_then(|mut r| r.charge(bytes, MemoryClass::Scratch))
        })
        .transpose();
    let Ok(_scratch) = scratch else {
        report(backend, &nodes, Some(Failure::Cpu), u32::MAX);
        return;
    };
    let mut shared_targets: Vec<Arc<Target>> = Vec::new();
    if shared_targets
        .try_reserve_exact(views.iter().len())
        .is_err()
        || shared_targets.capacity() != views.iter().len()
    {
        report(backend, &nodes, Some(Failure::Cpu), u32::MAX);
        return;
    }
    let mut failure = None;
    let mut failed_samples = 0;
    for (entity, camera, view, layers, previous, transmission, msaa) in &views {
        let has_transmission = phases
            .get(&view.retained_view_entity)
            .is_some_and(|phase| !phase.items.is_empty());
        let enabled = active.0
            && layers
                .is_some_and(|layers| layers.intersects(&RenderLayers::layer(super::DRAW_LAYER)));
        if !enabled {
            #[cfg(feature = "draw-probes")]
            super::data::runtime::frost_observation(
                shared.as_deref(),
                view.retained_view_entity,
                None,
            );
            if previous.is_some_and(|view| view.target.owned) && !has_transmission {
                commands.entity(entity).remove::<ViewTransmissionTexture>();
            }
            commands.entity(entity).remove::<FrostView>();
            continue;
        }
        let Some(size) = camera
            .physical_target_size
            .filter(|size| size.x > 0 && size.y > 0)
        else {
            continue;
        };
        let borrowed = transmission.filter(|_| has_transmission);
        let reuse = |target: &&Arc<Target>| {
            target.matches(Some(size), view.target_format)
                && borrowed.map_or(target.owned, |source| {
                    source.texture.id() == target.texture.id()
                })
        };
        let target = previous
            .map(|old| &old.target)
            .filter(reuse)
            .or_else(|| borrowed.and_then(|_| shared_targets.iter().find(reuse)))
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| {
                Target::prepare(&device, &queue, backend, size, view.target_format, borrowed)
            });
        match target {
            Ok(target) => {
                #[cfg(feature = "draw-probes")]
                super::data::runtime::frost_observation(
                    shared.as_deref(),
                    view.retained_view_entity,
                    Some(&target.texture),
                );
                commands.entity(entity).insert((
                    target.binding(),
                    FrostView {
                        target: target.clone(),
                    },
                ));
                shared_targets.push(target);
            }
            Err(error) => {
                #[cfg(feature = "draw-probes")]
                super::data::runtime::frost_observation(
                    shared.as_deref(),
                    view.retained_view_entity,
                    None,
                );
                failure = Some(error);
                failed_samples |= msaa.samples();
                if previous.is_some_and(|old| old.target.owned) && !has_transmission {
                    commands.entity(entity).remove::<ViewTransmissionTexture>();
                }
                commands.entity(entity).remove::<FrostView>();
            }
        }
    }
    report(backend, &nodes, failure, failed_samples);
}

fn report(
    backend: Option<&gearbox_api::draw_data::DrawingBackend>,
    nodes: &Query<&super::data::runtime::Node>,
    failure: Option<Failure>,
    samples: u32,
) {
    let Some(backend) = backend else {
        return;
    };
    super::data::runtime::frost_admission(
        backend,
        nodes,
        failure.map(|error| {
            (
                if error == Failure::Extent {
                    data_code::UNSUPPORTED
                } else {
                    data_code::BUDGET
                },
                error.reason(),
            )
        }),
        samples,
    );
}

fn copy_scene(
    view: ViewQuery<(
        &ExtractedCamera,
        &ViewTarget,
        Option<&FrostView>,
        Option<&ViewTransmissionTexture>,
    )>,
    mut context: RenderContext,
) {
    let (camera, target, frost, texture) = view.into_inner();
    let (Some(frost), Some(texture), Some(size)) = (frost, texture, camera.physical_target_size)
    else {
        return;
    };
    if !frost.ready(Some(size), target.main_texture().format())
        || frost.target.texture.id() != texture.texture.id()
    {
        return;
    }
    context.command_encoder().copy_texture_to_texture(
        target.main_texture().as_image_copy(),
        texture.texture.as_image_copy(),
        size.to_extents(),
    );
}
