use std::sync::Arc;

use crate::wire::draw_data::{
    AgeClock, DataDescriptor, DataShape, MAX_DECLARED_PAGES, MAX_SCANS, OctreeConfig, PageAddress,
    Presentation, StreamMode, VoxelKey,
};

use super::memory::{Buffer, Charge, CpuBudget, MemoryClass, StorageError};
use super::metadata::{DatasetMetadata, METADATA_SCRATCH, ScanMetadata};
use super::page::{LeafSpan, PageExtent, StoredPage};

mod retention;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RetentionCounts {
    pub evicted: u64,
    pub dropped: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GenerationCounts {
    pub scans: u32,
    pub pages: u32,
    pub source_samples: u64,
    pub valid_samples: u64,
    pub payload_bytes: u64,
}

#[derive(Debug)]
pub struct Generation {
    descriptor: Arc<DatasetMetadata>,
    scans: Buffer<Arc<ScanMetadata>>,
    pages: Buffer<Arc<StoredPage>>,
    counts: GenerationCounts,
    scan_high_watermark: u64,
    retained_at_s: f64,
    clock_anchor: Option<Arc<ScanMetadata>>,
    oldest_simulation_s: Option<f64>,
    _charge: Charge,
}

#[cfg(test)]
impl Drop for Generation {
    fn drop(&mut self) {
        super::concurrency_tests::checkpoint("generation_drop");
    }
}

impl Generation {
    pub(super) fn reservation_bytes(
        scans: usize,
        pages: usize,
        samples: u64,
        shape: &DataShape,
    ) -> Result<u64, StorageError> {
        if scans > 2 * MAX_SCANS as usize || pages > 2 * MAX_DECLARED_PAGES as usize {
            return Err(invalid("generation table exceeds its hard count limit"));
        }
        let stride = match shape {
            DataShape::Cloud { .. } | DataShape::Surfels { .. } => size_of::<[u64; 2]>(),
            DataShape::Octree { .. } => size_of::<VoxelKey>(),
            DataShape::MeshBlocks { .. } | DataShape::Volume { .. } | DataShape::Elevation { .. } => 0,
        };
        let metadata = (size_of::<Self>()
            + 2 * size_of::<usize>()
            + scans * size_of::<Arc<ScanMetadata>>()
            + pages * size_of::<Arc<StoredPage>>()) as u64;
        samples
            .checked_mul(stride as u64)
            .and_then(|bytes| bytes.checked_add(metadata + METADATA_SCRATCH))
            .ok_or(StorageError::Allocation)
    }

    pub fn build(
        budget: &CpuBudget,
        descriptor: Arc<DatasetMetadata>,
        scans: &[Arc<ScanMetadata>],
        pages: &[Arc<StoredPage>],
    ) -> Result<Arc<Self>, StorageError> {
        if scans.len() > MAX_SCANS as usize || pages.len() > MAX_DECLARED_PAGES as usize {
            return Err(invalid("generation table exceeds its hard count limit"));
        }
        let samples = pages
            .iter()
            .map(|page| u64::from(page.descriptor().items))
            .sum();
        let reservation = budget.reserve(Self::reservation_bytes(
            scans.len(),
            pages.len(),
            samples,
            &descriptor.value().shape,
        )?)?;
        Self::from_reservation(
            budget,
            descriptor,
            scans,
            pages,
            0,
            false,
            None,
            reservation,
        )
    }

