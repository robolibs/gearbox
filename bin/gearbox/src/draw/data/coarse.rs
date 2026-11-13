use std::sync::Arc;

use gearbox_api::draw_data::{Charge, CpuBudget, StorageError};

use super::appearance::Sample;
use super::cloud::{Geometry, Position, Records, invalid};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Tier {
    Coarse,
    #[default]
    Full,
}

#[derive(Debug)]
pub(super) struct Representatives {
    pub ordinals: Records<u32>,
    pub positions: Records<Position>,
    pub source_ids: Records<u64>,
}

impl Representatives {
    pub fn empty(budget: &CpuBudget) -> Result<Self, StorageError> {
        Ok(Self {
            ordinals: Records::new(budget, 0)?,
            positions: Records::new(budget, 0)?,
            source_ids: Records::new(budget, 0)?,
        })
    }

    pub fn build(
        budget: &CpuBudget,
        geometry: &Geometry,
        check: &impl Fn() -> Result<(), StorageError>,
    ) -> Result<Self, StorageError> {
        let nodes = &geometry.hierarchy.nodes.values;
        let mut ordinals = Records::new(budget, nodes.len())?;
        for (index, node) in nodes.iter().enumerate() {
            check()?;
            ordinals.values[index] = node.representative;
        }
        ordinals.values.sort_unstable();
        ordinals.values.dedup();
        let mut positions = Records::new(budget, ordinals.values.len())?;
        let mut source_ids = Records::new(budget, ordinals.values.len())?;
        for (index, ordinal) in ordinals.values.iter().copied().enumerate() {
            check()?;
            positions.values[index] = *geometry
                .positions
                .values
                .get(ordinal as usize)
                .ok_or_else(|| invalid("coarse representative outside source page"))?;
            source_ids.values[index] = geometry
                .source_id(ordinal as usize)
                .ok_or_else(|| invalid("coarse representative has no source ID"))?;
        }
        Ok(Self {
            ordinals,
            positions,
            source_ids,
        })
    }

    pub fn index(&self, ordinal: u32) -> Option<u32> {
        self.ordinals
            .values
            .binary_search(&ordinal)
            .ok()
            .map(|i| i as u32)
    }
}

pub(super) struct Samples {
    pub records: Records<Sample>,
    _charge: Charge,
}

impl Samples {
    pub fn build(
        budget: &CpuBudget,
        representatives: &Representatives,
        samples: &[Sample],
    ) -> Result<Arc<Self>, StorageError> {
        let charge = super::worker::charge(budget, size_of::<Self>() + 16)?;
        let mut records = Records::new(budget, representatives.ordinals.values.len())?;
        for (index, ordinal) in representatives.ordinals.values.iter().copied().enumerate() {
            records.values[index] = *samples
                .get(ordinal as usize)
                .ok_or_else(|| invalid("coarse attribute outside source page"))?;
        }
        Ok(Arc::new(Self {
            records,
            _charge: charge,
        }))
    }
}

#[cfg(test)]
#[path = "coarse_tests.rs"]
mod tests;
