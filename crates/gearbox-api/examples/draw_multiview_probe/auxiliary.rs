use super::*;

fn number(value: &serde_json::Value, field: &str) -> u64 {
    value[field]
        .as_u64()
        .expect("required native allocation observation")
}

fn settled(report: &Report) -> bool {
    let a = &report.allocations;
    let resident: u64 = ["positions", "attributes", "styles", "placements"]
        .into_iter()
        .map(|field| number(a, field))
        .sum();
    number(a, "other_gpu_bytes") == report.gpu_reserved
        && number(a, "staging_gpu_bytes") == 0
        && number(a, "retiring_gpu_bytes") == 0
        && number(a, "gpu_bytes")
            == resident + number(a, "auxiliary_gpu_bytes") + number(a, "other_gpu_bytes")
        && report.backdrops.as_array().is_some_and(|views| {
            views.iter().map(|view| number(view, "bytes")).sum::<u64>()
                == number(a, "auxiliary_gpu_bytes")
        })
}

fn wait_settled(control: &Control, name: &str, accept: impl Fn(&Report) -> bool) -> Result<Report> {
    let before = control.read()?.completed[0].map_or(0, |stamp| stamp.frame);
    control.wait(name, |report| {
        settled(report)
            && accept(report)
            && report.completed[0].is_some_and(|stamp| stamp.frame > before + 5)
    })
}

fn edit(
    data: &mut DrawDataClient,
    key: &DataKey,
    base: DatasetRevision,
    blur: f32,
) -> Result<DatasetRevision> {
    let presentation = Presentation {
        color: ColorMode::Constant {
            rgba: [255, 24, 255, 190],
        },
        size: PointSize::Metres { radius: 0.8 },
        glow: 0.0,
        blur,
        opacity: 1.0,
        ..Default::default()
    };
    let lease = call(data, DataRequest::new(data_op::OPEN_WRITER))?.lease;
    let result = call(
        data,
        DataRequest {
            lease,
            incarnation: base.incarnation,
            base_generation: base.generation,
            ..DataRequest::new(data_op::PATCH_PRESENTATION)
        }
        .with_key(key)?
        .with_metadata(&presentation)?,
    );
    let close = call(
        data,
        DataRequest {
            lease,
            ..DataRequest::new(data_op::CLOSE_WRITER)
        },
    );
    let reply = result?;
    close?;
    Ok(DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    })
}

fn record(control: &Control, name: &str, report: &Report) -> Result<()> {
    let limit = 256 * 1024 * 1024;
    require(
        number(&report.allocations, "gpu_bytes") <= limit
            && number(&report.allocations, "gpu_peak") <= limit,
        "auxiliary allocation exceeded the shared GPU cap",
    )?;
    for view in report
        .backdrops
        .as_array()
        .ok_or("missing native backdrop observations")?
    {
        let size = view["size"]
            .as_array()
            .ok_or("missing texture dimensions")?;
        require(
            number(view, "bytes")
                == size[0].as_u64().unwrap() * size[1].as_u64().unwrap() * number(view, "stride"),
            "actual texture extent/format disagree with auxiliary bytes",
        )?;
    }
    std::fs::write(
        control.root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(report)?,
    )?;
    Ok(())
}

