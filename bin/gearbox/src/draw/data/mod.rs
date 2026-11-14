mod admission;
mod age;
mod alpha;
mod appearance;
mod clip;
mod clock;
mod cloud;
mod coarse;
mod extract;
mod frontier;
mod hierarchy;
mod lod;
mod octree;
mod surface;
mod volume;
mod elevation;
pub(crate) mod pick;
pub(in crate::draw) mod legacy;
mod owned;
mod palette;
#[cfg(feature = "draw-probes")]
mod probe;
mod receipts;
mod refinement;
mod spatial_order;
mod render;
mod retirement;
pub(super) mod runtime;
mod style;
mod tree;
mod upload;
mod upload_window;
mod worker;

#[derive(bevy::prelude::SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct DataSystems;

pub(super) struct DataPlugin;

impl bevy::prelude::Plugin for DataPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        #[cfg(feature = "draw-probes")]
        probe::install(app);
        runtime::install(app);
        render::install(app);
        legacy::install(app);
        octree::install(app);
        pick::install(app);
        upload::install(app);
        receipts::install(app);
    }
}

#[cfg(test)]
mod color_tests;
#[cfg(test)]
mod gpu_tests;
#[cfg(test)]
mod test_allocations;
#[cfg(test)]
mod tests;