    /// `fresh` lists the sorted addresses a delta added; other pages come from a validated base.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_reservation(
        budget: &CpuBudget,
        descriptor: Arc<DatasetMetadata>,
        scans: &[Arc<ScanMetadata>],
        pages: &[Arc<StoredPage>],
        scan_high_watermark: u64,
        accumulation_candidate: bool,
        fresh: Option<&[PageAddress]>,
        mut reservation: super::Reservation,
    ) -> Result<Arc<Self>, StorageError> {
        let multiple = if accumulation_candidate { 2 } else { 1 };
        if scans.len() > multiple * MAX_SCANS as usize
            || pages.len() > multiple * MAX_DECLARED_PAGES as usize
        {
            return Err(invalid("generation table exceeds its hard count limit"));
        }
        if !descriptor.belongs_to(budget)
            || scans.iter().any(|scan| !scan.belongs_to(budget))
            || pages.iter().any(|page| !page.belongs_to(budget))
        {
            return Err(invalid("generation resources must share one CPU budget"));
        }
        let samples: u64 = pages
            .iter()
            .map(|page| u64::from(page.descriptor().items))
            .sum();
        let metadata = (size_of::<Self>() + 2 * size_of::<usize>()) as u64;
        let total = Self::reservation_bytes(scans.len(), pages.len(), samples, &descriptor.value().shape)?;
        if !reservation.belongs_to(budget) || reservation.bytes() < total {
            return Err(invalid(
                "generation construction reservation is missing or belongs to another budget",
            ));
        }
        let charge = reservation.charge(metadata, MemoryClass::Metadata)?;
        let _validation = reservation.charge(METADATA_SCRATCH, MemoryClass::Scratch)?;
        let mut scans = Buffer::cloned(&mut reservation, scans, MemoryClass::Metadata)?;
        let mut pages = Buffer::cloned(&mut reservation, pages, MemoryClass::Metadata)?;
        scans
            .data
            .sort_unstable_by_key(|scan| scan.value().scan_key);
        pages
            .data
            .sort_unstable_by_key(|page| page.descriptor().address);
        if pages
            .data
            .windows(2)
            .any(|pair| pair[0].descriptor().address == pair[1].descriptor().address)
        {
            return Err(invalid("duplicate page address in generation"));
        }
        validate_scans(
            descriptor.value(),
            &scans.data,
            &pages.data,
            accumulation_candidate,
        )?;
        let mut counts = GenerationCounts {
            scans: scans.data.len() as u32,
            pages: pages.data.len() as u32,
            source_samples: samples,
            ..Default::default()
        };
        let inherited = |page: &StoredPage| {
            fresh.is_some_and(|fresh| fresh.binary_search(&page.descriptor().address).is_err())
        };
        for page in &pages.data {
            counts.valid_samples += if inherited(page) && page.voxel_keys().is_some() {
                u64::from(page.descriptor().items)
            } else {
                page.validate_shape(&descriptor.value().shape)?
            };
            counts.payload_bytes += page.descriptor().byte_len;
        }
        if let StreamMode::Accumulate { max_bytes, .. } = descriptor.value().stream {
            if !accumulation_candidate && counts.payload_bytes > max_bytes {
                return Err(invalid(
                    "accumulated generation exceeds its payload byte limit",
                ));
            }
        }
        let len = usize::try_from(samples).map_err(|_| StorageError::Allocation)?;
        if descriptor.value().shape.columns().is_some() {
            let mut ids = Buffer::filled(&mut reservation, len, [0u64; 2], MemoryClass::Scratch)?;
            let mut cursor = 0;
            for page in &pages.data {
                for id in page.source_ids().unwrap() {
                    ids.data[cursor] = [page.descriptor().address.scan_key, id];
                    cursor += 1;
                }
            }
            #[cfg(test)]
            super::concurrency_tests::checkpoint("generation_validation");
            ids.data.sort_unstable();
            if ids.data.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(invalid("duplicate source ID across pages of one scan"));
            }
        } else if let DataShape::Octree { config, .. } = &descriptor.value().shape {
            validate_occupancy(budget, &mut reservation, config, &pages.data, fresh, len)?;
        }
        let scan_high_watermark =
            scan_high_watermark.max(scans.data.last().map_or(0, |scan| scan.value().scan_key));
        let mut generation = Self {
            descriptor,
            scans,
            pages,
            counts,
            scan_high_watermark,
            retained_at_s: 0.0,
            clock_anchor: None,
            oldest_simulation_s: None,
            _charge: charge,
        };
        generation.set_accumulation_clock(None)?;
        generation.refresh_oldest_capture();
        Ok(Arc::new(generation))
    }

    pub fn descriptor(&self) -> &DataDescriptor {
        self.descriptor.value()
    }
    pub fn metadata(&self) -> &Arc<DatasetMetadata> {
        &self.descriptor
    }
    pub fn scans(&self) -> &[Arc<ScanMetadata>] {
        &self.scans.data
    }
    pub fn pages(&self) -> &[Arc<StoredPage>] {
        &self.pages.data
    }
    pub fn counts(&self) -> GenerationCounts {
        self.counts
    }

    pub fn scan_high_watermark(&self) -> u64 {
        self.scan_high_watermark
    }
    pub fn accumulation_clock_anchor(&self) -> Option<&ScanMetadata> {
        self.clock_anchor.as_deref()
    }
    pub fn retained_at_s(&self) -> f64 {
        self.retained_at_s
    }
    pub(super) fn stamp_retention(&mut self, now_s: f64) {
        self.retained_at_s = now_s;
    }

    pub fn restyle(
        &self,
        budget: &CpuBudget,
        presentation: &Presentation,
    ) -> Result<Arc<Self>, StorageError> {
        if !self.descriptor.belongs_to(budget) {
            return Err(invalid("presentation budget differs from generation"));
        }
        let bytes = (size_of::<Self>()
            + 2 * size_of::<usize>()
            + self.scans.data.len() * size_of::<Arc<ScanMetadata>>()
            + self.pages.data.len() * size_of::<Arc<StoredPage>>()) as u64;
        let mut reservation = budget.reserve(bytes + METADATA_SCRATCH)?;
        let _scratch = reservation.charge(METADATA_SCRATCH, MemoryClass::Scratch)?;
        let mut descriptor = self.descriptor.value().clone();
        descriptor.presentation = presentation.clone();
        descriptor.validate().map_err(StorageError::Invalid)?;
        for scan in &self.scans.data {
            descriptor
                .validate_scan(scan.value())
                .map_err(StorageError::Invalid)?;
        }
        let descriptor = DatasetMetadata::new(budget, &descriptor)?;
        let scans = Buffer::cloned(&mut reservation, &self.scans.data, MemoryClass::Metadata)?;
        let pages = Buffer::cloned(&mut reservation, &self.pages.data, MemoryClass::Metadata)?;
        let charge = reservation.charge(reservation.bytes(), MemoryClass::Metadata)?;
        Ok(Arc::new(Self {
            descriptor,
            scans,
            pages,
            counts: self.counts,
            scan_high_watermark: self.scan_high_watermark,
            retained_at_s: self.retained_at_s,
            clock_anchor: self.clock_anchor.clone(),
            oldest_simulation_s: self.oldest_simulation_s,
            _charge: charge,
        }))
    }

    pub fn page(&self, address: PageAddress) -> Option<&Arc<StoredPage>> {
        self.pages
            .data
            .binary_search_by_key(&address, |page| page.descriptor().address)
            .ok()
            .map(|index| &self.pages.data[index])
    }
}

