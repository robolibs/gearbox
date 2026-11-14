use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gearbox_api::wire::draw_data::*;
use gearbox_api::{
    Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawDataClient, DrawDataWriter,
    IdentitySource, registry,
};
use serde::{Deserialize, Serialize};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "draw-multiview-probe";
#[path = "draw_multiview_probe/auxiliary.rs"]
mod auxiliary;
#[path = "draw_multiview_probe/cloud_lod.rs"]
mod cloud_lod;
#[path = "draw_multiview_probe/cpu_lifecycle.rs"]
mod cpu_lifecycle;
#[path = "draw_multiview_probe/legacy_admission.rs"]
mod legacy_admission;
#[path = "draw_multiview_probe/residency.rs"]
mod residency;
#[path = "draw_multiview_probe/selection.rs"]
mod selection;
#[path = "draw_multiview_probe/upload_fairness.rs"]
mod upload_fairness;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct Stamp {
    view: ViewIdentity,
    incarnation: u64,
    generation: u64,
    frame: u64,
    source_samples: u64,
    selected_items: u64,
}
#[derive(Debug, Deserialize, Serialize)]
struct Report {
    sequence: u64,
    views: [Option<ViewIdentity>; 2],
    samples: [u32; 2],
    completed: [Option<Stamp>; 2],
    delayed_pages: u64,
    secondary_alive: bool,
    active: bool,
    gpu_reserved: u64,
    cpu: serde_json::Value,
    allocations: serde_json::Value,
    uploads: serde_json::Value,
    legacy: serde_json::Value,
    scheduler: serde_json::Value,
    backdrops: serde_json::Value,
    lod: serde_json::Value,
    motion: serde_json::Value,
}

