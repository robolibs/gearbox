use std::sync::Arc;

use crate::wire::draw_data::{
    AttributeSemantic, CloudPageView, CloudSchema, ColumnView, DATA_CHUNK_BYTES, DataShape,
    MAX_ATTRIBUTES, OccupancyPageView, OctreeConfig, PageDescriptor, PageEncoding, VoxelKey,
};

use super::memory::{Buffer, Charge, CpuBudget, MemoryClass, Reservation, StorageError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkOutcome {
    Accepted,
    Duplicate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageFootprint {
    pub payload: u64,
    pub upload_index: u64,
    pub metadata: u64,
    pub scratch: u64,
}

impl PageFootprint {
    pub fn total(&self) -> u64 {
        self.payload + self.upload_index + self.metadata + self.scratch
    }

    pub fn for_descriptor(descriptor: &PageDescriptor) -> Result<Self, StorageError> {
        descriptor.validate().map_err(StorageError::Invalid)?;
        let scratch_stride = match descriptor.encoding {
            PageEncoding::CloudColumnsV1 => size_of::<u64>(),
            PageEncoding::OccupancyLeavesV1 => size_of::<VoxelKey>(),
            PageEncoding::MeshBlockV1 | PageEncoding::VolumeBlockV1 | PageEncoding::ElevationTileV1 => 0,
        };
        Ok(Self {
            payload: descriptor.byte_len,
            upload_index: 2 * descriptor.byte_len.div_ceil(8),
            metadata: (size_of::<PageUpload>()
                + size_of::<StoredPage>()
                + size_of::<StagedPage>()
                + 2 * size_of::<usize>()) as u64,
            scratch: u64::from(descriptor.items) * scratch_stride as u64,
        })
    }
}

#[derive(Debug)]
pub struct PageUpload {
    descriptor: PageDescriptor,
    bytes: Buffer<u8>,
    covered: Buffer<u8>,
    starts: Buffer<u8>,
    received: usize,
    reservation: Reservation,
    metadata: Charge,
}

impl PageUpload {
    pub fn new(budget: &CpuBudget, descriptor: PageDescriptor) -> Result<Self, StorageError> {
        let footprint = PageFootprint::for_descriptor(&descriptor)?;
        let reservation = budget.reserve(footprint.total())?;
        Self::from_reservation(descriptor, reservation)
    }

    pub fn from_reservation(
        descriptor: PageDescriptor,
        mut reservation: Reservation,
    ) -> Result<Self, StorageError> {
        let footprint = PageFootprint::for_descriptor(&descriptor)?;
        if reservation.bytes() != footprint.total() {
            return Err(StorageError::Invalid(
                "page reservation must equal its complete footprint".into(),
            ));
        }
        let metadata = reservation.charge(footprint.metadata, MemoryClass::Metadata)?;
        let len = usize::try_from(descriptor.byte_len).map_err(|_| StorageError::Allocation)?;
        let bytes = Buffer::filled(&mut reservation, len, 0, MemoryClass::Page)?;
        let covered = Buffer::filled(
            &mut reservation,
            len.div_ceil(8),
            0,
            MemoryClass::UploadIndex,
        )?;
        let starts = Buffer::filled(
            &mut reservation,
            len.div_ceil(8),
            0,
            MemoryClass::UploadIndex,
        )?;
        Ok(Self {
            descriptor,
            bytes,
            covered,
            starts,
            received: 0,
            reservation,
            metadata,
        })
    }

    pub fn descriptor(&self) -> &PageDescriptor {
        &self.descriptor
    }
    pub fn received_bytes(&self) -> u64 {
        self.received as u64
    }
    pub fn is_complete(&self) -> bool {
        self.received == self.bytes.data.len()
    }

    pub fn push(&mut self, offset: u64, chunk: &[u8]) -> Result<ChunkOutcome, StorageError> {
        let range = chunk_range(self.bytes.data.len(), offset, chunk)?;
        if range.clone().any(|index| bit(&self.covered.data, index)) {
            let exact = range.clone().all(|index| bit(&self.covered.data, index))
                && bit(&self.starts.data, range.start)
                && !(range.start + 1..range.end).any(|index| bit(&self.starts.data, index))
                && (range.end == self.bytes.data.len()
                    || !bit(&self.covered.data, range.end)
                    || bit(&self.starts.data, range.end));
            return if exact && self.bytes.data[range] == *chunk {
                Ok(ChunkOutcome::Duplicate)
            } else {
                Err(StorageError::Conflict)
            };
        }
        self.bytes.data[range.clone()].copy_from_slice(chunk);
        set_bit(&mut self.starts.data, range.start);
        for index in range {
            set_bit(&mut self.covered.data, index);
        }
        self.received += chunk.len();
        Ok(ChunkOutcome::Accepted)
    }

    fn verify(&self, encoding: PageEncoding) -> Result<(), StorageError> {
        if self.descriptor.encoding != encoding {
            return Err(StorageError::Invalid(
                "page encoding does not match dataset kind".into(),
            ));
        }
        if !self.is_complete() {
            return Err(StorageError::Incomplete);
        }
        self.descriptor
            .verify(&self.bytes.data)
            .map_err(StorageError::Invalid)
    }

    pub fn finish_cloud(mut self, schema: &CloudSchema) -> Result<StagedPage, StorageError> {
        self.verify(PageEncoding::CloudColumnsV1)?;
        let mut scratch = Buffer::filled(
            &mut self.reservation,
            self.descriptor.items as usize,
            0u64,
            MemoryClass::Scratch,
        )?;
        let view = CloudPageView::validate(schema, &self.bytes.data, &mut scratch.data)
            .map_err(StorageError::Invalid)?;
        let mut columns = [ColumnSpan::default(); MAX_ATTRIBUTES];
        let mut offset = 16 + 8 * view.columns().len();
        for (span, column) in columns.iter_mut().zip(view.columns()) {
            *span = ColumnSpan {
                offset: offset as u32,
                values: column.values.len() as u32,
                validity: column.validity.len() as u32,
            };
            offset += column.values.len() + column.validity.len();
        }
        let layout = PageLayout::Cloud {
            columns,
            count: view.columns().len(),
            source_id: view.source_id_column(),
        };
        drop(scratch);
        Ok(self.stage(layout))
    }

    pub fn finish_mesh(self) -> Result<StagedPage, StorageError> {
        self.verify(PageEncoding::MeshBlockV1)?;
        let layout = crate::wire::draw_data::mesh::mesh_layout(&self.bytes.data).map_err(StorageError::Invalid)?;
        if layout.triangles() != self.descriptor.items {
            return Err(StorageError::Invalid("mesh block triangle count differs from its descriptor".into()));
        }
        Ok(self.stage(PageLayout::Mesh(layout)))
    }

    pub fn finish_volume(self, field: &crate::wire::draw_data::VolumeField) -> Result<StagedPage, StorageError> {
        self.verify(PageEncoding::VolumeBlockV1)?;
        let layout = crate::wire::draw_data::volume::volume_layout(field, &self.bytes.data).map_err(StorageError::Invalid)?;
        if self.descriptor.items as usize != field.samples() {
            return Err(StorageError::Invalid("volume block sample count differs from its descriptor".into()));
        }
        if crate::wire::draw_data::volume_page_id(layout.key) != Some(self.descriptor.address.page_id) {
            return Err(StorageError::Invalid("volume block key differs from its page id".into()));
        }
        Ok(self.stage(PageLayout::Volume(layout)))
    }

    pub fn finish_elevation(self, grid: &crate::wire::draw_data::ElevationGrid) -> Result<StagedPage, StorageError> {
        self.verify(PageEncoding::ElevationTileV1)?;
        let layout = crate::wire::draw_data::elevation::elevation_layout(grid, &self.bytes.data).map_err(StorageError::Invalid)?;
        if self.descriptor.items as usize != grid.samples() {
            return Err(StorageError::Invalid("elevation tile sample count differs from its descriptor".into()));
        }
        if crate::wire::draw_data::elevation_page_id(layout.key, layout.layer) != Some(self.descriptor.address.page_id) {
            return Err(StorageError::Invalid("elevation tile key or layer differs from its page id".into()));
        }
        Ok(self.stage(PageLayout::Elevation(layout)))
    }

    pub fn finish_occupancy(mut self, config: &OctreeConfig) -> Result<StagedPage, StorageError> {
        self.verify(PageEncoding::OccupancyLeavesV1)?;
        let empty = VoxelKey {
            tile: [0; 3],
            depth: 0,
            morton: 0,
        };
        let mut scratch = Buffer::filled(
            &mut self.reservation,
            self.descriptor.items as usize,
            empty,
            MemoryClass::Scratch,
        )?;
        OccupancyPageView::validate(config, &self.bytes.data, &mut scratch.data)
            .map_err(StorageError::Invalid)?;
        let extent = scratch
            .data
            .iter()
            .fold(None, |extent: Option<PageExtent>, key| {
                let span = LeafSpan::of(key, config);
                let (first, last) = ((span.tile, span.lo), (span.tile, span.hi));
                Some(
                    extent.map_or(PageExtent { first, last }, |extent| PageExtent {
                        first: extent.first.min(first),
                        last: extent.last.max(last),
                    }),
                )
            });
        drop(scratch);
        Ok(self.stage(PageLayout::Occupancy { extent }))
    }

    fn stage(self, layout: PageLayout) -> StagedPage {
        StagedPage {
            page: Arc::new(StoredPage {
                descriptor: self.descriptor,
                bytes: self.bytes,
                layout,
                _metadata: self.metadata,
            }),
            starts: self.starts,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ColumnSpan {
    offset: u32,
    values: u32,
    validity: u32,
}

/// A leaf's cell interval at the octree's finest depth, within its root tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct LeafSpan {
    pub tile: [i64; 3],
    pub lo: u64,
    pub hi: u64,
}

impl LeafSpan {
    pub fn of(key: &VoxelKey, config: &OctreeConfig) -> Self {
        let shift = 3 * u32::from(config.max_depth.saturating_sub(key.depth));
        let lo = key.morton << shift;
        Self {
            tile: key.tile,
            lo,
            hi: lo | ((1u64 << shift) - 1),
        }
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        self.tile == other.tile && self.lo <= other.hi && other.lo <= self.hi
    }
}

/// First and last leaf positions of a page in (tile, cell) order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PageExtent {
    pub first: ([i64; 3], u64),
    pub last: ([i64; 3], u64),
}

impl PageExtent {
    pub fn overlaps(&self, other: &Self) -> bool {
        self.first <= other.last && other.first <= self.last
    }
}

#[derive(Clone, Copy, Debug)]
enum PageLayout {
    Cloud {
        columns: [ColumnSpan; MAX_ATTRIBUTES],
        count: usize,
        source_id: usize,
    },
    Occupancy {
        extent: Option<PageExtent>,
    },
    Mesh(crate::wire::draw_data::mesh::MeshLayout),
    Volume(crate::wire::draw_data::volume::VolumeLayout),
    Elevation(crate::wire::draw_data::elevation::ElevationLayout),
}

#[derive(Debug)]
pub struct StoredPage {
    descriptor: PageDescriptor,
    bytes: Buffer<u8>,
    layout: PageLayout,
    _metadata: Charge,
}

#[derive(Debug)]
pub struct PageMetadata {
    descriptor: PageDescriptor,
    digest: [u8; 32],
    layout: PageLayout,
    charge: Charge,
}

impl PageMetadata {
    pub fn descriptor(&self) -> &PageDescriptor {
        &self.descriptor
    }

    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub(super) fn belongs_to(&self, budget: &CpuBudget) -> bool {
        self.charge.belongs_to(budget)
    }

    pub(super) fn restore(&self, bytes: Buffer<u8>, charge: Charge) -> StoredPage {
        StoredPage {
            descriptor: self.descriptor.clone(),
            bytes,
            layout: self.layout,
            _metadata: charge,
        }
    }
}

impl StoredPage {
    pub fn metadata(&self, budget: &CpuBudget) -> Result<Arc<PageMetadata>, StorageError> {
        if !self.belongs_to(budget) {
            return Err(StorageError::Invalid(
                "page metadata must share its payload budget".into(),
            ));
        }
        let bytes = (size_of::<PageMetadata>() + 2 * size_of::<usize>()) as u64;
        let charge = budget
            .reserve(bytes)?
            .charge(bytes, MemoryClass::Metadata)?;
        Ok(Arc::new(PageMetadata {
            descriptor: self.descriptor.clone(),
            digest: *blake3::hash(self.bytes()).as_bytes(),
            layout: self.layout,
            charge,
        }))
    }

    pub(super) fn belongs_to(&self, budget: &CpuBudget) -> bool {
        self._metadata.belongs_to(budget)
    }

    pub(super) fn validate_shape(&self, shape: &DataShape) -> Result<u64, StorageError> {
        match (shape, &self.layout) {
            (
                DataShape::Cloud { schema } | DataShape::Surfels { schema, .. },
                PageLayout::Cloud {
                    count, source_id, ..
                },
            ) => {
                schema.validate().map_err(StorageError::Invalid)?;
                if schema.attributes.len() != *count
                    || schema.attributes[*source_id].semantic != AttributeSemantic::SourceId
                {
                    return Err(StorageError::Invalid(
                        "stored page does not match cloud schema".into(),
                    ));
                }
                let mut valid = 0;
                for (index, field) in schema.attributes.iter().enumerate() {
                    let column = self.column(index).unwrap();
                    column
                        .validate(field, self.descriptor.items)
                        .map_err(StorageError::Invalid)?;
                    if field.semantic == AttributeSemantic::Position {
                        valid = if column.validity.is_empty() {
                            u64::from(self.descriptor.items)
                        } else {
                            column
                                .validity
                                .iter()
                                .map(|byte| u64::from(byte.count_ones()))
                                .sum()
                        };
                    }
                }
                Ok(valid)
            }
            (DataShape::MeshBlocks { .. }, PageLayout::Mesh(_)) => Ok(u64::from(self.descriptor.items)),
            (DataShape::Volume { .. }, PageLayout::Volume(layout)) => Ok(u64::from(layout.valid_count)),
            (DataShape::Elevation { .. }, PageLayout::Elevation(layout)) => Ok(u64::from(layout.valid_count)),
            (DataShape::Octree { config, .. }, PageLayout::Occupancy { .. }) => {
                for key in self.voxel_keys().unwrap() {
                    key.validate(config).map_err(StorageError::Invalid)?;
                }
                Ok(u64::from(self.descriptor.items))
            }
            _ => Err(StorageError::Invalid(
                "page kind does not match generation".into(),
            )),
        }
    }
    pub fn descriptor(&self) -> &PageDescriptor {
        &self.descriptor
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes.data
    }

    pub(super) fn valid_count(&self, shape: &DataShape) -> u64 {
        if let Some(schema) = shape.columns() {
            let position = schema
                .attributes
                .iter()
                .position(|field| field.semantic == AttributeSemantic::Position)
                .unwrap();
            let column = self.column(position).unwrap();
            if !column.validity.is_empty() {
                return column
                    .validity
                    .iter()
                    .map(|byte| u64::from(byte.count_ones()))
                    .sum();
            }
        }
        u64::from(self.descriptor.items)
    }

    pub fn column(&self, index: usize) -> Option<ColumnView<'_>> {
        let PageLayout::Cloud { columns, count, .. } = &self.layout else {
            return None;
        };
        if index >= *count {
            return None;
        }
        let span = columns[index];
        let start = span.offset as usize;
        let end = start + span.values as usize;
        Some(ColumnView {
            values: &self.bytes.data[start..end],
            validity: &self.bytes.data[end..end + span.validity as usize],
        })
    }

    pub fn source_ids(&self) -> Option<impl ExactSizeIterator<Item = u64> + '_> {
        let PageLayout::Cloud { source_id, .. } = &self.layout else {
            return None;
        };
        Some(
            self.column(*source_id)?
                .values
                .chunks_exact(8)
                .map(|raw| u64::from_le_bytes(raw.try_into().unwrap())),
        )
    }

    /// Cached (tile, cell) extent of an occupancy page; `None` when empty or not occupancy.
    pub(super) fn occupancy_extent(&self) -> Option<PageExtent> {
        match self.layout {
            PageLayout::Occupancy { extent } => extent,
            PageLayout::Cloud { .. } | PageLayout::Mesh(_) | PageLayout::Volume(_) | PageLayout::Elevation(_) => None,
        }
    }

    /// Where a validated mesh block's arrays lie in [`Self::bytes`].
    pub fn mesh_layout(&self) -> Option<crate::wire::draw_data::mesh::MeshLayout> {
        match self.layout {
            PageLayout::Mesh(layout) => Some(layout),
            _ => None,
        }
    }

    /// Where a validated volume block's arrays lie in [`Self::bytes`].
    pub fn volume_layout(&self) -> Option<crate::wire::draw_data::volume::VolumeLayout> {
        match self.layout {
            PageLayout::Volume(layout) => Some(layout),
            _ => None,
        }
    }

    /// Where a validated elevation tile's arrays lie in [`Self::bytes`].
    pub fn elevation_layout(&self) -> Option<crate::wire::draw_data::elevation::ElevationLayout> {
        match self.layout {
            PageLayout::Elevation(layout) => Some(layout),
            _ => None,
        }
    }

    /// Leaves of a validated occupancy page, in stored order.
    pub fn occupancy_leaves(
        &self,
    ) -> Option<impl ExactSizeIterator<Item = crate::wire::draw_data::OccupancyLeaf> + '_> {
        if !matches!(self.layout, PageLayout::Occupancy { .. }) {
            return None;
        }
        Some(
            self.bytes.data[16..]
                .chunks_exact(48)
                .map(|raw| crate::wire::draw_data::pages::occupancy_leaf(raw).unwrap()),
        )
    }

    pub fn voxel_keys(&self) -> Option<impl ExactSizeIterator<Item = VoxelKey> + '_> {
        if !matches!(self.layout, PageLayout::Occupancy { .. }) {
            return None;
        }
        Some(self.bytes.data[16..].chunks_exact(48).map(|raw| VoxelKey {
            tile: std::array::from_fn(|i| {
                i64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap())
            }),
            morton: u64::from_le_bytes(raw[24..32].try_into().unwrap()),
            depth: raw[32],
        }))
    }
}

