use super::*;

const COUNTS: [u32; 3] = [100_000, 1_000_000, 5_000_000];
const PAGE: u32 = 65_536;

fn number(value: &serde_json::Value, field: &str) -> u64 {
    value[field].as_u64().expect("required cloud LOD counter")
}

pub(super) fn schema() -> CloudSchema {
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

pub(super) fn page(
    schema: &CloudSchema,
    start: u32,
    count: u32,
    total: u32,
) -> Result<(PageDescriptor, Vec<u8>)> {
    let side = (f64::from(total).sqrt().ceil()) as u32;
    let mut positions = Vec::with_capacity(count as usize * 12);
    let mut ids = Vec::with_capacity(count as usize * 8);
    for index in start..start + count {
        let x = (index % side) as f32 / (side - 1) as f32 * 30.0 - 15.0;
        let y = (index / side) as f32 / (side - 1) as f32 * 30.0 + 10.0;
        let z = 2.0 + 0.7 * (x * 0.2).sin() * (y * 0.2).cos();
        positions.extend([x, y, z].into_iter().flat_map(f32::to_le_bytes));
        ids.extend((u64::MAX - u64::from(index)).to_le_bytes());
    }
    let bytes = CloudPage {
        count,
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
    .encode(schema)?;
    let descriptor = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: u64::from(start / PAGE),
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: count,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    Ok((descriptor, bytes))
}

fn upload_case(writer: &mut DrawDataWriter, key: &DataKey, count: u32) -> Result<DatasetRevision> {
    let schema = schema();
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: key.clone(),
        shape: DataShape::Cloud {
            schema: schema.clone(),
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Constant {
                rgba: [255, 24, 24, 255],
            },
            size: PointSize::Pixels { radius: 1.7 },
            glow: 0.1,
            blur: 0.0,
            ..Default::default()
        },
    };
    let pages: Vec<_> = (0..count)
        .step_by(PAGE as usize)
        .map(|start| page(&schema, start, PAGE.min(count - start), count))
        .collect::<Result<_>>()?;
    let borrowed: Vec<_> = pages
        .iter()
        .map(|(descriptor, bytes)| DataUploadPage { descriptor, bytes })
        .collect();
    let scans = [ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: Some("synthetic/cloud-lod".into()),
        frame: DataFrame::default(),
        source_count: u64::from(count),
        organization: None,
    }];
    let reply = writer.upload(
        &DataUpload::replace(&descriptor, &scans, &borrowed),
        Instant::now() + Duration::from_secs(120),
        None,
    )?;
    Ok(DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    })
}

fn camera(instance: &str, bearing: f32, pitch: f32, distance: f32) -> Result<()> {
    let entry = registry::find(instance).ok_or("owned cloud LOD instance stopped")?;
    std::fs::write(
        registry::registry_dir().join(format!("{}.camera", entry.name)),
        format!(
            "look lat=52.3704406 lon=4.895168 bearing={bearing} pitch={pitch} distance={distance} above=2"
        ),
    )?;
    Ok(())
}

fn ready(report: &Report, revision: DatasetRevision, count: u32, after: u64) -> bool {
    report.completed.iter().enumerate().all(|(index, stamp)| {
        stamp.is_some_and(|stamp| {
            let cut = &report.lod[index];
            stamp.incarnation == revision.incarnation
                && stamp.generation == revision.generation
                && stamp.source_samples == u64::from(count)
                && stamp.selected_items > 0
                && stamp.selected_items <= 1_000_000
                && stamp.frame > after
                && cut["verified_ordinals"].as_bool() == Some(true)
                && cut["preparing"].as_bool() == Some(false)
                && cut["coarse"].as_bool() == Some(false)
                && cut["selected"].as_u64() == Some(stamp.selected_items)
        })
    }) && number(&report.allocations, "staging_gpu_bytes") == 0
        && number(&report.allocations, "retiring_gpu_bytes") == 0
}

