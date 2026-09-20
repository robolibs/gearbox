use super::*;
use crate::physics::backend::TerrainFrictionGrid;
use gearbox_fields::{FieldBounds, FieldLayout, FieldProfiles};

struct Attempt {
    entity: Entity,
    collider: ColliderId,
    grid: Arc<HeightGrid>,
    fallback: f64,
    layout: FieldLayout,
}

#[derive(Default)]
pub(super) struct Publication {
    attempt: Option<Attempt>,
    published: Option<ColliderId>,
    layout: Option<(Entity, FieldLayout)>,
}

pub(super) fn publish(
    terrain: Option<Res<ProceduralTerrain>>,
    layout: Option<Res<FieldLayout>>,
    profiles: Option<Res<FieldProfiles>>,
    mut physics: ResMut<PhysicsWorld>,
    mut state: Local<Publication>,
) {
    let Some(terrain) = terrain else {
        clear(&mut physics, &mut state);
        state.attempt = None;
        state.layout = None;
        return;
    };
    let (Some(layout), Some(profiles)) = (layout, profiles) else {
        clear(&mut physics, &mut state);
        state.attempt = None;
        return;
    };
    if !physics.uses_wheel_forces() {
        return;
    }
    let Some(collider) = physics.collider(terrain.collider) else {
        return;
    };
    let fallback = collider.friction();
    if state.attempt.as_ref().is_some_and(|last| {
        last.entity == terrain.entity
            && last.collider == terrain.collider
            && Arc::ptr_eq(&last.grid, &terrain.grid)
            && last.fallback == fallback
            && (!layout.is_changed() || last.layout == *layout)
    }) && !profiles.is_changed()
    {
        return;
    }
    state.attempt = Some(Attempt {
        entity: terrain.entity,
        collider: terrain.collider,
        grid: terrain.grid.clone(),
        fallback,
        layout: layout.clone(),
    });
    if state
        .layout
        .as_ref()
        .is_some_and(|(entity, last)| *entity == terrain.entity && !same_regions(last, &layout))
    {
        warn!("terrain: field region edits require reloading the terrain");
        return;
    }
    let grid = &terrain.grid;
    if grid.cols < 2 || grid.rows < 2 {
        warn!("terrain: field friction requires at least two rows and columns");
        return;
    }
    let domain = FieldBounds {
        min: Vec2::new(grid.min_x, grid.min_z),
        max: Vec2::new(
            grid.min_x + (grid.cols - 1) as f32 * grid.cell,
            grid.min_z + (grid.rows - 1) as f32 * grid.cell,
        ),
    };
    let result = layout
        .validate(&profiles, domain)
        .and_then(|()| layout.friction_samples(grid, fallback));
    let values = match result {
        Ok(values) => values,
        Err(error) => {
            warn!("terrain: field friction rejected: {error}");
            return;
        }
    };
    let Some(values) = values else {
        clear(&mut physics, &mut state);
        state.layout = Some((terrain.entity, layout.clone()));
        return;
    };
    let friction = TerrainFrictionGrid {
        origin: [grid.min_x as f64, grid.min_z as f64],
        cell_size: [grid.cell as f64; 2],
        cols: grid.cols,
        rows: grid.rows,
        values,
    };
    if let Err(error) = physics.register_wheel_ground(terrain.collider, Some(friction)) {
        state.attempt = None;
        warn!("terrain: field friction registration failed: {error}");
        return;
    }
    if state.published != Some(terrain.collider) {
        clear(&mut physics, &mut state);
    }
    state.published = Some(terrain.collider);
    state.layout = Some((terrain.entity, layout.clone()));
}

fn same_regions(a: &FieldLayout, b: &FieldLayout) -> bool {
    a.default == b.default
        && a.fields.len() == b.fields.len()
        && a.fields.iter().zip(&b.fields).all(|(a, b)| {
            a.name == b.name && a.profile == b.profile && a.min == b.min && a.max == b.max
        })
}

fn clear(physics: &mut PhysicsWorld, state: &mut Publication) {
    if let Some(collider) = state.published.take()
        && physics.collider(collider).is_some()
        && let Err(error) = physics.register_wheel_ground(collider, None)
    {
        state.published = Some(collider);
        warn!("terrain: field friction reset failed: {error}");
    }
}

#[cfg(test)]
mod tests;