fn validate_scans(
    descriptor: &DataDescriptor,
    scans: &[Arc<ScanMetadata>],
    pages: &[Arc<StoredPage>],
    accumulation_candidate: bool,
) -> Result<(), StorageError> {
    if !matches!(descriptor.shape, DataShape::Cloud { .. }) {
        return if scans.is_empty() {
            Ok(())
        } else {
            Err(invalid("only cloud generations own scans"))
        };
    }
    let limit = match descriptor.stream {
        StreamMode::Accumulate { .. } if accumulation_candidate => 2 * MAX_SCANS,
        StreamMode::Accumulate { max_scans, .. } => max_scans,
        _ => 1,
    };
    if scans.len() > limit as usize
        || scans
            .windows(2)
            .any(|pair| pair[0].value().scan_key == pair[1].value().scan_key)
    {
        return Err(invalid("duplicate scans or stream scan count exceeded"));
    }
    let mut page_index = 0;
    let mut epoch = None;
    let mut source_clock = None;
    for scan in scans {
        let scan = scan.value();
        descriptor
            .validate_scan(scan)
            .map_err(StorageError::Invalid)?;
        if let StreamMode::Accumulate { age_clock, .. } = descriptor.stream {
            let capture = scan.frame.capture.as_ref().unwrap();
            if epoch.is_some_and(|previous| previous != capture.scene_epoch) {
                return Err(invalid("accumulated scans must share a scene epoch"));
            }
            epoch = Some(capture.scene_epoch);
            if age_clock == AgeClock::Source {
                let clock = capture.source_clock.as_deref().unwrap();
                if source_clock.is_some_and(|previous| previous != clock) {
                    return Err(invalid("accumulated scans must share a source clock"));
                }
                source_clock = Some(clock);
            }
        }
        let mut samples = 0u64;
        while let Some(page) = pages.get(page_index) {
            if page.descriptor().address.scan_key != scan.scan_key {
                break;
            }
            samples += u64::from(page.descriptor().items);
            page_index += 1;
        }
        if samples != scan.source_count {
            return Err(invalid("scan source count does not match its pages"));
        }
    }
    if page_index != pages.len() {
        return Err(invalid("cloud page references an undeclared scan"));
    }
    Ok(())
}

