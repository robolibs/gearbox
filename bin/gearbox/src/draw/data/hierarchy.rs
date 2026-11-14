use gearbox_api::draw_data::{CpuBudget, StorageError};
use gearbox_api::wire::draw_data::MAX_PAGE_POINTS;

use super::cloud::{Geometry, Records, invalid};

pub(super) const LEAF_POINTS: usize = 256;
const CHECK_INTERVAL: usize = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Node {
    pub bounds_m: [[f64; 3]; 2],
    pub first: u32,
    pub count: u32,
    pub children: Option<[u32; 2]>,
    pub representative: u32,
    pub error_m: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Key {
    morton: u64,
    source_id: u64,
    render_index: u32,
}

impl Key {
    fn byte(self, pass: usize) -> usize {
        let value = if pass < 8 {
            self.source_id
        } else {
            self.morton
        };
        ((value >> ((pass % 8) * 8)) & 255) as usize
    }
}

#[derive(Debug)]
pub(super) struct Hierarchy {
    pub order: Records<u32>,
    pub nodes: Records<Node>,
    pub refinement: super::refinement::Refinement,
}

impl Hierarchy {
    pub fn empty(budget: &CpuBudget) -> Result<Self, StorageError> {
        Ok(Self {
            order: Records::new(budget, 0)?,
            nodes: Records::new(budget, 0)?,
            refinement: super::refinement::Refinement::empty(budget)?,
        })
    }

    pub fn build(
        budget: &CpuBudget,
        geometry: &Geometry,
        check: &impl Fn() -> Result<(), StorageError>,
    ) -> Result<Self, StorageError> {
        check()?;
        let count = geometry.positions.values.len();
        if count > MAX_PAGE_POINTS as usize {
            return Err(invalid("spatial page exceeds point limit"));
        }
        let Some(bounds) = geometry.bounds_m else {
            return Self::empty(budget);
        };
        let mut keys = Records::<Key>::new(budget, count)?;
        let mut scratch = Records::<Key>::new(budget, count)?;
        for (index, key) in keys.values.iter_mut().enumerate() {
            checkpoint(index, check)?;
            *key = Key {
                morton: morton(geometry.source_position(index)?, bounds),
                source_id: geometry
                    .source_id(index)
                    .ok_or_else(|| invalid("source ID missing"))?,
                render_index: index as u32,
            };
        }
        radix(&mut keys.values, &mut scratch.values, check)?;
        drop(scratch);
        let mut result = Self {
            order: Records::new(budget, count)?,
            nodes: Records::new(budget, node_capacity(count))?,
            refinement: super::refinement::Refinement::empty(budget)?,
        };
        for (index, key) in keys.values.iter().enumerate() {
            checkpoint(index, check)?;
            result.order.values[index] = key.render_index;
        }
        drop(keys);
        super::spatial_order::order(geometry, &mut result.order.values, check)?;
        let mut next = 0;
        result.build_node(geometry, 0, count, &mut next, check)?;
        result.nodes.values.truncate(next);
        result.refinement = super::refinement::Refinement::build(
            budget,
            geometry,
            &result.order.values,
            &result.nodes.values,
            check,
        )?;
        check()?;
        Ok(result)
    }

    fn build_node(
        &mut self,
        geometry: &Geometry,
        first: usize,
        count: usize,
        next: &mut usize,
        check: &impl Fn() -> Result<(), StorageError>,
    ) -> Result<u32, StorageError> {
        check()?;
        let index = *next;
        *next += 1;
        let mut node = Node {
            first: first as u32,
            count: count as u32,
            ..Default::default()
        };
        if count <= LEAF_POINTS {
            node.representative = self.order.values[first];
            let point = geometry.source_position(node.representative as usize)?;
            node.bounds_m = [point, point];
            let mut representative_id = geometry.source_id(node.representative as usize).unwrap();
            for &ordinal in &self.order.values[first..first + count] {
                let point = geometry.source_position(ordinal as usize)?;
                for (axis, value) in point.into_iter().enumerate() {
                    node.bounds_m[0][axis] = node.bounds_m[0][axis].min(value);
                    node.bounds_m[1][axis] = node.bounds_m[1][axis].max(value);
                }
                let source_id = geometry.source_id(ordinal as usize).unwrap();
                if source_id < representative_id {
                    representative_id = source_id;
                    node.representative = ordinal;
                }
            }
        } else {
            let left = self.build_node(geometry, first, count / 2, next, check)?;
            let right =
                self.build_node(geometry, first + count / 2, count - count / 2, next, check)?;
            node.children = Some([left, right]);
            let a = self.nodes.values[left as usize];
            let b = self.nodes.values[right as usize];
            for axis in 0..3 {
                node.bounds_m[0][axis] = a.bounds_m[0][axis].min(b.bounds_m[0][axis]);
                node.bounds_m[1][axis] = a.bounds_m[1][axis].max(b.bounds_m[1][axis]);
            }
            node.representative = if geometry.source_id(a.representative as usize)
                < geometry.source_id(b.representative as usize)
            {
                a.representative
            } else {
                b.representative
            };
        }
        node.error_m = (0..3).fold(0.0f64, |distance, axis| {
            let distance = distance.hypot(node.bounds_m[1][axis] - node.bounds_m[0][axis]);
            if distance == 0.0 {
                0.0
            } else {
                distance.next_up()
            }
        });
        self.nodes.values[index] = node;
        Ok(index as u32)
    }
}

fn node_capacity(count: usize) -> usize {
    if count == 0 {
        0
    } else {
        2 * count.div_ceil(LEAF_POINTS).next_power_of_two() - 1
    }
}

fn checkpoint(
    index: usize,
    check: &impl Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    if index % CHECK_INTERVAL == 0 {
        check()?;
    }
    Ok(())
}

fn morton(point: [f64; 3], bounds: [[f64; 3]; 2]) -> u64 {
    let mut result = 0;
    for axis in 0..3 {
        let span = bounds[1][axis] - bounds[0][axis];
        let value = if span == 0.0 {
            0
        } else {
            (((point[axis] - bounds[0][axis]) / span).clamp(0.0, 1.0) * 2_097_151.0) as u64
        };
        for bit in 0..21 {
            result |= ((value >> bit) & 1) << (3 * bit + axis);
        }
    }
    result
}

fn radix(
    keys: &mut [Key],
    scratch: &mut [Key],
    check: &impl Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    for pass in 0..16 {
        check()?;
        let (input, output) = if pass % 2 == 0 {
            (&*keys, &mut *scratch)
        } else {
            (&*scratch, &mut *keys)
        };
        let mut offsets = [0usize; 256];
        for (index, key) in input.iter().enumerate() {
            checkpoint(index, check)?;
            offsets[key.byte(pass)] += 1;
        }
        let mut sum = 0;
        for count in &mut offsets {
            let next = sum + *count;
            *count = sum;
            sum = next;
        }
        for (index, key) in input.iter().enumerate() {
            checkpoint(index, check)?;
            let offset = &mut offsets[key.byte(pass)];
            output[*offset] = *key;
            *offset += 1;
        }
    }
    check()
}

#[cfg(test)]
#[path = "hierarchy_tests.rs"]
mod tests;
