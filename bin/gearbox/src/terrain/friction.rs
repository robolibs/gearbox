use super::*;
use crate::physics::backend::TerrainFrictionGrid;
use gearbox_fields::{FieldLayout, FieldProfiles};

struct Attempt {
    grid: Arc<HeightGrid>,
    fallback: f64,
    layout: FieldLayout,
}

#[derive(Default)]
struct ChunkPublication {
    attempt: Option<Attempt>,
    layout: Option<FieldLayout>,
    published: bool,
}

#[derive(Default)]
pub(super) struct Publication {
    chunks: HashMap<ColliderId, ChunkPublication>,
}

pub(super) fn publish(
    terrain: Option<Res<ProceduralTerrain>>,
    ground: Option<Res<GroundColliders>>,
    layout: Option<Res<FieldLayout>>,
    profiles: Option<Res<FieldProfiles>>,
    mut physics: ResMut<PhysicsWorld>,
    mut state: Local<Publication>,
) {
    if !physics.uses_wheel_forces() {
        return;
    }
    if terrain.is_none() || ground.is_none() {
        for (&collider, entry) in &mut state.chunks {
            clear(&mut physics, collider, entry);
        }
        state.chunks.clear();
        return;
    }
    let ground = ground.unwrap();
    state.chunks.retain(|collider, entry| {
        let live = ground.0.values().any(|chunk| chunk.collider == *collider);
        if !live {
            clear(&mut physics, *collider, entry);
        }
        live
    });
    let (Some(layout), Some(profiles)) = (layout, profiles) else {
        for (&collider, entry) in &mut state.chunks {
            clear(&mut physics, collider, entry);
            entry.attempt = None;
        }
        return;
    };
    for (&(site, _), chunk) in &ground.0 {
        let Some(collider) = physics.collider(chunk.collider) else { continue };
        let fallback = collider.friction();
        let entry = state.chunks.entry(chunk.collider).or_default();
        if entry.attempt.as_ref().is_some_and(|last| {
            Arc::ptr_eq(&last.grid, &chunk.grid)
                && last.fallback == fallback
                && (!layout.is_changed() || last.layout == *layout)
        }) && !profiles.is_changed() {
            continue;
        }
        entry.attempt = Some(Attempt {
            grid: chunk.grid.clone(), fallback, layout: layout.clone(),
        });
        if entry.layout.as_ref().is_some_and(|last| !same_regions(last, &layout)) {
            warn!("terrain: field region edits require reloading the terrain");
            continue;
        }
        let grid = &chunk.grid;
        let result = layout.validate(&profiles)
            .and_then(|()| layout.friction_samples(grid, fallback));
        let values = match result {
            Ok(values) => values,
            Err(error) => {
                warn!("terrain: field friction rejected: {error}");
                continue;
            }
        };
        let Some(values) = values else {
            clear(&mut physics, chunk.collider, entry);
            entry.layout = Some(layout.clone());
            continue;
        };
        let friction = TerrainFrictionGrid {
            origin: [
                grid.min_x as f64 + gearbox_globe::physics_offset(site).x,
                grid.min_z as f64,
            ],
            cell_size: [grid.cell as f64; 2],
            cols: grid.cols,
            rows: grid.rows,
            values,
        };
        if let Err(error) = physics.register_wheel_ground(chunk.collider, Some(friction)) {
            entry.attempt = None;
            warn!("terrain: field friction registration failed: {error}");
            continue;
        }
        entry.published = true;
        entry.layout = Some(layout.clone());
    }
}

fn same_regions(a: &FieldLayout, b: &FieldLayout) -> bool {
    a.default == b.default
        && a.ways == b.ways
        && a.fields.len() == b.fields.len()
        && a.fields.iter().zip(&b.fields).all(|(a, b)| {
            a.name == b.name && a.profile == b.profile && a.min == b.min && a.max == b.max
                && a.wear == b.wear && a.way == b.way && a.way_width == b.way_width
                && a.tyre == b.tyre
        })
}

fn clear(physics: &mut PhysicsWorld, collider: ColliderId, state: &mut ChunkPublication) {
    if !state.published {
        return;
    }
    if physics.collider(collider).is_some()
        && let Err(error) = physics.register_wheel_ground(collider, None)
    {
        warn!("terrain: field friction reset failed: {error}");
        return;
    }
    state.published = false;
}

#[cfg(test)]
mod tests;