struct Control {
    root: PathBuf,
    sequence: u64,
    captures: Vec<serde_json::Value>,
    gpu_headroom: Option<u64>,
    cpu_headroom: Option<u64>,
    secondary_size: Option<[u32; 2]>,
    secondary_east_m: Option<f32>,
    /// Views draw every page from its coarse tier while set.
    hold_full: bool,
}
impl Control {
    fn read(&self) -> Result<Report> {
        Ok(serde_json::from_slice(&std::fs::read(
            self.root.join("report.json"),
        )?)?)
    }
    fn wait(&self, name: &str, accept: impl Fn(&Report) -> bool) -> Result<Report> {
        let until = deadline();
        loop {
            if let Ok(report) = self.read()
                && report.sequence >= self.sequence
                && accept(&report)
            {
                println!("{}", serde_json::json!({"checkpoint":name,"report":report}));
                return Ok(report);
            }
            if Instant::now() >= until {
                return Err(format!("timeout at {name}: {:?}", self.read()).into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    fn set(&mut self, action: &str, active: bool, maximum: u64) -> Result<Report> {
        self.sequence += 1;
        let bytes = serde_json::to_vec(
            &serde_json::json!({"sequence":self.sequence,"action":action,"active":active,"max_generation":maximum,"gpu_headroom":self.gpu_headroom,"cpu_headroom":self.cpu_headroom,"secondary_size":self.secondary_size,"secondary_east_m":self.secondary_east_m,"hold_full":self.hold_full}),
        )?;
        std::fs::write(self.root.join("control.tmp"), bytes)?;
        std::fs::rename(
            self.root.join("control.tmp"),
            self.root.join("control.json"),
        )?;
        self.wait(action, |_| true)
    }
    fn capture(&mut self, name: &str, maximum: u64, colors: [&str; 2]) -> Result<()> {
        self.set("capture", true, maximum)?;
        let until = deadline();
        for suffix in ['a', 'b'] {
            let path = self.root.join(format!("{}-{suffix}.png", self.sequence));
            loop {
                let complete = std::fs::File::open(&path).ok().is_some_and(|mut file| {
                    let mut tail = [0; 12];
                    file.seek(SeekFrom::End(-12)).is_ok()
                        && file.read_exact(&mut tail).is_ok()
                        && tail == [0, 0, 0, 0, b'I', b'E', b'N', b'D', 174, 66, 96, 130]
                });
                if complete {
                    break;
                }
                if Instant::now() >= until {
                    return Err(format!("missing complete capture {}", path.display()).into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        self.captures
            .push(serde_json::json!({"name":name,"sequence":self.sequence,"colors":colors}));
        std::fs::write(
            self.root.join("captures.json"),
            serde_json::to_vec_pretty(&self.captures)?,
        )?;
        Ok(())
    }
}

fn call(data: &mut DrawDataClient, mut request: DataRequest) -> Result<DataReply> {
    request.host_epoch = data.capabilities().host_epoch;
    let reply = data.request_with_deadline(&request, deadline())?;
    if reply.code != data_code::OK {
        return Err(format!(
            "API code {}: {}",
            reply.code,
            String::from_utf8_lossy(&reply.message)
        )
        .into());
    }
    Ok(reply)
}

fn status(
    data: &mut DrawDataClient,
    key: &DataKey,
    revision: Option<DatasetRevision>,
    view: Option<ViewIdentity>,
) -> Result<DatasetStatus> {
    let reply = call(
        data,
        DataRequest::new(data_op::STATUS)
            .with_key(key)?
            .with_metadata(&DataStatusQuery {
                generation: revision.map(|value| value.generation),
                view,
            })?,
    )?;
    Ok(DatasetStatus::from_json(&reply.metadata)?)
}

fn upload(
    writer: &mut DrawDataWriter,
    key: &DataKey,
    base: Option<DatasetRevision>,
    rgba: [u8; 4],
) -> Result<DatasetRevision> {
    upload_at(writer, key, base, rgba, 0.0)
}

fn upload_at(
    writer: &mut DrawDataWriter,
    key: &DataKey,
    base: Option<DatasetRevision>,
    rgba: [u8; 4],
    offset: f64,
) -> Result<DatasetRevision> {
    let schema = CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            Attribute {
                name: "position".into(),
                scalar: ScalarType::F64,
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
    };
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: key.clone(),
        shape: DataShape::Cloud {
            schema: schema.clone(),
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Constant { rgba },
            size: PointSize::Metres { radius: 0.8 },
            glow: 0.0,
            blur: 0.0,
            opacity: 1.0,
            ..Default::default()
        },
    };
    let payloads = [-1.4f64, 1.4]
        .into_iter()
        .enumerate()
        .map(|(index, x)| {
            CloudPage {
                count: 1,
                columns: vec![
                    ColumnData {
                        values: [x + offset, 25.0, 2.0]
                            .into_iter()
                            .flat_map(f64::to_le_bytes)
                            .collect(),
                        validity: vec![],
                    },
                    ColumnData {
                        values: (u64::MAX - index as u64).to_le_bytes().to_vec(),
                        validity: vec![],
                    },
                ],
            }
            .encode(&schema)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let descriptors: Vec<_> = payloads
        .iter()
        .enumerate()
        .map(|(index, bytes)| PageDescriptor {
            address: PageAddress {
                scan_key: 0,
                page_id: index as u64,
            },
            encoding: PageEncoding::CloudColumnsV1,
            items: 1,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(bytes),
            origin_m: [0.0; 3],
        })
        .collect();
    let pages: Vec<_> = descriptors
        .iter()
        .zip(&payloads)
        .map(|(descriptor, bytes)| DataUploadPage { descriptor, bytes })
        .collect();
    let scans = [ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: Some(NS.into()),
        frame: DataFrame::default(),
        source_count: 2,
        organization: None,
    }];
    let mut upload = DataUpload::replace(&descriptor, &scans, &pages);
    upload.base = base;
    let reply = writer.upload(&upload, deadline(), None)?;
    Ok(DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    })
}

fn pair(report: &Report, a: u64, b: u64) -> bool {
    report
        .completed
        .iter()
        .zip([a, b])
        .all(|(stamp, generation)| {
            stamp.is_some_and(|stamp| {
                stamp.generation == generation
                    && stamp.source_samples == 2
                    && stamp.selected_items == 2
            })
        })
}

fn clear(data: &mut DrawDataClient, lease: u64) -> Result<()> {
    call(
        data,
        DataRequest {
            lease,
            ..DataRequest::new(data_op::CLEAR)
        }
        .with_metadata(&NamespaceFilter {
            namespace: NS.into(),
        })?,
    )?;
    Ok(())
}

fn run(
    control: &mut Control,
    data: &mut DrawDataClient,
    writer: &mut DrawDataWriter,
) -> Result<()> {
    let key = DataKey::new(NS, "cloud")?;
    let observer = DataKey::new("vfx-gallery/cloud-basics", "view-0")?;
    let baseline = status(data, &observer, None, None)?.resources;
    control.set("configure", true, 1)?;
    let first = upload(writer, &key, None, [255, 24, 24, 255])?;
    let initial = control.wait("both_initial", |r| pair(r, 1, 1))?;
    let [a, b] = initial.views.map(|v| v.unwrap());
    require(a != b, "views must have distinct identities")?;
    require(
        initial.samples[1] == 1,
        "secondary must use single-sample rasterization",
    )?;
    require(
        initial.samples[0] > 1,
        "primary camera must exercise different pipeline/MSAA keys",
    )?;
    for view in [a, b] {
        data.wait_visible_with_deadline(&key, first, Some(view), deadline())?;
    }
    control.capture("initial", 1, ["red", "red"])?;
    let second = upload(writer, &key, Some(first), [24, 24, 255, 255])?;
    let advanced = control.wait("primary_second", |r| pair(r, 2, 1) && r.delayed_pages > 0)?;
    let frame = advanced.completed[0].unwrap().frame;
    control.wait("secondary_still_drawing_first", |r| {
        pair(r, 2, 1) && r.completed[1].unwrap().frame > frame + 3
    })?;
    require(
        matches!(
            status(data, &key, Some(second), Some(b))?.visibility,
            GenerationVisibility::Pending { .. }
        ),
        "stalled secondary must report pending replacement",
    )?;
    control.capture("one_page_delayed", 1, ["blue", "red"])?;
    let third = upload(writer, &key, Some(second), [255, 24, 255, 255])?;
    control.wait("primary_third", |r| pair(r, 3, 1))?;
    require(
        matches!(
            status(data, &key, Some(second), Some(b))?.visibility,
            GenerationVisibility::Superseded { .. }
        ),
        "never displayed secondary generation must be superseded",
    )?;
    data.wait_visible_with_deadline(&key, second, Some(a), deadline())?;
    control.capture("superseded_on_secondary", 1, ["magenta", "red"])?;
    control.set("configure", true, 0)?;
    control.wait("secondary_recovers_directly_to_third", |r| pair(r, 3, 3))?;
    data.wait_visible_with_deadline(&key, third, Some(b), deadline())?;
    control.capture("recovered", 0, ["magenta", "magenta"])?;
    control.set("configure", false, 0)?;
    let settled = control.wait("paused_secondary", |r| {
        !r.active && r.completed[0].unwrap().frame > r.completed[1].unwrap().frame + 20
    })?;
    let paused_frame = settled.completed[1].unwrap().frame;
    let fourth = upload(writer, &key, Some(third), [24, 24, 255, 255])?;
    let paused = control.wait("primary_advances_while_secondary_paused", |r| pair(r, 4, 3))?;
    require(
        paused.completed[1].unwrap().frame == paused_frame,
        "paused view must not complete a frame",
    )?;
    require(paused.views[1] == Some(b), "paused view identity changed")?;
    control.set("configure", true, 0)?;
    control.wait("resumed_same_identity", |r| pair(r, 4, 4))?;
    control.set("remove", true, 0)?;
    control.wait("secondary_removed", |r| {
        !r.secondary_alive && r.views[1].is_none()
    })?;
    let until = deadline();
    while !matches!(
        status(data, &key, Some(fourth), Some(b))?.visibility,
        GenerationVisibility::ViewGone { .. }
    ) {
        if Instant::now() >= until {
            return Err("removed view did not become view_gone".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    control.set("recreate", true, 0)?;
    let recreated = control.wait("new_view_identity", |r| {
        pair(r, 4, 4) && r.views[1] != Some(b)
    })?;
    let replacement = recreated.views[1].unwrap();
    data.wait_visible_with_deadline(&key, fourth, Some(replacement), deadline())?;
    require(
        matches!(
            status(data, &key, Some(fourth), Some(b))?.visibility,
            GenerationVisibility::ViewGone { .. }
        ),
        "old view identity resurrected",
    )?;
    control.capture("recreated", 0, ["blue", "blue"])?;
    control.set("configure", true, 4)?;
    let fifth = upload(writer, &key, Some(fourth), [255, 24, 255, 255])?;
    let stalled = control.wait("replacement_pending_before_clear", |r| pair(r, 5, 4))?;
    let frame = stalled.completed[0].unwrap().frame;
    control.wait("old_frontier_live_before_clear", |r| {
        pair(r, 5, 4) && r.completed[1].unwrap().frame > frame + 3
    })?;
    clear(data, writer.lease())?;
    let fresh = upload(writer, &key, None, [255, 24, 24, 255])?;
    require(
        fresh.incarnation != fifth.incarnation && fresh.generation == 1,
        "clear did not replace the dataset incarnation",
    )?;
    control.wait("clear_recreation_replaces_both_frontiers", |r| {
        pair(r, 1, 1)
            && r.completed
                .iter()
                .all(|stamp| stamp.unwrap().incarnation == fresh.incarnation)
    })?;
    for view in [a, replacement] {
        data.wait_visible_with_deadline(&key, fresh, Some(view), deadline())?;
    }
    control.capture("clear_recreated", 4, ["red", "red"])?;
    let mut previous = fresh;
    for (headroom, reason, rgba, old_color, next_color, failed_name, recovered_name) in [
        (
            0,
            "GPU allocation",
            [24, 24, 255, 255],
            "red",
            "blue",
            "allocation_denied",
            "allocation_recovered",
        ),
        (
            16,
            "GPU staging",
            [255, 24, 255, 255],
            "blue",
            "magenta",
            "staging_denied",
            "staging_recovered",
        ),
    ] {
        let settled = control.read()?.completed[0]
            .ok_or("missing completed frame")?
            .frame;
        control.wait("settled_before_quota", |r| {
            pair(r, previous.generation, previous.generation)
                && r.completed
                    .iter()
                    .all(|stamp| stamp.unwrap().frame > settled + 20)
        })?;
        control.gpu_headroom = Some(headroom);
        let held = control.set("configure", true, 0)?;
        require(
            held.gpu_reserved > 0,
            "GPU quota reservation was not installed",
        )?;
        let revision = upload(writer, &key, Some(previous), rgba)?;
        let until = deadline();
        loop {
            let current = status(data, &key, Some(revision), Some(a))?;
            if let GenerationVisibility::Failed { target, failure } = &current.visibility {
                require(
                    target.dataset == revision && target.view == a,
                    "failure has wrong target",
                )?;
                if failure.reason.contains(reason) {
                    require(
                        failure.phase == DataPhase::Rendering && failure.code == data_code::BUDGET,
                        "quota denial has wrong phase/code",
                    )?;
                    println!(
                        "{}",
                        serde_json::json!({"checkpoint":failed_name,"status":current})
                    );
                    break;
                }
            }
            if Instant::now() >= until {
                return Err(format!("missing {reason} denial: {current:?}").into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let stalled_frame = control.read()?.completed[0]
            .ok_or("missing prior frame")?
            .frame;
        control.wait("both_old_generations_continue_under_quota", |r| {
            pair(r, previous.generation, previous.generation)
                && r.completed
                    .iter()
                    .all(|stamp| stamp.unwrap().frame > stalled_frame + 5)
        })?;
        for view in [a, replacement] {
            require(
                matches!(
                    status(data, &key, Some(revision), Some(view))?.visibility,
                    GenerationVisibility::Failed { .. }
                ),
                "unprepared revision must fail in both views",
            )?;
            data.wait_visible_with_deadline(&key, previous, Some(view), deadline())?;
        }
        control.capture(failed_name, 0, [old_color, old_color])?;
        control.gpu_headroom = None;
        control.set("configure", true, 0)?;
        control.wait(recovered_name, |r| {
            pair(r, revision.generation, revision.generation) && r.gpu_reserved == 0
        })?;
        for view in [a, replacement] {
            data.wait_visible_with_deadline(&key, revision, Some(view), deadline())?;
            require(
                status(data, &key, Some(revision), Some(view))?
                    .failure
                    .is_none(),
                "render failure was not cleared after recovery",
            )?;
        }
        control.capture(recovered_name, 0, [next_color, next_color])?;
        previous = revision;
    }
    clear(data, writer.lease())?;
    writer.close(deadline())?;
    control.set("remove", true, 0)?;
    let until = deadline();
    loop {
        let current = status(data, &observer, None, None)?.resources;
        if current.cpu_bytes == baseline.cpu_bytes
            && current.gpu_bytes == baseline.gpu_bytes
            && current.retained_datasets == baseline.retained_datasets
        {
            println!(
                "{}",
                serde_json::json!({"checkpoint":"all_probe_data_released","resources":current})
            );
            break;
        }
        if Instant::now() >= until {
            return Err(
                format!("probe resources did not return: {current:?} vs {baseline:?}").into(),
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!(
        "{}",
        serde_json::json!({"result":"pass","first":first,"last_before_clear":fifth,"fresh_incarnation":fresh,"primary":a,"secondary":b,"replacement":replacement,"completed_gpu_frames":true})
    );
    Ok(())
}

fn main() -> Result<()> {
    let instance = std::env::args()
        .nth(1)
        .ok_or("provide owned INSTANCE and PROBE_DIR")?;
    let root = PathBuf::from(std::env::args().nth(2).ok_or("provide PROBE_DIR")?).canonicalize()?;
    let entry = registry::find(&instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "draw-multiview-probe")?;
    let mut data = client.draw_data()?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(2, deadline())?;
    let mut control = Control {
        root,
        sequence: 0,
        captures: Vec::new(),
        gpu_headroom: None,
        cpu_headroom: None,
        secondary_size: None,
        secondary_east_m: None,
        hold_full: false,
    };
    let result = if std::env::args().nth(3).as_deref() == Some("auxiliary") {
        auxiliary::run(&mut control, &mut data, &mut writer)
    } else if std::env::args().nth(3).as_deref() == Some("cpu-lifecycle") {
        cpu_lifecycle::run(&mut control, &mut data, &mut writer)
    } else if std::env::args().nth(3).as_deref() == Some("upload-fairness") {
        upload_fairness::run(&mut control, &client, &mut data, &mut writer)
    } else if std::env::args().nth(3).as_deref() == Some("selection") {
        selection::run(&mut control, &mut data, &mut writer)
    } else if std::env::args().nth(3).as_deref() == Some("cloud-lod") {
        writer.close(deadline())?;
        cloud_lod::run(&mut control, &mut data, &pipeline)
    } else if std::env::args().nth(3).as_deref() == Some("residency") {
        residency::run(&mut control, &mut data, &mut writer)
    } else if std::env::args().nth(3).as_deref() == Some("legacy-admission") {
        writer.close(deadline())?;
        legacy_admission::run(&mut control, &client)
    } else {
        run(&mut control, &mut data, &mut writer)
    };
    let _ = clear(&mut data, writer.lease());
    control.gpu_headroom = None;
    control.cpu_headroom = None;
    let _ = control.set("remove", true, 0);
    let close = writer.close(deadline());
    result?;
    close?;
    Ok(())
}
