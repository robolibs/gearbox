use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gearbox_api::wire::draw_data::*;
use gearbox_api::{DataAdmission, DataRequestTicket, DrawDataClient, DrawDataPipeline};

#[allow(dead_code)]
#[path = "draw_data_contracts.rs"]
mod contracts;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct UploadFixture {
    pub descriptor: DataDescriptor,
    pub scan: ScanDescriptor,
    pub pages: Vec<(PageDescriptor, Vec<u8>)>,
    pub bytes: u64,
}

impl UploadFixture {
    pub fn cloud(namespace: &str, id: &str, points: usize) -> Result<Self> {
        if !(1..=1_048_576).contains(&points) {
            return Err("fixture count outside 1..1048576".into());
        }
        let mut schema = contracts::schema();
        schema.attributes.truncate(2);
        let mut pages = Vec::new();
        for start in (0..points).step_by(50_000) {
            let end = points.min(start + 50_000);
            let mut positions = Vec::with_capacity((end - start) * 24);
            let mut ids = Vec::with_capacity((end - start) * 8);
            for id in start..end {
                for coordinate in [(id % 1000) as f64 * 0.01, (id / 1000) as f64 * 0.01, 0.0] {
                    positions.extend_from_slice(&coordinate.to_le_bytes());
                }
                ids.extend_from_slice(&(id as u64).to_le_bytes());
            }
            let data = CloudPage {
                count: (end - start) as u32,
                columns: vec![
                    ColumnData {
                        values: positions,
                        validity: vec![],
                    },
                    ColumnData {
                        values: ids,
                        validity: vec![],
                    },
                ],
            }
            .encode(&schema)?;
            let page = PageDescriptor {
                address: PageAddress {
                    scan_key: 0,
                    page_id: pages.len() as u64,
                },
                encoding: PageEncoding::CloudColumnsV1,
                items: (end - start) as u32,
                byte_len: data.len() as u64,
                crc32: page_crc32(&data),
                origin_m: [0.0; 3],
            };
            pages.push((page, data));
        }
        let bytes = pages.iter().map(|(page, _)| page.byte_len).sum();
        Ok(Self {
            descriptor: DataDescriptor {
                key: DataKey::new(namespace, id)?,
                stream: StreamMode::RetainedMap,
                shape: DataShape::Cloud { schema },
                ..contracts::cloud()
            },
            scan: ScanDescriptor {
                source_count: points as u64,
                organization: None,
                ..contracts::scan(0)
            },
            pages,
            bytes,
        })
    }
}

