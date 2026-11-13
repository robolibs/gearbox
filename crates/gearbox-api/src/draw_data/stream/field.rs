use super::{StreamingStore, failure};
use crate::draw_data::TransactionError;
use crate::wire::draw_data::volume::{BlockView, Volume};
use crate::wire::draw_data::*;

impl StreamingStore {
    /// Reads a retained volume at volume-frame points without touching the renderer.
    pub(super) fn field(&self, request: &DataRequest, query: &FieldQuery) -> Result<DataReply, TransactionError> {
        let key = request.data_key().map_err(|error| failure(data_code::INVALID, error))?;
        let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        let entry = registry
            .get(&(key.namespace.clone(), key.id.clone()))
            .ok_or_else(|| failure(data_code::NOT_FOUND, "dataset is not retained"))?;
        if query.generation.is_some_and(|generation| generation != entry.revision.generation) {
            return Err(failure(data_code::SUPERSEDED, "the requested generation is no longer current"));
        }
        let DataShape::Volume { field, .. } = &entry.payload.descriptor().shape else {
            return Err(failure(data_code::INVALID, "field queries need a volume dataset"));
        };
        if query.channel == Channel::Weight && !field.weights() {
            return Err(failure(data_code::INVALID, "only TSDF volumes carry weights"));
        }
        let generation = entry
            .payload
            .resident()
            .ok_or_else(|| failure(data_code::UNSUPPORTED, "volume is not resident"))?;
        let blocks = generation
            .pages()
            .iter()
            .filter_map(|page| page.volume_layout().map(|layout| BlockView::trusted(page.bytes(), layout)));
        let volume = Volume::new(field, blocks);
        let answer = FieldAnswer {
            revision: entry.revision,
            field: field.clone(),
            spacing_m: field.voxel_m,
            interpolation: query.interpolation,
            samples: query
                .points
                .iter()
                .map(|p| volume.sample(*p, query.channel, query.interpolation, query.gradient))
                .collect(),
        };
        Ok(DataReply {
            metadata: serde_json::to_vec(&answer).map_err(|error| failure(data_code::INTERNAL, error.to_string()))?,
            stage: data_stage::RETAINED,
            incarnation: entry.revision.incarnation,
            stored_generation: entry.revision.generation,
            ..DataReply::new(self.epoch)
        })
    }
}
