use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use gearbox_api::wire::draw_data::*;
use gearbox_api::{
    Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawDataClient, DrawDataWriter,
    IdentitySource, clock_op, registry,
};
use serde::{Deserialize, Serialize};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-gallery/typed-frame-contract";
#[path = "draw_cloud_frames/fixture.rs"]
mod fixture;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

fn call(data: &mut DrawDataClient, mut request: DataRequest) -> Result<DataReply> {
    request.host_epoch = data.capabilities().host_epoch;
    let reply = data.request_with_deadline(&request, deadline())?;
    if reply.code != data_code::OK {
        return Err(format!(
            "API {}: {}",
            reply.code,
            String::from_utf8_lossy(&reply.message)
        )
        .into());
    }
    Ok(reply)
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

fn upload(
    writer: &mut DrawDataWriter,
    value: &fixture::Fixture,
    base: Option<DatasetRevision>,
) -> Result<DatasetRevision> {
    let pages = [DataUploadPage {
        descriptor: &value.page,
        bytes: &value.bytes,
    }];
    let mut upload = DataUpload::replace(
        &value.descriptor,
        std::slice::from_ref(&value.scan),
        if base.is_some() { &[] } else { &pages },
    );
    upload.base = base;
    if base.is_some() {
        upload.mode = CommitMode::Delta;
    }
    let reply = writer.upload(&upload, deadline(), None)?;
    Ok(DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
struct Uploads {
    positions: u64,
    attributes: u64,
    styles: u64,
    placements: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Allocations {
    positions: u64,
    attributes: u64,
    styles: u64,
    placements: u64,
    points: u64,
    pages: u64,
    gpu_bytes: u64,
    auxiliary_gpu_bytes: u64,
    staging_gpu_bytes: u64,
    retiring_gpu_bytes: u64,
    other_gpu_bytes: u64,
    uploaded: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Metrics {
    allocations: Allocations,
    uploads: Uploads,
}

fn metrics(out: &Path, points: u64, accept: impl Fn(&Metrics) -> bool) -> Result<Metrics> {
    let until = deadline();
    let mut previous = None;
    let mut stable = 0;
    loop {
        if let Ok(bytes) = std::fs::read(out.join("report.json"))
            && let Ok(value) = serde_json::from_slice::<Metrics>(&bytes)
        {
            let a = &value.allocations;
            let valid = a.points == points
                && a.positions == points * 16
                && a.attributes == points * 32
                && a.pages == points / u64::from(fixture::POINTS)
                && a.gpu_bytes == a.positions + a.attributes + a.styles + a.placements + a.auxiliary_gpu_bytes
                && a.staging_gpu_bytes == 0 && a.retiring_gpu_bytes == 0 && a.other_gpu_bytes == 0
                && a.uploaded
                    == value.uploads.positions
                        + value.uploads.attributes
                        + value.uploads.styles
                        + value.uploads.placements;
            if valid && accept(&value) && previous.as_ref() == Some(&value.uploads) {
                stable += 1;
            } else {
                stable = 0;
            }
            previous = Some(value.uploads.clone());
            if stable >= 3 {
                return Ok(value);
            }
        }
        if Instant::now() >= until {
            return Err(format!("frame probe metrics did not settle: {previous:?}").into());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn cli(bin: &str, instance: &str, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("timeout")
        .args(["90", bin, "--instance", instance, "--timeout", "30"])
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!("CLI {args:?}: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(output.stdout)
}

struct Capture<'a> {
    bin: &'a str,
    instance: &'a str,
    machine: &'a str,
    out: &'a Path,
}

impl Capture<'_> {
    fn stage(
        &self,
        stage: &str,
        data: &mut DrawDataClient,
        fixtures: &[fixture::Fixture],
        revisions: &[DatasetRevision],
        accept: impl Fn(&Metrics) -> bool,
    ) -> Result<Metrics> {
        let mut receipts = Vec::new();
        for (value, &revision) in fixtures.iter().zip(revisions) {
            let visible = data
                .wait_visible_with_deadline(&value.descriptor.key, revision, None, deadline())?
                .visibility();
            if !matches!(
                visible,
                GenerationVisibility::Visible {
                    disposition: FrontierDisposition::Drawn,
                    source_samples: 27,
                    selected_items: 27,
                    ..
                }
            ) {
                return Err("frame marker did not complete a drawn frame".into());
            }
            receipts.push(serde_json::json!({"key":value.descriptor.key,"scan":value.scan,"visibility":visible}));
        }
        let value = metrics(
            self.out,
            u64::from(fixture::POINTS) * fixtures.len() as u64,
            accept,
        )?;
        let screenshot = self.out.join(format!("{stage}.png"));
        cli(
            self.bin,
            self.instance,
            &[
                "instance",
                "screenshot",
                screenshot.to_str().ok_or("non-UTF8 path")?,
                "--viewport",
                "--timeout",
                "30",
            ],
        )?;
        let pose = cli(
            self.bin,
            self.instance,
            &["--json", "scene", "pose", self.machine],
        )?;
        std::fs::write(self.out.join(format!("{stage}.machine.json")), pose)?;
        let report = serde_json::json!({"stage":stage,"metrics":value,"datasets":receipts});
        std::fs::write(
            self.out.join(format!("{stage}.json")),
            serde_json::to_vec_pretty(&report)?,
        )?;
        println!("{report}");
        Ok(value)
    }
}

fn run(client: &Client, capture: &Capture) -> Result<()> {
    let home = client
        .info()?
        .props()
        .get("home")
        .ok_or("missing home datum")?;
    let coordinates: Vec<f64> = home
        .split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    if coordinates.len() != 2 || !coordinates.iter().all(|v| v.is_finite()) {
        return Err("invalid home datum".into());
    }
    let datum = fixture::datum(coordinates[0], coordinates[1]);
    let mut fixtures: Vec<_> = (0..4)
        .map(|i| fixture::make(i, capture.machine, datum))
        .collect::<Result<_>>()?;
    let mut data = client.draw_data()?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(4, deadline())?;
    let original = client.clock(clock_op::GET)?;
    let result = (|| -> Result<()> {
        client.clock(clock_op::PAUSE)?;
        clear(&mut data, writer.lease())?;
        let mut revisions: Vec<_> = fixtures
            .iter()
            .map(|f| upload(&mut writer, f, None))
            .collect::<Result<_>>()?;
        writer.close(deadline())?;
        let initial = capture.stage("01-initial", &mut data, &fixtures, &revisions, |_| true)?;
        if initial.uploads.positions != initial.allocations.positions
            || initial.uploads.attributes != initial.allocations.attributes
        {
            return Err("initial point records uploaded more than once".into());
        }
        client.clock(clock_op::PLAY)?;
        cli(
            capture.bin,
            capture.instance,
            &[
                "machine",
                "move",
                capture.machine,
                "--forward",
                "1",
                "--distance",
                "3",
            ],
        )?;
        client.clock(clock_op::PAUSE)?;
        let moved = capture.stage("02-frame-moved", &mut data, &fixtures, &revisions, |m| {
            m.uploads.placements > initial.uploads.placements
        })?;
        if moved.uploads.styles != initial.uploads.styles {
            return Err("live motion reuploaded styles".into());
        }
        fixtures[3].descriptor.presentation.glow = 0.27;
        fixtures[3].descriptor.presentation.blur = 0.7;
        writer = pipeline.writer(4, deadline())?;
        let reply = call(
            &mut data,
            DataRequest {
                lease: writer.lease(),
                incarnation: revisions[3].incarnation,
                base_generation: revisions[3].generation,
                ..DataRequest::new(data_op::PATCH_PRESENTATION)
            }
            .with_key(&fixtures[3].descriptor.key)?
            .with_metadata(&fixtures[3].descriptor.presentation)?,
        )?;
        revisions[3] = DatasetRevision {
            incarnation: reply.incarnation,
            generation: reply.stored_generation,
        };
        writer.close(deadline())?;
        let styled = capture.stage(
            "03-snapshot-restyled",
            &mut data,
            &fixtures,
            &revisions,
            |m| m.uploads.styles > moved.uploads.styles,
        )?;
        fixtures[3].scan.frame.pose.translation_m[0] += 1.0;
        writer = pipeline.writer(4, deadline())?;
        revisions[3] = upload(&mut writer, &fixtures[3], Some(revisions[3]))?;
        writer.close(deadline())?;
        let posed = capture.stage(
            "04-snapshot-local-pose",
            &mut data,
            &fixtures,
            &revisions,
            |m| m.uploads.styles > styled.uploads.styles,
        )?;
        fixture::replace_capture(&mut fixtures[3].scan.frame)?;
        writer = pipeline.writer(4, deadline())?;
        revisions[3] = upload(&mut writer, &fixtures[3], Some(revisions[3]))?;
        writer.close(deadline())?;
        let replaced = capture.stage(
            "05-capture-replaced",
            &mut data,
            &fixtures,
            &revisions,
            |m| m.uploads.styles > posed.uploads.styles,
        )?;
        for value in [&moved, &styled, &posed, &replaced] {
            if value.uploads.positions != initial.uploads.positions
                || value.uploads.attributes != initial.uploads.attributes
            {
                return Err("motion or metadata edit reuploaded point records".into());
            }
        }
        writer = pipeline.writer(4, deadline())?;
        call(
            &mut data,
            DataRequest {
                lease: writer.lease(),
                incarnation: revisions[3].incarnation,
                base_generation: revisions[3].generation,
                ..DataRequest::new(data_op::DELETE)
            }
            .with_key(&fixtures[3].descriptor.key)?,
        )?;
        writer.close(deadline())?;
        let mut previous_styles = replaced.uploads.styles;
        for (stage, position) in [
            ("06-reference-initial", [-3.0, 3.0, 3.0]),
            ("07-reference-local", [-2.0, 3.0, 3.0]),
            ("08-reference-capture", [1.0, -2.0, 3.0]),
        ] {
            fixtures[0].scan.frame.pose.translation_m = position;
            writer = pipeline.writer(4, deadline())?;
            revisions[0] = upload(&mut writer, &fixtures[0], Some(revisions[0]))?;
            writer.close(deadline())?;
            let reference =
                capture.stage(stage, &mut data, &fixtures[..3], &revisions[..3], |m| {
                    m.uploads.styles > previous_styles
                })?;
            if reference.uploads.positions != initial.uploads.positions
                || reference.uploads.attributes != initial.uploads.attributes
            {
                return Err("world reference update reuploaded point records".into());
            }
            previous_styles = reference.uploads.styles;
        }
        writer = pipeline.writer(4, deadline())?;
        clear(&mut data, writer.lease())?;
        writer.close(deadline())?;
        let cleared = metrics(capture.out, 0, |_| true)?;
        let image = capture.out.join("09-clear.png");
        cli(
            capture.bin,
            capture.instance,
            &[
                "instance",
                "screenshot",
                image.to_str().ok_or("non-UTF8 path")?,
                "--viewport",
                "--timeout",
                "30",
            ],
        )?;
        std::fs::write(
            capture.out.join("09-clear.json"),
            serde_json::to_vec_pretty(&cleared)?,
        )?;
        Ok(())
    })();
    let cleanup = (|| -> Result<()> {
        let mut cleanup = pipeline.writer(4, deadline())?;
        let cleared = clear(&mut data, cleanup.lease());
        let closed = cleanup.close(deadline());
        cleared?;
        closed?;
        Ok(())
    })();
    let restored = client.clock(if original.paused != 0 {
        clock_op::PAUSE
    } else {
        clock_op::PLAY
    });
    result?;
    cleanup?;
    restored?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 4 || !args[0].starts_with("draw-vfx-cloud-frames-") {
        return Err("provide owned FRAME_INSTANCE MACHINE OUTPUT BIN".into());
    }
    let entry = registry::find(&args[0]).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "typed-cloud-frames")?;
    run(
        &client,
        &Capture {
            instance: &args[0],
            machine: &args[1],
            out: Path::new(&args[2]),
            bin: &args[3],
        },
    )
}