fn check(report: &Report, limits: &DataLimits, count: u32) -> Result<()> {
    let allocated = &report.allocations;
    let resident = number(allocated, "points");
    require(
        resident <= u64::from(count),
        "resident source count exceeds retained source",
    )?;
    require(
        number(allocated, "positions") == resident * 12
            && number(allocated, "positions") == number(allocated, "demanded_position_bytes"),
        "resident positions differ from exact selected-page demand",
    )?;
    require(
        number(allocated, "attributes") == 0,
        "constant-colour pages hold no attribute words",
    )?;
    require(
        number(allocated, "gpu_bytes") <= limits.gpu_bytes,
        "GPU byte cap exceeded",
    )?;
    require(
        number(&report.cpu, "committed") <= limits.cpu_bytes,
        "CPU byte cap exceeded",
    )?;
    require(
        number(allocated, "max_frame_upload") <= limits.upload_bytes_per_frame,
        "upload frame cap exceeded",
    )?;
    let mut selected = 0;
    for cut in report.lod.as_array().ok_or("missing LOD views")? {
        require(
            number(cut, "valid") == u64::from(count),
            "LOD dropped source metadata",
        )?;
        require(
            number(cut, "represented") + number(cut, "culled") == u64::from(count),
            "LOD count partition is inconsistent",
        )?;
        require(
            number(cut, "selected") <= u64::from(limits.visible_points),
            "per-view point cap exceeded",
        )?;
        require(
            cut["verified_ordinals"].as_bool() == Some(true),
            "unverified source ordinals were published",
        )?;
        selected += number(cut, "selected");
    }
    require(
        number(allocated, "selected_points") == selected,
        "actual draw ranges differ from the selected counts",
    )?;
    Ok(())
}

fn continuous_motion(control: &mut Control, instance: &str, count: u32) -> Result<Report> {
    let poses = [
        (0, 180, 25, 10),
        (2, 150, 35, 50),
        (4, 210, 15, 32),
        (6, 180, 48, 120),
        (8, 160, 9, 29),
    ];
    let path = control.root.join(format!("motion-{count}.path"));
    let text = poses.iter().map(|(time, bearing, pitch, distance)| {
        format!("{time} look lat=52.3704406 lon=4.895168 bearing={bearing} pitch={pitch} distance={distance} above=2\n")
    }).collect::<String>();
    std::fs::write(&path, text)?;
    let started = control.set("begin_motion", true, 0)?;
    require(
        started.motion["expected_sources"] == serde_json::json!([count, count]),
        "motion audit started without both source frontiers",
    )?;
    let entry = registry::find(instance).ok_or("owned motion instance stopped")?;
    std::fs::write(
        registry::registry_dir().join(format!("{}.camera", entry.name)),
        format!("path {}", path.display()),
    )?;
    let start = Instant::now();
    for step in 1..=3 {
        std::thread::sleep(Duration::from_secs(step * 2).saturating_sub(start.elapsed()));
        control.capture(&format!("lod-{count}-motion-{step}"), 0, ["red", "red"])?;
    }
    std::thread::sleep(Duration::from_secs(9).saturating_sub(start.elapsed()));
    let ended = control.set("end_motion", true, 0)?;
    let audit = &ended.motion;
    require(
        audit["active"].as_bool() == Some(false)
            && number(audit, "frames") >= 20
            && number(audit, "frames") == number(audit, "end_frame") - number(audit, "start_frame")
            && number(audit, "failed_frames") == 0,
        "per-frame motion audit found missing, invalid, empty or over-budget cuts",
    )?;
    for view in audit["views"].as_array().ok_or("missing motion views")? {
        require(
            number(view, "frames") == number(audit, "frames")
                && number(view, "camera_changes") >= 12
                && number(view, "selection_changes") >= 2
                && number(view, "completions") > 0
                && number(view, "invalid_completions") == 0,
            "motion audit did not verify moving cameras, replaced cuts and completed draws",
        )?;
    }
    Ok(ended)
}

