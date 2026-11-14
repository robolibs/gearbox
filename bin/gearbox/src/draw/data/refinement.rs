use gearbox_api::draw_data::{CpuBudget, StorageError};

use super::cloud::{Geometry, Records};
use super::hierarchy::{LEAF_POINTS, Node};

pub(super) const LEVELS: usize = 8;

#[derive(Debug)]
pub(super) struct Refinement {
    pub order: Records<u32>,
    pub errors_m: Records<[f64; LEVELS]>,
}

#[derive(Clone, Copy)]
struct Scratch {
    points: [[f64; 3]; LEAF_POINTS],
    ids: [u64; LEAF_POINTS],
    groups: [(u16, u16); LEAF_POINTS],
    next: [(u16, u16); LEAF_POINTS],
    selected: [bool; LEAF_POINTS],
}

impl Default for Scratch {
    fn default() -> Self {
        Self {
            points: [[0.0; 3]; LEAF_POINTS],
            ids: [0; LEAF_POINTS],
            groups: [(0, 0); LEAF_POINTS],
            next: [(0, 0); LEAF_POINTS],
            selected: [false; LEAF_POINTS],
        }
    }
}

impl Refinement {
    pub fn empty(budget: &CpuBudget) -> Result<Self, StorageError> {
        Ok(Self {
            order: Records::new(budget, 0)?,
            errors_m: Records::new(budget, 0)?,
        })
    }

    pub fn build(
        budget: &CpuBudget,
        geometry: &Geometry,
        order: &[u32],
        nodes: &[Node],
        check: &impl Fn() -> Result<(), StorageError>,
    ) -> Result<Self, StorageError> {
        let mut result = Self {
            order: Records::new(budget, order.len())?,
            errors_m: Records::new(budget, nodes.len())?,
        };
        let mut scratch = Records::<Scratch>::new(budget, usize::from(!order.is_empty()))?;
        for (index, node) in nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.children.is_none())
        {
            check()?;
            let range = node.first as usize..(node.first + node.count) as usize;
            let source = &order[range.clone()];
            let output = &mut result.order.values[range];
            result.errors_m.values[index] =
                scratch.values[0].build(geometry, source, output, check)?;
        }
        Ok(result)
    }
}

impl Scratch {
    fn build(
        &mut self,
        geometry: &Geometry,
        source: &[u32],
        output: &mut [u32],
        check: &impl Fn() -> Result<(), StorageError>,
    ) -> Result<[f64; LEVELS], StorageError> {
        self.selected.fill(false);
        for (index, ordinal) in source.iter().copied().enumerate() {
            self.points[index] = geometry.source_position(ordinal as usize)?;
            self.ids[index] = geometry.source_id(ordinal as usize).unwrap();
        }
        self.groups[0] = (0, source.len() as u16);
        let mut groups = 1;
        let mut written = 0;
        let mut previous = f64::INFINITY;
        let mut errors = [0.0; LEVELS];
        for level in 0..=LEVELS {
            check()?;
            let mut error: f64 = 0.0;
            let mut next = 0;
            for &(first, end) in &self.groups[..groups] {
                let range = usize::from(first)..usize::from(end);
                let representative = range.clone().min_by_key(|index| self.ids[*index]).unwrap();
                if !self.selected[representative] {
                    self.selected[representative] = true;
                    output[written] = source[representative];
                    written += 1;
                }
                for point in &self.points[range] {
                    let distance = (0..3).fold(0.0f64, |distance, axis| {
                        distance
                            .hypot(
                                (point[axis] - self.points[representative][axis])
                                    .abs()
                                    .next_up(),
                            )
                            .next_up()
                    });
                    error = error.max(distance);
                }
                if end - first > 1 {
                    let middle = first + (end - first) / 2;
                    self.next[next] = (first, middle);
                    self.next[next + 1] = (middle, end);
                    next += 2;
                } else {
                    self.next[next] = (first, end);
                    next += 1;
                }
            }
            debug_assert_eq!(written, groups);
            if written == source.len() {
                break;
            }
            previous = previous.min(error);
            errors[level] = previous;
            std::mem::swap(&mut self.groups, &mut self.next);
            groups = next;
        }
        debug_assert_eq!(written, source.len());
        Ok(errors)
    }
}

#[cfg(test)]
#[path = "refinement_tests.rs"]
mod tests;
