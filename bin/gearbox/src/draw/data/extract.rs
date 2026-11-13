use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::{
    Extract, ExtractSchedule, RenderApp, extract_component::ExtractComponent,
    sync_component::SyncComponentPlugin, sync_world::RenderEntity,
};
use gearbox_api::draw_data::{DrawingBackend, MemoryClass};

use super::{owned::List, runtime::Shared};

pub(super) trait Budgeted: ExtractComponent {
    fn denied(item: QueryItem<'_, '_, Self::QueryData>, backend: &DrawingBackend);
}

pub(super) fn install<C: Budgeted>(app: &mut App) {
    app.add_plugins(SyncComponentPlugin::<C>::default());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.add_systems(ExtractSchedule, extract::<C>);
    }
}

fn extract<C: Budgeted>(
    mut commands: Commands,
    shared: Extract<Option<Res<Shared>>>,
    query: Extract<Query<(RenderEntity, C::QueryData), C::QueryFilter>>,
) {
    let Some(shared) = shared.as_ref() else {
        return;
    };
    let count = query.iter().count();
    if count == 0 {
        return;
    }
    let Ok(mut batch) = List::new(shared.backend.budget(), count, MemoryClass::Scratch) else {
        for (_, item) in &query {
            C::denied(item, &shared.backend);
        }
        return;
    };
    for (entity, item) in &query {
        if let Some(component) = C::extract_component(item) {
            batch
                .push((entity, component))
                .expect("reserved extraction batch slot");
        } else {
            commands.entity(entity).remove::<C::Target>();
        }
    }
    commands.try_insert_batch(batch);
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