pub enum Connection<'a> {
    Direct(&'a mut DrawDataClient),
    Pipeline {
        client: &'a DrawDataPipeline,
        window: usize,
    },
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

fn checked(reply: DataReply) -> Result<DataReply> {
    if reply.code != data_code::OK {
        return Err(format!(
            "data request failed {}: {}",
            reply.code,
            String::from_utf8_lossy(&reply.message)
        )
        .into());
    }
    Ok(reply)
}

impl Connection<'_> {
    pub fn epoch(&self) -> u64 {
        match self {
            Self::Direct(client) => client.capabilities().host_epoch,
            Self::Pipeline { client, .. } => client.host_epoch(),
        }
    }

    pub fn call(&mut self, request: DataRequest) -> Result<DataReply> {
        let reply = match self {
            Self::Direct(client) => client.request_with_deadline(&request, deadline())?,
            Self::Pipeline { client, .. } => client
                .submit(&request, deadline(), DataAdmission::Wait)?
                .wait(deadline())?,
        };
        checked(reply)
    }

    pub fn open(&mut self) -> Result<u64> {
        Ok(self
            .call(DataRequest {
                host_epoch: self.epoch(),
                ..DataRequest::new(data_op::OPEN_WRITER)
            })?
            .lease)
    }

    pub fn clear(&mut self, namespace: &str, lease: u64) -> Result<()> {
        self.call(
            DataRequest {
                host_epoch: self.epoch(),
                lease,
                ..DataRequest::new(data_op::CLEAR)
            }
            .with_metadata(&NamespaceFilter {
                namespace: namespace.into(),
            })?,
        )?;
        Ok(())
    }

    pub fn close_writer(&mut self, lease: u64) -> Result<()> {
        self.call(DataRequest {
            host_epoch: self.epoch(),
            lease,
            ..DataRequest::new(data_op::CLOSE_WRITER)
        })?;
        Ok(())
    }

    pub fn upload(
        &mut self,
        fixture: &UploadFixture,
        lease: u64,
        transaction: u64,
        base: Option<DatasetRevision>,
    ) -> Result<DatasetRevision> {
        let begin = DataRequest {
            host_epoch: self.epoch(),
            lease,
            transaction,
            incarnation: base.map_or(0, |r| r.incarnation),
            base_generation: base.map_or(0, |r| r.generation),
            total_bytes: fixture.bytes,
            ..DataRequest::new(data_op::BEGIN)
        }
        .with_key(&fixture.descriptor.key)?
        .with_metadata(&BeginMetadata {
            descriptor: fixture.descriptor.clone(),
            mode: CommitMode::Replace,
            scans: 1,
            pages: fixture.pages.len() as u32,
            removed_pages: 0,
            removed_scans: 0,
        })?;
        self.call(begin.clone())?;
        self.call(
            DataRequest {
                scan_key: fixture.scan.scan_key,
                ..edit(&begin, data_op::DECLARE_SCAN)
            }
            .with_metadata(&fixture.scan)?,
        )?;
        for (page, bytes) in &fixture.pages {
            self.call(
                DataRequest {
                    scan_key: page.address.scan_key,
                    page_id: page.address.page_id,
                    ..edit(&begin, data_op::DECLARE_PAGE)
                }
                .with_metadata(page)?,
            )?;
            let mut pending = Pending(VecDeque::new());
            for (index, chunk) in bytes.chunks(DATA_CHUNK_BYTES).enumerate() {
                let request = DataRequest {
                    scan_key: page.address.scan_key,
                    page_id: page.address.page_id,
                    offset: (index * DATA_CHUNK_BYTES) as u64,
                    payload: chunk.to_vec(),
                    ..edit(&begin, data_op::CHUNK)
                };
                match self {
                    Self::Direct(_) => {
                        self.call(request)?;
                    }
                    Self::Pipeline { client, window } => {
                        if *window == 0 || *window > 64 {
                            return Err("invalid fixture pipeline window".into());
                        }
                        if pending.0.len() >= *window {
                            checked(pending.0.pop_front().unwrap().wait(deadline())?)?;
                        }
                        pending.0.push_back(client.submit(
                            &request,
                            deadline(),
                            DataAdmission::Wait,
                        )?);
                    }
                }
            }
            while let Some(mut request) = pending.0.pop_front() {
                checked(request.wait(deadline())?)?;
            }
        }
        let reply = self.call(edit(&begin, data_op::COMMIT))?;
        if reply.stage != data_stage::RETAINED || reply.visible_generation != 0 {
            return Err("incorrect retention acknowledgement".into());
        }
        Ok(DatasetRevision {
            incarnation: reply.incarnation,
            generation: reply.stored_generation,
        })
    }

    pub fn verify(&mut self, fixture: &UploadFixture, revision: DatasetRevision) -> Result<()> {
        let reply = self.call(
            DataRequest {
                host_epoch: self.epoch(),
                ..DataRequest::new(data_op::LIST)
            }
            .with_metadata(&DataListQuery {
                namespace: fixture.descriptor.key.namespace.clone(),
                limit: 128,
                cursor: None,
            })?,
        )?;
        let listing = DrawSummaryPage::from_json(&reply.metadata)?;
        let item = listing
            .items
            .iter()
            .find(|item| item.id == fixture.descriptor.key.id)
            .ok_or("missing retained fixture")?;
        match item.object {
            SummaryOwner::Dataset {
                revision: current,
                source_samples,
                valid_samples,
                payload_bytes,
                pages,
                scans,
                ..
            } if current == revision
                && source_samples == fixture.scan.source_count
                && valid_samples == source_samples
                && payload_bytes == fixture.bytes
                && pages as usize == fixture.pages.len()
                && scans == 1 =>
            {
                Ok(())
            }
            _ => Err("retained fixture counts/revision do not match".into()),
        }
    }
}

struct Pending(VecDeque<DataRequestTicket>);
impl Drop for Pending {
    fn drop(&mut self) {
        for request in &self.0 {
            request.cancel();
        }
    }
}

fn edit(begin: &DataRequest, operation: u32) -> DataRequest {
    DataRequest {
        host_epoch: begin.host_epoch,
        lease: begin.lease,
        transaction: begin.transaction,
        incarnation: begin.incarnation,
        base_generation: begin.base_generation,
        key: begin.key.clone(),
        ..DataRequest::new(operation)
    }
}
