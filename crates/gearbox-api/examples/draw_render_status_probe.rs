use std::io::{Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use gearbox_api::wire::draw_data::*;
use gearbox_api::{
    Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawDataClient, DrawDataWriter,
    IdentitySource, registry,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

fn call(client: &mut DrawDataClient, mut request: DataRequest) -> Result<DataReply> {
    request.host_epoch = client.capabilities().host_epoch;
    let reply = client.request_with_deadline(&request, deadline())?;
    if reply.code != data_code::OK {
        return Err(format!(
            "{}: {}",
            reply.code,
            String::from_utf8_lossy(&reply.message)
        )
        .into());
    }
    Ok(reply)
}

fn status(client: &mut DrawDataClient, key: &DataKey) -> Result<DatasetStatus> {
    let reply = call(
        client,
        DataRequest::new(data_op::STATUS)
            .with_key(key)?
            .with_metadata(&DataStatusQuery {
                generation: None,
                view: None,
            })?,
    )?;
    if !matches!(reply.stage, data_stage::RETAINED | data_stage::VISIBLE) {
        return Err("probe lost its retained generation".into());
    }
    Ok(DatasetStatus::from_json(&reply.metadata)?)
}

fn wait_status(
    client: &mut DrawDataClient,
    key: &DataKey,
    label: &str,
    accept: impl Fn(&DatasetStatus) -> bool,
) -> Result<DatasetStatus> {
    let until = deadline();
    loop {
        let value = status(client, key)?;
        if accept(&value) {
            println!(
                "{}",
                serde_json::json!({"checkpoint": label, "status": value})
            );
            return Ok(value);
        }
        if Instant::now() >= until {
            return Err(format!("timed out at {label}: {}", serde_json::to_string(&value)?).into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn upload(
    writer: &mut DrawDataWriter,
    key: &DataKey,
    base: Option<DatasetRevision>,
    extent: Option<f64>,
) -> Result<DatasetRevision> {
    let schema = CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: [
            ("position", ScalarType::F64, 3, AttributeSemantic::Position),
            ("source_id", ScalarType::U64, 1, AttributeSemantic::SourceId),
            ("rgba", ScalarType::U8, 4, AttributeSemantic::Color),
        ]
        .into_iter()
        .map(|(name, scalar, components, semantic)| Attribute {
            name: name.into(),
            scalar,
            components,
            semantic,
            units: if semantic == AttributeSemantic::Position {
                "m"
            } else {
                ""
            }
            .into(),
            normalization: if semantic == AttributeSemantic::Color {
                Normalization::Unorm
            } else {
                Normalization::None
            },
            nullable: false,
        })
        .collect(),
    };
    let bytes = CloudPage {
        count: 2,
        columns: vec![
            ColumnData {
                values: [0.0f64, 0.0, 1.0, extent.unwrap_or(1.0), 0.0, 1.0]
                    .into_iter()
                    .flat_map(f64::to_le_bytes)
                    .collect(),
                validity: vec![],
            },
            ColumnData {
                values: [u64::MAX, u64::MAX - 1]
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect(),
                validity: vec![],
            },
            ColumnData {
                values: vec![240, 155, 70, 255, 80, 150, 230, 96],
                validity: vec![],
            },
        ],
    }
    .encode(&schema)?;
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: key.clone(),
        shape: DataShape::Cloud { schema },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Attribute {
                name: "rgba".into(),
                space: ColorSpace::Srgb,
            },
            ..Default::default()
        },
    };
    let scans = [ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: Some("renderer-status-probe".into()),
        frame: DataFrame::default(),
        source_count: 2,
        organization: None,
    }];
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: 2,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let pages = [DataUploadPage {
        descriptor: &page,
        bytes: &bytes,
    }];
    let mut upload = DataUpload::replace(
        &descriptor,
        if extent.is_some() { &scans } else { &[] },
        if extent.is_some() { &pages } else { &[] },
    );
    upload.base = base;
    let reply = writer.upload(&upload, deadline(), None)?;
    Ok(DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    })
}

fn wait_activation(key: &DataKey, revision: DatasetRevision) -> Result<()> {
    let path = std::env::var("GEARBOX_PROBE_RENDER_LOG")?;
    let expected = format!(
        "typed cloud activated {}/{} incarnation={} generation={} points=2",
        key.namespace, key.id, revision.incarnation, revision.generation
    );
    let until = deadline();
    loop {
        let mut log = std::fs::File::open(&path)?;
        let start = log.metadata()?.len().saturating_sub(1 << 20);
        log.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        log.take(1 << 20).read_to_end(&mut bytes)?;
        if String::from_utf8_lossy(&bytes).contains(&expected) {
            println!(
                "{}",
                serde_json::json!({"checkpoint":"native_activation_log", "entry":expected, "completed_frame_ack":false})
            );
            return Ok(());
        }
        if Instant::now() >= until {
            return Err(format!("missing native activation: {expected}").into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn probe(
    data: &mut DrawDataClient,
    writer: &mut DrawDataWriter,
    key: &DataKey,
    observer: &DataKey,
    baseline: &DataResourceUsage,
) -> Result<()> {
    let bad = upload(writer, key, None, Some(1e12 + 0.1))?;
    let failed = wait_status(data, key, "preparation_failure", |value| {
        value.retained == Some(bad)
            && value
                .failure
                .as_ref()
                .is_some_and(|f| f.phase == DataPhase::Preparing && f.code == data_code::INVALID)
    })?;
    if !matches!(failed.visibility, GenerationVisibility::Failed { .. }) {
        return Err("failed retained preparation must report failed visibility".into());
    }
    let repaired = upload(writer, key, Some(bad), Some(1.0))?;
    wait_status(data, key, "replacement_resources", |value| {
        value.retained == Some(repaired)
            && value.failure.is_none()
            && value.resources.gpu_bytes > baseline.gpu_bytes
    })?;
    let completed = data.wait_visible_with_deadline(key, repaired, None, deadline())?;
    println!(
        "{}",
        serde_json::json!({"checkpoint":"completed_frame", "visibility":completed.visibility()})
    );
    wait_activation(key, repaired)?;
    let empty = upload(writer, key, Some(repaired), None)?;
    let empty_receipt =
        data.wait_visible_with_deadline(key, empty, Some(completed.target.view), deadline())?;
    if empty_receipt.disposition != FrontierDisposition::Empty || empty_receipt.selected_items != 0
    {
        return Err("empty retained cloud did not complete an empty frontier".into());
    }
    println!(
        "{}",
        serde_json::json!({"checkpoint":"empty_completed_frame", "visibility":empty_receipt.visibility()})
    );
    let historical =
        data.wait_visible_with_deadline(key, repaired, Some(completed.target.view), deadline())?;
    if historical.target.dataset != repaired {
        return Err("old generation inherited newer visibility".into());
    }
    println!(
        "{}",
        serde_json::json!({"checkpoint":"historical_completed_frame", "visibility":historical.visibility()})
    );
    let obsolete =
        data.wait_visible_with_deadline(key, bad, Some(completed.target.view), deadline());
    if !matches!(obsolete, Err(gearbox_api::WaitVisibleError::Terminal(GenerationVisibility::Superseded { target, successor })) if target.dataset == bad && successor == empty)
    {
        return Err("never-displayed generation was not explicitly superseded".into());
    }
    println!(
        "{}",
        serde_json::json!({"checkpoint":"never_displayed_superseded", "generation":bad.generation})
    );
    let mut current = upload(writer, key, Some(empty), Some(1.0))?;
    data.wait_visible_with_deadline(key, current, Some(completed.target.view), deadline())?;
    for (label, bounds, selected) in [
        (
            "partially_clipped_frame",
            Some(ClipBounds {
                min_m: [-0.1, -1.0, 0.0],
                max_m: [0.1, 1.0, 2.0],
                ..Default::default()
            }),
            1,
        ),
        (
            "wholly_clipped_frame",
            Some(ClipBounds {
                min_m: [2.0; 3],
                max_m: [3.0; 3],
                ..Default::default()
            }),
            0,
        ),
        ("unclipped_frame", None, 2),
    ] {
        let reply = call(
            data,
            DataRequest {
                lease: writer.lease(),
                incarnation: current.incarnation,
                base_generation: current.generation,
                ..DataRequest::new(data_op::PATCH_PRESENTATION)
            }
            .with_key(key)?
            .with_metadata(&Presentation {
                clip: bounds,
                ..Default::default()
            })?,
        )?;
        current = DatasetRevision {
            incarnation: reply.incarnation,
            generation: reply.stored_generation,
        };
        let receipt =
            data.wait_visible_with_deadline(key, current, Some(completed.target.view), deadline())?;
        let expected = if selected == 0 {
            FrontierDisposition::Culled
        } else {
            FrontierDisposition::Drawn
        };
        if receipt.source_samples != 2
            || receipt.selected_items != selected
            || receipt.disposition != expected
        {
            return Err(format!("incorrect clipping receipt: {receipt:?}").into());
        }
        println!(
            "{}",
            serde_json::json!({"checkpoint":label, "visibility":receipt.visibility()})
        );
    }
    clear(data, writer.lease(), &key.namespace)?;
    wait_status(data, observer, "deferred_release_baseline", |value| {
        value.resources.gpu_bytes == baseline.gpu_bytes
            && value.resources.cpu_bytes == baseline.cpu_bytes
            && value.resources.resident_page_bytes == baseline.resident_page_bytes
            && value.resources.retained_datasets == baseline.retained_datasets
            && value.resources.incomplete_bytes == 0
            && value.resources.staged_bytes == 0
    })?;
    Ok(())
}

fn clear(data: &mut DrawDataClient, lease: u64, namespace: &str) -> Result<()> {
    call(
        data,
        DataRequest {
            lease,
            ..DataRequest::new(data_op::CLEAR)
        }
        .with_metadata(&NamespaceFilter {
            namespace: namespace.into(),
        })?,
    )?;
    Ok(())
}

fn main() -> Result<()> {
    let instance = std::env::args()
        .nth(1)
        .ok_or("provide the owned cloud-basics INSTANCE")?;
    let entry = registry::find(&instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "renderer-status-probe")?;
    let mut data = client.draw_data()?;
    let observer = DataKey::new("vfx-gallery/cloud-basics", "view-0")?;
    let baseline = wait_status(
        &mut data,
        &observer,
        "existing_native_allocations",
        |value| value.resources.gpu_bytes > 0,
    )?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(2, deadline())?;
    let key = DataKey::new(
        &format!("render-status-probe/{}", std::process::id()),
        "recovery",
    )?;
    let result = probe(&mut data, &mut writer, &key, &observer, &baseline.resources);
    let cleanup = clear(&mut data, writer.lease(), &key.namespace);
    let close = writer.close(deadline());
    result?;
    cleanup?;
    close?;
    println!(
        "{}",
        serde_json::json!({"result":"pass", "instance":instance, "gpu_baseline_bytes":baseline.resources.gpu_bytes, "cpu_baseline_bytes":baseline.resources.cpu_bytes, "visibility_ack_tested":true})
    );
    Ok(())
}
