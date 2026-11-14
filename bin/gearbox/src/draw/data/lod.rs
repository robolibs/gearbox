use bevy::math::DMat4;

use super::cloud::SpatialMetadata;

#[path = "lod_projection.rs"]
mod projection;
pub(super) use projection::{Glyph, View};

impl View {
    /// Screen error of a page's root bounds, or `None` when the view cannot see the page.
    pub(super) fn page_error(
        self,
        metadata: &SpatialMetadata,
        world: DMat4,
        glyph: Glyph,
    ) -> Option<f64> {
        let root = metadata.hierarchy.nodes.values.first()?;
        let projection = self.project(
            world,
            metadata.rebase_m,
            root.bounds_m,
            metadata.max_error_m,
            glyph,
        );
        projection.visible.then(|| projection.error())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct PageKey {
    pub incarnation: u64,
    pub scan: u64,
    pub page: u64,
}