pub(super) fn run(
    control: &mut Control,
    data: &mut DrawDataClient,
    pipeline: &gearbox_api::DrawDataPipeline,
) -> Result<()> {
    let instance = std::env::args().nth(1).ok_or("missing owned instance")?;
    let limits = data.capabilities().limits.clone();
    require(
        limits == DataLimits::reference(),
        "cloud LOD must use unchanged reference limits",
    )?;
    control.set("configure", true, 0)?;
    let baseline = control.wait("lod-baseline", |report| {
        report.views.iter().all(Option::is_some) && number(&report.allocations, "gpu_bytes") == 0
    })?;
    let key = DataKey::new(NS, "cloud-lod")?;
    let mut cases = Vec::new();
    for count in COUNTS {
        camera(&instance, 180.0, 25.0, 10.0)?;
        let mut writer = pipeline.writer(2, deadline())?;
        let revision = upload_case(&mut writer, &key, count)?;
        writer.close(deadline())?;
        let mut frames = 0;
        let mut poses = Vec::new();
        for (name, bearing, pitch, distance) in [
            ("near", 180.0, 25.0, 10.0),
            ("far", 180.0, 48.0, 120.0),
            ("grazing", 160.0, 9.0, 29.0),
        ] {
            camera(&instance, bearing, pitch, distance)?;
            std::thread::sleep(Duration::from_millis(500));
            let report = control.wait(&format!("lod-{count}-{name}"), |report| {
                ready(report, revision, count, frames + 5)
            })?;
            check(&report, &limits, count)?;
            frames = report
                .completed
                .iter()
                .flatten()
                .map(|stamp| stamp.frame)
                .max()
                .unwrap();
            control.capture(&format!("lod-{count}-{name}"), 0, ["red", "red"])?;
            poses.push(report);
        }
        if count > limits.visible_points {
            require(
                poses.iter().any(|report| {
                    report
                        .lod
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|cut| number(cut, "budget_limited_branches") > 0)
                }),
                "5M-source scene did not exercise the point cap",
            )?;
        }
        for step in 0..20 {
            camera(
                &instance,
                150.0 + (step % 7) as f32 * 10.0,
                15.0 + (step % 4) as f32 * 5.0,
                10.0 + (step % 5) as f32 * 8.0,
            )?;
            let report = control.read()?;
            require(
                number(&report.allocations, "gpu_bytes") <= limits.gpu_bytes,
                "motion exceeded GPU cap",
            )?;
            require(
                number(&report.allocations, "max_frame_upload") <= limits.upload_bytes_per_frame,
                "motion exceeded upload cap",
            )?;
            for cut in report.lod.as_array().ok_or("missing motion LOD views")? {
                require(
                    number(cut, "selected") > 0
                        && number(cut, "selected") <= u64::from(limits.visible_points),
                    "rapid motion published an empty or oversized frontier",
                )?;
                require(
                    cut["verified_ordinals"].as_bool() == Some(true),
                    "motion published duplicate or invalid ordinals",
                )?;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let audit = continuous_motion(control, &instance, count)?;
        let moved = control.wait(&format!("lod-{count}-motion-complete"), |report| {
            ready(report, revision, count, frames + 5)
        })?;
        check(&moved, &limits, count)?;
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
        let cleared = control.wait(&format!("lod-{count}-clear"), |report| {
            number(&report.allocations, "gpu_bytes") == 0
                && number(&report.cpu, "allocated") <= number(&baseline.cpu, "allocated")
                && number(&report.cpu, "page_bytes") == 0
        })?;
        control.capture(&format!("lod-{count}-clear"), 0, ["none", "none"])?;
        cases.push(serde_json::json!({"source_points":count,"revision":revision,"poses":poses,"motion":moved,"motion_audit":audit.motion,"cleared":cleared}));
    }
    std::fs::write(
        control.root.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "result":"pass", "limits":limits,"baseline":baseline,"cases":cases,
            "scope":"native source scale, per-render-frame continuous-motion cut/resource audit, completed-draw receipts, sampled motion pixels, and exact clear; not external-provider, all lifecycle or performance acceptance",
        }))?,
    )?;
    Ok(())
}

#[test]
fn lod_source_pages_preserve_exact_ids_and_last_page_count() {
    let schema = schema();
    for total in COUNTS {
        let start = (total - 1) / PAGE * PAGE;
        let (descriptor, bytes) = page(&schema, start, total - start, total).unwrap();
        assert_eq!(descriptor.items, total - start);
        assert_eq!(descriptor.address.page_id, u64::from(start / PAGE));
        assert_eq!(descriptor.crc32, page_crc32(&bytes));
        assert!(descriptor.items > 0 && descriptor.items <= PAGE);
        let mut scratch = vec![0; descriptor.items as usize];
        let view = CloudPageView::validate(&schema, &bytes, &mut scratch).unwrap();
        assert_eq!(view.source_ids().next(), Some(u64::MAX - u64::from(start)));
        assert_eq!(
            view.source_ids().last(),
            Some(u64::MAX - u64::from(total - 1))
        );
    }
}
