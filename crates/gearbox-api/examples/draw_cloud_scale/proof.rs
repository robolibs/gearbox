use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[path = "delta.rs"]
mod delta;
#[path = "quiescence.rs"]
mod quiescence;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Allocations {
    positions: u64,
    attributes: u64,
    indices: u64,
    lod_views: u64,
    styles: u64,
    placements: u64,
    points: u64,
    pages: u64,
    gpu_bytes: u64,
    auxiliary_gpu_bytes: u64,
    staging_gpu_bytes: u64,
    retiring_gpu_bytes: u64,
    other_gpu_bytes: u64,
    gpu_peak: u64,
    uploaded: u64,
    frame_upload: u64,
    max_frame_upload: u64,
}

impl Allocations {
    fn resident(&self) -> u64 {
        self.positions + self.attributes + self.indices + self.styles + self.placements
    }
    fn empty(&self) -> bool {
        self.resident() == 0 && self.gpu_bytes == 0 && self.points == 0 && self.pages == 0
    }
    fn million(&self) -> bool {
        self.points == 1_000_000
            && self.pages == 16
            && self.positions == 12_000_000
            && self.attributes == 4_000_000
            && self.indices > 4_000_000
            && self.styles > 0
            && self.styles < 65_536
            && self.placements > 0
            && self.placements < 65_536
            && self.gpu_bytes == self.resident() + self.lod_views + self.auxiliary_gpu_bytes
            && self.staging_gpu_bytes == 0
            && self.retiring_gpu_bytes == 0
            && self.other_gpu_bytes == 0
    }
}

#[derive(Deserialize)]
struct Report {
    sequence: u64,
    allocations: Allocations,
    cpu: quiescence::Cpu,
    receipts: quiescence::Receipts,
    uploads: delta::Uploads,
}

fn wait(root: &Path, label: &str, accept: impl Fn(&Allocations) -> bool) -> Result<Allocations> {
    Ok(wait_report(root, label, |report| accept(&report.allocations))?.allocations)
}

fn wait_report(root: &Path, label: &str, accept: impl Fn(&Report) -> bool) -> Result<Report> {
    let until = deadline();
    let mut last = None;
    loop {
        if let Ok(bytes) = std::fs::read(root.join("report.json"))
            && let Ok(report) = serde_json::from_slice::<Report>(&bytes)
        {
            if accept(&report) {
                return Ok(report);
            }
            last = Some(report.allocations);
        }
        if Instant::now() >= until {
            return Err(format!("allocation timeout at {label}: {last:?}").into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn record(root: &Path, name: &str, value: serde_json::Value) -> Result<()> {
    std::fs::write(
        root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&value)?,
    )?;
    eprintln!("scale checkpoint {name}: {value}");
    Ok(())
}

pub fn run(
    data: &mut DrawDataClient,
    writer: &mut DrawDataWriter,
    fixture: &fixture::Fixture,
    root: &Path,
) -> Result<DatasetRevision> {
    require(root.is_dir(), "native probe directory must exist")?;
    let observer = fixture::make(DataKey::new(OBSERVER, "empty")?, true)?;
    let observer_revision = upload(writer, &observer)?;
    clear(data, writer.lease(), NS)?;
    data.wait_visible_with_deadline(
        &observer.descriptor.key,
        observer_revision,
        None,
        deadline(),
    )?;
    let empty = wait(root, "initial_empty", Allocations::empty)?;
    let baseline = quiescence::baseline(data, &observer.descriptor.key, root)?;
    let limits = data.capabilities().limits.clone();
    record(
        root,
        "baseline",
        serde_json::json!({"allocations":empty,"resources":baseline.resources,"quiescence":baseline,"limits":limits}),
    )?;
    let revision = upload(writer, fixture)?;
    let receipt = visible(data, &fixture.descriptor.key, revision)?;
    let allocated = wait(root, "million", Allocations::million)?;
    require(
        allocated.max_frame_upload <= limits.upload_bytes_per_frame,
        "per-frame upload allowance exceeded",
    )?;
    require(
        allocated.gpu_peak <= limits.gpu_bytes,
        "GPU budget exceeded",
    )?;
    require(
        allocated.uploaded - empty.uploaded == allocated.resident(),
        "initial GPU upload contains unexpected copies",
    )?;
    let current = status(data, &fixture.descriptor.key)?;
    require(
        current.failure.is_none()
            && current.resources.gpu_bytes
                == allocated.resident() + allocated.lod_views + allocated.auxiliary_gpu_bytes,
        "public status disagrees with live buffers",
    )?;
    record(
        root,
        "million",
        serde_json::json!({"revision":revision,"visibility":receipt,"allocations":allocated,"status":current}),
    )?;
    let changed = Presentation {
        glow: 0.28,
        ..fixture::presentation()
    };
    let reply = call(
        data,
        DataRequest {
            lease: writer.lease(),
            incarnation: revision.incarnation,
            base_generation: revision.generation,
            ..DataRequest::new(data_op::PATCH_PRESENTATION)
        }
        .with_key(&fixture.descriptor.key)?
        .with_metadata(&changed)?,
    )?;
    let restyled = DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    };
    let receipt = visible(data, &fixture.descriptor.key, restyled)?;
    let style = wait(root, "style_only", |value| {
        value.million() && value.uploaded > allocated.uploaded
    })?;
    require(
        style.uploaded - allocated.uploaded == style.styles + style.placements,
        "presentation-only edit reuploaded point records",
    )?;
    require(
        style.gpu_peak <= limits.gpu_bytes
            && style.max_frame_upload <= limits.upload_bytes_per_frame,
        "style edit exceeded GPU/upload budget",
    )?;
    record(
        root,
        "style-only",
        serde_json::json!({"revision":restyled,"visibility":receipt,"allocations":style,"uploaded_delta":style.uploaded-allocated.uploaded}),
    )?;
    delta::run(data, writer, fixture, root, restyled, changed)?;
    clear(data, writer.lease(), NS)?;
    let cleared = wait(root, "clear", Allocations::empty)?;
    let released = quiescence::baseline(data, &observer.descriptor.key, root)?;
    require(
        released.resources == baseline.resources,
        &format!("clear resource mismatch: {released:?} vs {baseline:?}"),
    )?;
    record(
        root,
        "clear",
        serde_json::json!({"allocations":cleared,"resources":released.resources,"quiescence":released}),
    )?;
    let fresh = upload(writer, fixture)?;
    require(
        fresh.incarnation != revision.incarnation && fresh.generation == 1,
        "clear/reload reused an old incarnation",
    )?;
    let receipt = visible(data, &fixture.descriptor.key, fresh)?;
    let reloaded = wait(root, "reload", Allocations::million)?;
    require(
        reloaded.uploaded - cleared.uploaded == reloaded.resident(),
        "reload did not upload exactly its live records/uniforms",
    )?;
    require(
        reloaded.max_frame_upload <= limits.upload_bytes_per_frame
            && reloaded.gpu_peak <= limits.gpu_bytes,
        "reload exceeded budgets",
    )?;
    record(
        root,
        "reload",
        serde_json::json!({"result":"pass","revision":fresh,"visibility":receipt,"allocations":reloaded}),
    )?;
    Ok(fresh)
}
