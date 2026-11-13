use std::sync::Arc;

use crate::wire::draw_data::{
    Attribute, ColorMode, DataDescriptor, DataFrame, DataShape, PaletteEntry, ScanDescriptor,
};

use super::memory::{Charge, CpuBudget, MemoryClass, StorageError};

pub(super) const METADATA_SCRATCH: u64 = 64 * 1024;

#[derive(Debug)]
pub struct Metadata<T> {
    value: T,
    charge: Charge,
}

pub type DatasetMetadata = Metadata<DataDescriptor>;
pub type ScanMetadata = Metadata<ScanDescriptor>;

impl<T> Metadata<T> {
    pub fn value(&self) -> &T {
        &self.value
    }
    pub(super) fn belongs_to(&self, budget: &CpuBudget) -> bool {
        self.charge.belongs_to(budget)
    }
}

impl DatasetMetadata {
    pub fn new(budget: &CpuBudget, value: &DataDescriptor) -> Result<Arc<Self>, StorageError> {
        tracked(budget, value, descriptor_heap, DataDescriptor::to_json)
    }
}

impl ScanMetadata {
    pub fn new(budget: &CpuBudget, value: &ScanDescriptor) -> Result<Arc<Self>, StorageError> {
        tracked(budget, value, scan_heap, ScanDescriptor::to_json)
    }
}

fn tracked<T: Clone>(
    budget: &CpuBudget,
    value: &T,
    heap: fn(&T, bool) -> usize,
    validate: fn(&T) -> Result<Vec<u8>, String>,
) -> Result<Arc<Metadata<T>>, StorageError> {
    let _scratch = budget
        .reserve(METADATA_SCRATCH)?
        .charge(METADATA_SCRATCH, MemoryClass::Scratch)?;
    drop(validate(value).map_err(StorageError::Invalid)?);
    let payload = heap(value, false);
    let bytes = (size_of::<Metadata<T>>() + 2 * size_of::<usize>())
        .checked_add(payload)
        .ok_or(StorageError::Allocation)? as u64;
    let charge = budget
        .reserve(bytes)?
        .charge(bytes, MemoryClass::Metadata)?;
    let value = value.clone();
    if heap(&value, true) > payload {
        return Err(StorageError::Allocation);
    }
    Ok(Arc::new(Metadata { value, charge }))
}

fn string(value: &String, capacity: bool) -> usize {
    if capacity {
        value.capacity()
    } else {
        value.len()
    }
}

fn optional(value: &Option<String>, capacity: bool) -> usize {
    value.as_ref().map_or(0, |value| string(value, capacity))
}

fn frame_heap(frame: &DataFrame, capacity: bool) -> usize {
    string(&frame.frame, capacity)
        + frame
            .capture
            .as_ref()
            .map_or(0, |stamp| optional(&stamp.source_clock, capacity))
        + frame.capture_transform.as_ref().map_or(0, |pose| {
            string(&pose.source_frame, capacity)
                + string(&pose.target_frame, capacity)
                + optional(&pose.stamp.source_clock, capacity)
        })
}

fn descriptor_heap(value: &DataDescriptor, capacity: bool) -> usize {
    let shape = match &value.shape {
        DataShape::Cloud { schema } => {
            let count = if capacity {
                schema.attributes.capacity()
            } else {
                schema.attributes.len()
            };
            count * size_of::<Attribute>()
                + schema
                    .attributes
                    .iter()
                    .map(|field| string(&field.name, capacity) + string(&field.units, capacity))
                    .sum::<usize>()
        }
        DataShape::Octree { frame, .. } | DataShape::MeshBlocks { frame } => frame_heap(frame, capacity),
        DataShape::Volume { field, frame } => {
            frame_heap(frame, capacity)
                + match &field.kind {
                    crate::wire::draw_data::FieldKind::Scalar { units, .. } => string(units, capacity),
                    _ => 0,
                }
        }
        DataShape::Elevation { grid, frame } => {
            frame_heap(frame, capacity)
                + grid.layers.len() * size_of::<crate::wire::draw_data::ElevationLayer>()
                + grid.layers.iter().map(|layer| string(&layer.name, capacity) + string(&layer.units, capacity)).sum::<usize>()
        }
        DataShape::Surfels { schema, frame } => {
            frame_heap(frame, capacity)
                + schema.attributes.len() * size_of::<Attribute>()
                + schema
                    .attributes
                    .iter()
                    .map(|field| string(&field.name, capacity) + string(&field.units, capacity))
                    .sum::<usize>()
        }
    };
    let color = match &value.presentation.color {
        ColorMode::Attribute { name, .. } | ColorMode::Scalar { name, .. } => {
            string(name, capacity)
        }
        ColorMode::Category { name, entries, .. } => {
            string(name, capacity)
                + size_of::<PaletteEntry>()
                    * if capacity {
                        entries.capacity()
                    } else {
                        entries.len()
                    }
        }
        _ => 0,
    };
    string(&value.key.namespace, capacity)
        + string(&value.key.id, capacity)
        + shape
        + color
        + optional(&value.presentation.radius_attribute, capacity)
}

fn scan_heap(value: &ScanDescriptor, capacity: bool) -> usize {
    optional(&value.source, capacity) + frame_heap(&value.frame, capacity)
}