pub(super) fn run(
    control: &mut Control,
    data: &mut DrawDataClient,
    writer: &mut DrawDataWriter,
) -> Result<()> {
    let key = DataKey::new(NS, "cloud")?;
    control.set("configure", true, 0)?;
    let first = upload(writer, &key, None, [255, 24, 24, 255])?;
    writer.close(deadline())?;
    let initial = wait_settled(control, "opaque", |r| {
        pair(r, first.generation, first.generation)
    })?;
    require(
        number(&initial.allocations, "auxiliary_gpu_bytes") == 0,
        "opaque blur-zero cloud allocated a backdrop",
    )?;
    record(control, "opaque", &initial)?;
    control.capture("opaque", 0, ["red", "red"])?;

    control.gpu_headroom = Some(65_536);
    control.set("configure", true, 0)?;
    let frosted = edit(data, &key, first, 0.8)?;
    let until = deadline();
    loop {
        let current = status(data, &key, Some(frosted), initial.views[0])?;
        if current.failure.as_ref().is_some_and(|failure| {
            failure.phase == DataPhase::Rendering
                && failure.code == data_code::BUDGET
                && failure.reason.contains("frost GPU auxiliary")
        }) {
            std::fs::write(
                control.root.join("denied-status.json"),
                serde_json::to_vec_pretty(&current)?,
            )?;
            break;
        }
        if Instant::now() >= until {
            return Err(format!("missing auxiliary-budget failure: {current:?}").into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let denied = wait_settled(control, "denied", |r| {
        pair(r, first.generation, first.generation)
            && r.gpu_reserved > 0
            && number(&r.allocations, "auxiliary_gpu_bytes") == 0
    })?;
    require(
        denied.uploads["positions"] == initial.uploads["positions"]
            && denied.uploads["attributes"] == initial.uploads["attributes"],
        "frost-only replacement rebuilt point records",
    )?;
    record(control, "denied", &denied)?;
    control.capture("denied", 0, ["red", "red"])?;

    control.gpu_headroom = None;
    control.set("configure", true, 0)?;
    let recovered = wait_settled(control, "recovered", |r| {
        pair(r, frosted.generation, frosted.generation)
            && r.gpu_reserved == 0
            && r.backdrops
                .as_array()
                .is_some_and(|views| views.iter().all(|view| number(view, "bytes") > 0))
    })?;
    for view in recovered.views {
        data.wait_visible_with_deadline(&key, frosted, view, deadline())?;
        let current = status(data, &key, Some(frosted), view)?;
        require(
            current.failure.is_none(),
            "auxiliary failure survived successful recovery",
        )?;
        require(
            current.resources.gpu_bytes == number(&recovered.allocations, "gpu_bytes"),
            "public STATUS omits auxiliary allocations",
        )?;
    }
    record(control, "recovered", &recovered)?;
    control.capture("recovered", 0, ["magenta", "magenta"])?;

    control.secondary_size = Some([960, 540]);
    control.set("recreate", true, 0)?;
    let resized = wait_settled(control, "resized", |r| {
        pair(r, frosted.generation, frosted.generation)
            && r.views[1] != recovered.views[1]
            && r.backdrops[1]["size"] == serde_json::json!([960, 540])
    })?;
    require(
        resized.uploads["positions"] == initial.uploads["positions"]
            && resized.uploads["attributes"] == initial.uploads["attributes"],
        "view resize reuploaded unchanged records",
    )?;
    require(
        number(&resized.backdrops[0], "bytes") == number(&recovered.backdrops[0], "bytes"),
        "secondary resize changed primary backdrop",
    )?;
    record(control, "resized", &resized)?;
    control.capture("resized", 0, ["magenta", "magenta"])?;

    let sharp = edit(data, &key, frosted, 0.0)?;
    let disabled = wait_settled(control, "disabled", |r| {
        pair(r, sharp.generation, sharp.generation)
            && number(&r.allocations, "auxiliary_gpu_bytes") == 0
    })?;
    record(control, "disabled", &disabled)?;
    control.capture("disabled", 0, ["magenta", "magenta"])?;
    let lease = call(data, DataRequest::new(data_op::OPEN_WRITER))?.lease;
    let cleared = clear(data, lease);
    let closed = call(
        data,
        DataRequest {
            lease,
            ..DataRequest::new(data_op::CLOSE_WRITER)
        },
    );
    cleared?;
    closed?;
    let cleared = control.wait("cleared", |r| {
        settled(r)
            && number(&r.allocations, "gpu_bytes") == 0
            && number(&r.allocations, "points") == 0
            && number(&r.allocations, "pages") == 0
    })?;
    record(control, "cleared", &cleared)?;
    control.capture("cleared", 0, ["none", "none"])?;
    std::fs::write(
        control.root.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "result":"pass", "scope":"native auxiliary allocation, admission, recovery, view resize and release",
            "initial_uploads":initial.uploads,"final_uploads":disabled.uploads,
            "recovered_auxiliary_bytes":number(&recovered.allocations,"auxiliary_gpu_bytes"),
            "resized_auxiliary_bytes":number(&resized.allocations,"auxiliary_gpu_bytes")
        }))?,
    )?;
    Ok(())
}
