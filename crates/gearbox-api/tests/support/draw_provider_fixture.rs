use std::sync::Arc;

use gearbox_api::draw_data::*;
use gearbox_api::wire::draw_data::*;

#[derive(Clone, Copy)]
pub struct Grid {
    pub count: u32,
    pub start: u64,
    pub page: u64,
}

impl Grid {
    pub fn schema() -> CloudSchema {
        CloudSchema {
            revision: SCHEMA_REVISION,
            attributes: vec![
                Attribute {
                    name: "position".into(),
                    scalar: ScalarType::F32,
                    components: 3,
                    semantic: AttributeSemantic::Position,
                    units: "m".into(),
                    normalization: Normalization::None,
                    nullable: false,
                },
                Attribute {
                    name: "source_id".into(),
                    scalar: ScalarType::U64,
                    components: 1,
                    semantic: AttributeSemantic::SourceId,
                    units: "".into(),
                    normalization: Normalization::None,
                    nullable: false,
                },
            ],
        }
    }

    pub fn len(self) -> usize {
        32 + self.count as usize * 20
    }

    pub fn fill(self, offset: usize, bytes: &mut [u8]) {
        let header = [
            u32::from_le_bytes(*b"GBC1"),
            self.count,
            2,
            0,
            self.count * 12,
            0,
            self.count * 8,
            0,
        ];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let offset = offset + index;
            *byte = if offset < 32 {
                header[offset / 4].to_le_bytes()[offset % 4]
            } else if offset < 32 + self.count as usize * 12 {
                let offset = offset - 32;
                let id = self.start + (offset / 12) as u64;
                let value = match offset / 4 % 3 {
                    0 => (id % 257) as f32 * 0.125,
                    1 => (id / 257 % 257) as f32 * 0.125,
                    _ => (id / 66_049 % 17) as f32 * 0.125,
                };
                value.to_le_bytes()[offset % 4]
            } else {
                let offset = offset - 32 - self.count as usize * 12;
                (u64::MAX - self.start - (offset / 8) as u64).to_le_bytes()[offset % 8]
            };
        }
    }

    pub fn descriptor(self) -> PageDescriptor {
        let mut hash = crc32fast::Hasher::new();
        let mut bytes = [0; DATA_CHUNK_BYTES];
        for offset in (0..self.len()).step_by(bytes.len()) {
            let count = bytes.len().min(self.len() - offset);
            self.fill(offset, &mut bytes[..count]);
            hash.update(&bytes[..count]);
        }
        PageDescriptor {
            address: PageAddress {
                scan_key: 0,
                page_id: self.page,
            },
            encoding: PageEncoding::CloudColumnsV1,
            items: self.count,
            byte_len: self.len() as u64,
            crc32: hash.finalize(),
            origin_m: [self.page as f64 * 32.0, 0.0, 0.0],
        }
    }

    pub fn stored(self, budget: &CpuBudget) -> Arc<StoredPage> {
        self.stored_scan(budget, 0)
    }

    pub fn stored_scan(self, budget: &CpuBudget, scan_key: u64) -> Arc<StoredPage> {
        let mut descriptor = self.descriptor();
        descriptor.address.scan_key = scan_key;
        let mut upload = PageUpload::new(budget, descriptor).unwrap();
        let mut bytes = [0; DATA_CHUNK_BYTES];
        for offset in (0..self.len()).step_by(bytes.len()) {
            let count = bytes.len().min(self.len() - offset);
            self.fill(offset, &mut bytes[..count]);
            upload.push(offset as u64, &bytes[..count]).unwrap();
        }
        upload.finish_cloud(&Self::schema()).unwrap().into_page()
    }
}

impl PageProvider for Grid {
    fn owned_bytes(&self) -> u64 {
        0
    }

    fn availability(&self, page: &PageDescriptor) -> ProviderAvailability {
        if page.address.page_id == self.page {
            ProviderAvailability::Available
        } else {
            ProviderAvailability::Missing
        }
    }

    fn load(&self, sink: &mut PageSink<'_>) -> Result<(), ProviderError> {
        let mut bytes = [0; DATA_CHUNK_BYTES];
        for offset in (0..self.len()).step_by(bytes.len()) {
            sink.check()?;
            let count = bytes.len().min(self.len() - offset);
            self.fill(offset, &mut bytes[..count]);
            sink.push(&bytes[..count])?;
        }
        Ok(())
    }
}
