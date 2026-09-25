use super::*;
use crate::physics::backend::TerrainFrictionGrid;
use gearbox_fields::{FieldBounds, FieldLayout, FieldProfiles};

struct Attempt {
    entity: Entity,
    colliders: Vec<ColliderId>,
    grid: Arc<HeightGrid>,
    fallback: f64,
    layout: FieldLayout,
}

#[derive(Default)]
pub(super) struct Publication {
    attempt: Option<Attempt>,
    published: Vec<ColliderId>,
    layout: Option<(Entity, FieldLayout)>,
}

/// The ground under the wheels is streamed in chunks, so the friction grid is
/// registered on every chunk that is currently laid. The grid is in world
/// coordinates, so each chunk reads its own part of the one grid.
pub(super) fn publish(
    terrain: Option<Res<ProceduralTerrain>>,
    layout: Option<Res<FieldLayout>>,
    profiles: Option<Res<FieldProfiles>>,
    ground: Res<super::GroundColliders>,
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
    let mut colliders: Vec<ColliderId> = ground.0.values().copied().collect();
    colliders.sort();
    let Some(fallback) = colliders
        .first()
        .and_then(|collider| physics.collider(*collider))
        .map(|collider| collider.friction())
    else {
        return;
    };
    if state.attempt.as_ref().is_some_and(|last| {
        last.entity == terrain.entity
            && last.colliders == colliders
            && Arc::ptr_eq(&last.grid, &terrain.grid)
            && last.fallback == fallback
            && (!layout.is_changed() || last.layout == *layout)
    }) && !profiles.is_changed()
    {
        return;
    }
    state.attempt = Some(Attempt {
        entity: terrain.entity,
        colliders: colliders.clone(),
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
    for collider in &colliders {
        if let Err(error) = physics.register_wheel_ground(*collider, Some(friction.clone())) {
            state.attempt = None;
            warn!("terrain: field friction registration failed: {error}");
            return;
        }
    }
    // Chunks that went away since the last publication keep no grid.
    let stale: Vec<ColliderId> =
        state.published.iter().copied().filter(|c| !colliders.contains(c)).collect();
    state.published = stale;
    clear(&mut physics, &mut state);
    state.published = colliders;
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
    let mut kept = Vec::new();
    for collider in std::mem::take(&mut state.published) {
        if physics.collider(collider).is_none() {
            continue;
        }
        if let Err(error) = physics.register_wheel_ground(collider, None) {
            kept.push(collider);
            warn!("terrain: field friction reset failed: {error}");
        }
    }
    state.published = kept;
}

#[cfg(test)]
mod tests;
