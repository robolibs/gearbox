use std::sync::Arc;

use gearbox_api::draw_data::{Charge, CpuBudget, StorageError};

use super::appearance::Attributes;
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

/// The radius a coarse representative covers: about three quarters of the spacing
/// between representatives over the page's largest face, so neighbours touch.
pub(super) fn coverage_radius_m(metadata: &super::cloud::SpatialMetadata) -> f32 {
    let count = metadata.coarse.positions.values.len();
    let Some([lo, hi]) = metadata.bounds_m else {
        return 0.0;
    };
    if count == 0 {
        return 0.0;
    }
    let mut extent = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    extent.sort_unstable_by(|a, b| b.total_cmp(a));
    let area = extent[0] * extent[1].max(extent[0] * 1e-3);
    let radius = 0.75 * (area / count as f64).sqrt();
    if radius.is_finite() { radius as f32 } else { 0.0 }
}

pub(super) struct Samples {
    pub attributes: Attributes,
    _charge: Charge,
}

impl Samples {
    pub fn build(
        budget: &CpuBudget,
        representatives: &Representatives,
        attributes: &Attributes,
    ) -> Result<Arc<Self>, StorageError> {
        let charge = super::worker::charge(budget, size_of::<Self>() + 16)?;
        Ok(Arc::new(Self {
            attributes: attributes.gather(budget, &representatives.ordinals.values)?,
            _charge: charge,
        }))
    }
}

#[cfg(test)]
#[path = "coarse_tests.rs"]
mod tests;