#[derive(Debug)]
pub struct StagedPage {
    page: Arc<StoredPage>,
    starts: Buffer<u8>,
}

impl StagedPage {
    pub(super) fn shared_page(&self) -> Arc<StoredPage> {
        self.page.clone()
    }
    pub fn page(&self) -> &StoredPage {
        &self.page
    }

    pub fn retry(&self, offset: u64, chunk: &[u8]) -> Result<ChunkOutcome, StorageError> {
        let bytes = self.page.bytes();
        let range = chunk_range(bytes.len(), offset, chunk)?;
        let exact = bit(&self.starts.data, range.start)
            && !(range.start + 1..range.end).any(|index| bit(&self.starts.data, index))
            && (range.end == bytes.len() || bit(&self.starts.data, range.end));
        if exact && bytes[range] == *chunk {
            Ok(ChunkOutcome::Duplicate)
        } else {
            Err(StorageError::Conflict)
        }
    }

    pub fn into_page(self) -> Arc<StoredPage> {
        self.page
    }
}

fn chunk_range(
    len: usize,
    offset: u64,
    chunk: &[u8],
) -> Result<std::ops::Range<usize>, StorageError> {
    let start = usize::try_from(offset)
        .map_err(|_| StorageError::Invalid("chunk offset overflow".into()))?;
    let end = start
        .checked_add(chunk.len())
        .ok_or_else(|| StorageError::Invalid("chunk end overflow".into()))?;
    if chunk.is_empty() || chunk.len() > DATA_CHUNK_BYTES || end > len {
        return Err(StorageError::Invalid(
            "chunk bounds or size are invalid".into(),
        ));
    }
    Ok(start..end)
}

fn bit(bits: &[u8], index: usize) -> bool {
    bits[index / 8] & (1 << (index % 8)) != 0
}
fn set_bit(bits: &mut [u8], index: usize) {
    bits[index / 8] |= 1 << (index % 8);
}
