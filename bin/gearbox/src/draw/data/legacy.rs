use std::sync::Arc;

use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::render::sync_component::SyncComponent;
use gearbox_api::draw_data::{Charge, CpuBudget, StorageError};
use gearbox_api::{DrawEntity, draw_flag, draw_kind, draw_style};

use super::cloud::{Position, Records, invalid};
use super::worker::charge;

#[path = "legacy_render.rs"]
pub(super) mod render;
#[cfg(test)]
#[path = "legacy_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Sample {
    rgba: u32,
    radius: f32,
    phase: f32,
    padding: f32,
}

struct RecordBuffer<T> {
    records: Records<T>,
    _charge: Charge,
}

impl<T: Copy + Default> RecordBuffer<T> {
    fn new(budget: &CpuBudget, count: usize) -> Result<Self, StorageError> {
        Ok(Self {
            records: Records::new(budget, count)?,
            _charge: charge(budget, size_of::<Self>() + 2 * size_of::<usize>())?,
        })
    }
}

impl<T> std::ops::Deref for RecordBuffer<T> {
    type Target = Records<T>;
    fn deref(&self) -> &Self::Target {
        &self.records
    }
}

impl<T> std::ops::DerefMut for RecordBuffer<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.records
    }
}

#[derive(Clone, ShaderType)]
struct Uniform {
    unused: Vec4,
    world_from_local: Mat4,
    fx: Vec4,
    fx2: Vec4,
    code: UVec4,
}

pub(in crate::draw) struct Points {
    positions: Arc<RecordBuffer<Position>>,
    samples: Arc<RecordBuffer<Sample>>,
    uniform: Uniform,
    center: Vec3,
    opaque: bool,
    on_top: bool,
    _charge: Charge,
}

impl Points {
    pub fn prepare(
        budget: &CpuBudget,
        e: &DrawEntity,
        ns: &str,
        id: &str,
        old: Option<&Self>,
    ) -> Result<Option<Arc<Self>>, StorageError> {
        if e.kind != draw_kind::POINTS {
            return Ok(None);
        }
        let _scratch = charge(budget, e.values.len() * size_of::<f32>())?;
        let Ok(look) = super::super::mesh::compact_point_look(e, ns, id) else {
            return Ok(None);
        };
        if e.points.len() % 3 != 0 || e.points.iter().any(|value| !value.is_finite()) {
            return Err(invalid("legacy point positions must be finite triples"));
        }
        let count = e.points.len() / 3;
        if count > u32::MAX as usize {
            return Err(StorageError::Allocation);
        }
        let positions = match old.filter(|old| {
            old.positions.values.len() == count
                && old
                    .positions
                    .values
                    .iter()
                    .zip(e.points.chunks_exact(3))
                    .all(|(old, point)| {
                        old.xyz
                            .iter()
                            .zip(point)
                            .all(|(a, b)| a.to_bits() == b.to_bits())
                    })
        }) {
            Some(old) => old.positions.clone(),
            None => {
                let mut records = RecordBuffer::new(budget, count)?;
                for (index, (record, point)) in records
                    .values
                    .iter_mut()
                    .zip(e.points.chunks_exact(3))
                    .enumerate()
                {
                    *record = Position {
                        xyz: [point[0], point[1], point[2]],
                        source_index: index as u32,
                    };
                }
                Arc::new(records)
            }
        };
        let center = old
            .filter(|old| Arc::ptr_eq(&old.positions, &positions))
            .map(|old| old.center)
            .unwrap_or_else(|| {
                Aabb::enclosing(
                    positions
                        .values
                        .iter()
                        .map(|point| Vec3::from_array(point.xyz)),
                )
                .map(|bounds| Vec3::from(bounds.center))
                .unwrap_or(Vec3::ZERO)
            });
        let sample = |index| {
            let (rgba, radius, phase) = look.sample(index);
            Sample {
                rgba: u32::from_le_bytes(rgba),
                radius,
                phase,
                padding: 0.0,
            }
        };
        let samples = match old.filter(|old| {
            old.samples.values.len() == count
                && old
                    .samples
                    .values
                    .iter()
                    .enumerate()
                    .all(|(index, old)| *old == sample(index))
        }) {
            Some(old) => old.samples.clone(),
            None => {
                let mut records = RecordBuffer::new(budget, count)?;
                for (index, record) in records.values.iter_mut().enumerate() {
                    *record = sample(index);
                }
                Arc::new(records)
            }
        };
        let on_top = e.flags & draw_flag::ON_TOP != 0;
        let fx = look.fx;
        Ok(Some(Arc::new(Self {
            positions,
            samples,
            center,
            uniform: Uniform {
                unused: Vec4::new(
                    super::super::visual::blur_key(e) as f32 / 255.0,
                    0.0,
                    0.0,
                    0.0,
                ),
                world_from_local: Mat4::IDENTITY,
                fx: Vec4::new(fx.glow, fx.pulse_hz, fx.flow, fx.spacing),
                fx2: Vec4::new(fx.fade, 0.0, fx.urgency, 0.0),
                code: UVec4::new((look.flags & 0xfff) << 2 | (fx.style & 0x3f) << 14, 0, 0, 0),
            },
            opaque: fx.style == draw_style::SPLAT && !on_top,
            on_top,
            _charge: charge(budget, size_of::<Self>() + 2 * size_of::<usize>())?,
        })))
    }

    fn sort_center(&self, world_from_local: Mat4) -> Vec3 {
        world_from_local.transform_point3(self.center)
    }
}

#[derive(Component, Clone)]
pub(in crate::draw) struct Node {
    points: Arc<Points>,
    uniform: Uniform,
    visible: bool,
    _charge: Arc<Charge>,
}

impl SyncComponent for Node {
    type Target = Self;
}
impl ExtractComponent for Node {
    type QueryData = (
        &'static Self,
        &'static GlobalTransform,
        &'static InheritedVisibility,
    );
    type QueryFilter = ();
    type Out = Self;
    fn extract_component(
        (node, transform, visibility): bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>,
    ) -> Option<Self> {
        let mut out = node.clone();
        out.uniform.world_from_local = transform.to_matrix();
        out.visible = visibility.get();
        Some(out)
    }
}

impl super::extract::Budgeted for Node {
    fn denied(
        _: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>,
        _: &gearbox_api::draw_data::DrawingBackend,
    ) {
        tracing::warn!("compact legacy point extraction: CpuBudget");
    }
}

pub(in crate::draw) fn install(app: &mut App) {
    super::extract::install::<Node>(app);
    render::install(app);
}

pub(in crate::draw) fn set(
    commands: &mut Commands,
    slot: &mut Option<Entity>,
    root: Entity,
    points: Option<&Arc<Points>>,
    budget: &CpuBudget,
) -> Result<(), StorageError> {
    let Some(points) = points.filter(|points| !points.positions.values.is_empty()) else {
        if let Some(entity) = slot.take() {
            commands.entity(entity).try_despawn();
        }
        return Ok(());
    };
    let node = Node {
        points: points.clone(),
        uniform: points.uniform.clone(),
        visible: false,
        _charge: Arc::new(charge(budget, 4096)?),
    };
    if let Some(entity) = *slot {
        commands.entity(entity).insert(node);
    } else {
        *slot = Some(
            commands
                .spawn((
                    node,
                    Transform::IDENTITY,
                    GlobalTransform::IDENTITY,
                    Visibility::Inherited,
                    ChildOf(root),
                ))
                .id(),
        );
    }
    Ok(())
}