fn invalid(message: &str) -> StorageError {
    StorageError::Invalid(message.into())
}

const _: () = assert!(size_of::<LeafSpan>() <= size_of::<VoxelKey>());

/// Rejects overlapping leaves. With `fresh`, only pages a delta added and pages whose
/// extent intersects them are compared; inherited pages were disjoint in the base.
fn validate_occupancy(
    budget: &CpuBudget,
    reservation: &mut super::Reservation,
    config: &OctreeConfig,
    pages: &[Arc<StoredPage>],
    fresh: Option<&[PageAddress]>,
    len: usize,
) -> Result<(), StorageError> {
    let is_fresh = |page: &StoredPage| {
        fresh.is_none_or(|fresh| fresh.binary_search(&page.descriptor().address).is_ok())
    };
    let count = pages.iter().filter(|page| is_fresh(page)).count();
    let mut touched = budget.reserve((count * size_of::<PageExtent>()) as u64)?;
    let mut merged: Buffer<PageExtent> =
        Buffer::with_capacity(&mut touched, count, MemoryClass::Scratch)?;
    for page in pages.iter().filter(|page| is_fresh(page)) {
        if let Some(extent) = page.occupancy_extent() {
            merged.data.push(extent);
        }
    }
    merged.data.sort_unstable_by_key(|extent| extent.first);
    let mut write = 0;
    for read in 0..merged.data.len() {
        let extent = merged.data[read];
        if write > 0 && merged.data[write - 1].last >= extent.first {
            let last = &mut merged.data[write - 1].last;
            *last = (*last).max(extent.last);
        } else {
            merged.data[write] = extent;
            write += 1;
        }
    }
    merged.data.truncate(write);
    let affected = |page: &StoredPage| {
        is_fresh(page)
            || page.occupancy_extent().is_some_and(|extent| {
                let index = merged
                    .data
                    .partition_point(|union| union.first <= extent.last);
                index > 0 && merged.data[index - 1].overlaps(&extent)
            })
    };
    let mut spans = Buffer::filled(reservation, len, LeafSpan::default(), MemoryClass::Scratch)?;
    let mut cursor = 0;
    for page in pages.iter().filter(|page| affected(page)) {
        for key in page.voxel_keys().unwrap() {
            spans.data[cursor] = LeafSpan::of(&key, config);
            cursor += 1;
        }
    }
    spans.data.truncate(cursor);
    #[cfg(test)]
    super::concurrency_tests::checkpoint("generation_validation");
    spans
        .data
        .sort_unstable_by(|a, b| (a.tile, a.lo).cmp(&(b.tile, b.lo)).then(b.hi.cmp(&a.hi)));
    if spans.data.windows(2).any(|pair| pair[0].overlaps(&pair[1])) {
        return Err(invalid(
            "overlapping occupancy leaves across generation pages",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod occupancy_tests {
    use super::*;
    use crate::wire::draw_data::{
        OccupancyLeaf, OccupancyValue, PageDescriptor, PageEncoding, encode_occupancy_page,
        page_crc32,
    };

    fn config() -> OctreeConfig {
        OctreeConfig {
            leaf_resolution_m: 0.1,
            max_depth: 4,
            free_max: 0.3,
            occupied_min: 0.7,
        }
    }

    fn page(budget: &CpuBudget, id: u64, keys: &[VoxelKey]) -> Arc<StoredPage> {
        let leaves: Vec<_> = keys
            .iter()
            .map(|key| OccupancyLeaf {
                key: *key,
                value: OccupancyValue {
                    probability: Some(0.9),
                    confidence: None,
                    class_id: None,
                },
            })
            .collect();
        let bytes = encode_occupancy_page(&config(), &leaves).unwrap();
        let descriptor = PageDescriptor {
            address: PageAddress {
                scan_key: 0,
                page_id: id,
            },
            encoding: PageEncoding::OccupancyLeavesV1,
            items: leaves.len() as u32,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [0.0; 3],
        };
        let mut upload = super::super::PageUpload::new(budget, descriptor).unwrap();
        upload.push(0, &bytes).unwrap();
        upload.finish_occupancy(&config()).unwrap().into_page()
    }

    fn key(tile: [i64; 3], depth: u8, morton: u64) -> VoxelKey {
        VoxelKey {
            tile,
            depth,
            morton,
        }
    }

    fn check(
        budget: &CpuBudget,
        pages: &[Arc<StoredPage>],
        fresh: Option<&[PageAddress]>,
    ) -> Result<(), StorageError> {
        let len = pages
            .iter()
            .map(|page| page.descriptor().items as usize)
            .sum();
        let mut reservation = budget
            .reserve((len * size_of::<VoxelKey>()) as u64)
            .unwrap();
        validate_occupancy(budget, &mut reservation, &config(), pages, fresh, len)
    }

    #[test]
    fn delta_validation_compares_only_pages_touching_the_fresh_extent() {
        let budget = CpuBudget::new(1 << 20);
        let parent = page(&budget, 1, &[key([0, 0, 0], 1, 3)]);
        let child = page(&budget, 2, &[key([0, 0, 0], 2, 3 << 3)]);
        let elsewhere = page(&budget, 3, &[key([5, 0, 0], 3, 7), key([5, 0, 0], 3, 8)]);
        let pages = [parent, child, elsewhere];
        let address = |id| PageAddress {
            scan_key: 0,
            page_id: id,
        };
        assert!(check(&budget, &pages, None).is_err());
        assert!(check(&budget, &pages, Some(&[address(3)])).is_ok());
        assert!(check(&budget, &pages, Some(&[address(2)])).is_err());
        let sibling = page(&budget, 4, &[key([5, 0, 0], 2, 0)]);
        let pages = [pages[2].clone(), sibling];
        assert!(check(&budget, &pages, Some(&[address(4)])).is_err());
        assert_eq!(budget.usage().scratch_bytes, 0);
    }

    #[test]
    fn spans_nest_at_every_depth_and_separate_adjacent_cells() {
        let config = OctreeConfig {
            max_depth: 21,
            ..config()
        };
        let root = LeafSpan::of(&key([-1, 2, 0], 0, 0), &config);
        assert_eq!((root.lo, root.hi), (0, u64::MAX >> 1));
        for depth in 1..=21u8 {
            let first = LeafSpan::of(&key([-1, 2, 0], depth, 0), &config);
            let next = LeafSpan::of(&key([-1, 2, 0], depth, 1), &config);
            assert!(root.overlaps(&first) && first.hi + 1 == next.lo && !first.overlaps(&next));
        }
        let other_tile = LeafSpan::of(&key([0, 2, 0], 0, 0), &config);
        assert!(!root.overlaps(&other_tile));
    }
}
