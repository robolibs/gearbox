use super::*;

const PAGE: u32 = 65_536;
const BANK: u32 = 48 * PAGE;
const TOTAL: u32 = 2 * BANK;

fn number(value: &serde_json::Value, field: &str) -> u64 {
    value[field].as_u64().expect("required residency counter")
}

fn source_bank(view: &serde_json::Value, bank: u64) -> bool {
    let pages = u64::from(BANK / PAGE);
    view["source_page_bounds"] == serde_json::json!([bank * pages, (bank + 1) * pages - 1])
        && view["pages"].as_u64() == Some(pages)
        && view["represented"].as_u64() == Some(u64::from(BANK))
}

fn fixture_page(bank: u64, start: u32) -> Result<(PageDescriptor, Vec<u8>)> {
    let schema = cloud_lod::schema();
    let (mut descriptor, bytes) = cloud_lod::page(&schema, start, PAGE, BANK)?;
    let mut page = descriptor.decode_cloud(&schema, &bytes)?;
    for (offset, id) in page.columns[1].values.chunks_exact_mut(8).enumerate() {
        id.copy_from_slice(
            &(u64::MAX - bank * u64::from(BANK) - u64::from(start) - offset as u64).to_le_bytes(),
        );
    }
    let bytes = page.encode(&schema)?;
    descriptor.address.page_id += bank * u64::from(BANK / PAGE);
    descriptor.crc32 = page_crc32(&bytes);
    descriptor.origin_m = [if bank == 0 { -200.0 } else { 200.0 }, 0.0, 0.0];
    Ok((descriptor, bytes))
}

fn upload_banks(writer: &mut DrawDataWriter, key: &DataKey) -> Result<DatasetRevision> {
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: key.clone(),
        shape: DataShape::Cloud {
            schema: cloud_lod::schema(),
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Constant {
                rgba: [255, 24, 24, 255],
            },
            size: PointSize::Pixels { radius: 1.7 },
            glow: 0.1,
            ..Default::default()
        },
    };
    let mut pages = Vec::new();
    for bank in 0..2 {
        for start in (0..BANK).step_by(PAGE as usize) {
            pages.push(fixture_page(bank, start)?);
        }
    }
    let scans = [ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: Some("synthetic/residency-banks".into()),
        frame: DataFrame::default(),
        source_count: u64::from(TOTAL),
        organization: None,
    }];
    descriptor.validate_scans(&scans)?;
    let borrowed: Vec<_> = pages
        .iter()
        .map(|(descriptor, bytes)| DataUploadPage { descriptor, bytes })
        .collect();
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

fn camera(instance: &str, east: f64) -> Result<()> {
    let entry = registry::find(instance).ok_or("owned residency instance stopped")?;
    let longitude = 4.895168 + east / (111_320.0 * 52.370216_f64.to_radians().cos());
    std::fs::write(
        registry::registry_dir().join(format!("{}.camera", entry.name)),
        format!("look lat=52.3704406 lon={longitude} bearing=180 pitch=65 distance=65 above=2"),
    )?;
    Ok(())
}

fn settled(report: &Report, revision: DatasetRevision, points: u64, after: u64) -> bool {
    report.completed.iter().enumerate().all(|(index, stamp)| {
        stamp.is_some_and(|stamp| {
            let cut = &report.lod[index];
            stamp.incarnation == revision.incarnation
                && stamp.generation == revision.generation
                && stamp.frame > after
                && stamp.source_samples == u64::from(TOTAL)
                && cut["valid"].as_u64() == Some(u64::from(TOTAL))
                && cut["preparing"].as_bool() == Some(false)
                && cut["selected"].as_u64() == Some(stamp.selected_items)
                && if points == 0 {
                    stamp.selected_items == 0
                } else {
                    stamp.selected_items > 0
                }
        })
    }) && number(&report.allocations, "points") == points
        && number(&report.allocations, "positions") == points * 16
        && number(&report.allocations, "attributes") == points * 32
        && number(&report.allocations, "retiring_gpu_bytes") == 0
        && number(&report.allocations, "staging_gpu_bytes") == 0
}

fn check(report: &Report, limits: &DataLimits) -> Result<()> {
    require(
        number(&report.allocations, "gpu_bytes") <= limits.gpu_bytes,
        "residency exceeded GPU cap",
    )?;
    require(
        number(&report.cpu, "committed") <= limits.cpu_bytes,
        "residency exceeded CPU cap",
    )?;
    require(
        number(&report.allocations, "max_frame_upload") <= limits.upload_bytes_per_frame,
        "residency exceeded upload cap",
    )?;
    for cut in report.lod.as_array().ok_or("missing residency views")? {
        require(
            number(cut, "selected") <= u64::from(limits.visible_points),
            "residency exceeded selected-point cap",
        )?;
        require(
            number(cut, "represented") + number(cut, "culled") == u64::from(TOTAL),
            "residency lost source metadata",
        )?;
        require(
            cut["verified_ordinals"].as_bool() == Some(true),
            "residency published invalid ordinals",
        )?;
    }
    Ok(())
}

pub(super) fn run(
    control: &mut Control,
    data: &mut DrawDataClient,
    writer: &mut DrawDataWriter,
) -> Result<()> {
    let instance = std::env::args().nth(1).ok_or("missing owned instance")?;
    let limits = data.capabilities().limits.clone();
    let reference = DataLimits::reference();
    require(
        limits.gpu_bytes == 256 << 20
            && limits.cpu_bytes == reference.cpu_bytes
            && limits.visible_points == reference.visible_points
            && limits.upload_bytes_per_frame == reference.upload_bytes_per_frame,
        "residency requires reference limits with GEARBOX_DRAW_GPU_BYTES=256 MiB",
    )?;
    require(
        u64::from(TOTAL) * 48 > limits.gpu_bytes,
        "source fixture must exceed GPU capacity",
    )?;
    control.set("configure", true, 0)?;
    let baseline = control.wait("residency-baseline", |report| {
        report.views.iter().all(Option::is_some) && number(&report.allocations, "gpu_bytes") == 0
    })?;
    camera(&instance, -200.0)?;
    let revision = upload_banks(writer, &DataKey::new(NS, "residency")?)?;
    writer.close(deadline())?;
    let mut previous_uploads = 0;
    let mut page_loads = 0;
    let mut source_bytes = None;
    let mut frame = 0;
    let mut stages = Vec::new();
    let mut transitions = Vec::new();
    for (name, east, points) in [
        ("resident-left", -200.0, u64::from(BANK)),
        ("direct-right", 200.0, u64::from(BANK)),
        ("direct-left", -200.0, u64::from(BANK)),
        ("evicted-center", 0.0, 0),
        ("resident-right", 200.0, u64::from(BANK)),
        ("evicted-again", 0.0, 0),
        ("reloaded-left", -200.0, u64::from(BANK)),
    ] {
        camera(&instance, east)?;
        if name.starts_with("direct-") {
            let coarse_name = name.replace("direct-", "coarse-");
            let report = control.wait(&coarse_name, |report| {
                report.completed.iter().enumerate().all(|(index, stamp)| {
                    stamp.is_some_and(|stamp| {
                        stamp.frame > frame
                            && stamp.incarnation == revision.incarnation
                            && stamp.generation == revision.generation
                            && stamp.selected_items > 0
                            && report.lod[index]["coarse"].as_bool() == Some(true)
                    })
                })
            })?;
            check(&report, &limits)?;
            require(
                number(&report.allocations, "coarse_pages") == u64::from(BANK / PAGE),
                "coarse cut omitted visible source pages",
            )?;
            require(
                number(&report.allocations, "coarse_points") == u64::from(BANK / PAGE) * 256,
                "coarse cache did not deduplicate hierarchy representatives",
            )?;
            require(
                number(&report.allocations, "coarse_positions")
                    == u64::from(BANK / PAGE) * 256 * 16,
                "coarse positions retained full source pages",
            )?;
            require(
                number(&report.allocations, "coarse_attributes")
                    == u64::from(BANK / PAGE) * 256 * 32,
                "coarse attributes retained full source pages",
            )?;
            require(
                number(&report.uploads, "coarse_positions") > 0,
                "coarse transition never uploaded compact positions",
            )?;
            control.capture(&coarse_name, 0, ["red", "red"])?;
            let captured = control.read()?;
            require(
                captured.sequence == control.sequence
                    && captured.lod.as_array().is_some_and(|views| {
                        views.len() == 2
                            && views.iter().all(|view| {
                                view["coarse"].as_bool() == Some(true)
                                    && view["selected"].as_u64()
                                        == Some(u64::from(BANK / PAGE) * 256)
                            })
                    }),
                "coarse capture finished after full-cut publication",
            )?;
            check(&captured, &limits)?;
            transitions.push(captured);
        }
        std::thread::sleep(Duration::from_millis(500));
        let report = control.wait(name, |report| settled(report, revision, points, frame + 5))?;
        check(&report, &limits)?;
        page_loads += points / u64::from(PAGE);
        require(
            number(&report.allocations, "page_loads") == page_loads,
            "source pages were redundantly loaded across views",
        )?;
        require(
            number(&report.allocations, "evicted_pages") == page_loads - points / u64::from(PAGE),
            "evicted source-page counter differs from exact residency",
        )?;
        require(
            number(&report.allocations, "demanded_position_bytes") == points * 16,
            "source residency differs from selected-page demand",
        )?;
        let uploads = number(&report.uploads, "positions");
        require(
            uploads - previous_uploads == points * 16,
            "source reload bytes differ from the requested bank",
        )?;
        require(
            number(&report.uploads, "attributes") == uploads * 2,
            "position/attribute residency diverged",
        )?;
        let bytes = number(&report.cpu, "page_bytes");
        require(
            bytes >= u64::from(TOTAL) * 20,
            "source payload was silently discarded",
        )?;
        require(
            source_bytes.is_none_or(|expected| expected == bytes),
            "camera motion changed retained source bytes",
        )?;
        source_bytes = Some(bytes);
        previous_uploads = uploads;
        frame = report
            .completed
            .iter()
            .flatten()
            .map(|stamp| stamp.frame)
            .max()
            .unwrap();
        control.capture(
            name,
            0,
            if points == 0 {
                ["none", "none"]
            } else {
                ["red", "red"]
            },
        )?;
        stages.push(report);
    }
    let mut divergent = Vec::new();
    for (name, east, offset, banks) in [
        ("divergent-left-right", -200.0, 400.0, [0, 1]),
        ("divergent-right-left", 200.0, -400.0, [1, 0]),
    ] {
        camera(&instance, east)?;
        control.secondary_east_m = Some(offset);
        control.set("configure", true, 0)?;
        let report = control.wait(name, |report| {
            report.completed.iter().enumerate().all(|(index, stamp)| {
                stamp.is_some_and(|stamp| {
                    stamp.frame > frame + 5
                        && stamp.incarnation == revision.incarnation
                        && stamp.generation == revision.generation
                        && report.lod[index]["preparing"].as_bool() == Some(false)
                        && report.lod[index]["full_pages"]
                            .as_u64()
                            .is_some_and(|n| n > 0 && n < u64::from(BANK / PAGE))
                        && report.lod[index]["coarse_pages"]
                            .as_u64()
                            .is_some_and(|n| n > 0)
                        && source_bank(&report.lod[index], banks[index])
                })
            }) && number(&report.allocations, "retiring_gpu_bytes") == 0
                && number(&report.allocations, "staging_gpu_bytes") == 0
        })?;
        check(&report, &limits)?;
        let full: Vec<_> = report
            .lod
            .as_array()
            .unwrap()
            .iter()
            .map(|view| number(view, "full_pages"))
            .collect();
        require(
            full[0].abs_diff(full[1]) <= 1,
            "divergent views received unfair full-page shares",
        )?;
        require(
            number(&report.allocations, "pages")
                == full.iter().sum::<u64>() + number(&report.allocations, "coarse_pages")
                && number(&report.allocations, "positions")
                    == full.iter().sum::<u64>() * u64::from(PAGE) * 16
                        + number(&report.allocations, "coarse_positions")
                && number(&report.allocations, "attributes")
                    == full.iter().sum::<u64>() * u64::from(PAGE) * 32
                        + number(&report.allocations, "coarse_attributes"),
            "divergent banks overlap or retain unused full pages",
        )?;
        for view in report.lod.as_array().unwrap() {
            require(
                number(view, "full_pages") + number(view, "coarse_pages") == u64::from(BANK / PAGE),
                "mixed cut omitted source pages",
            )?;
        }
        control.capture(name, 0, ["red", "red"])?;
        let captured = control.read()?;
        check(&captured, &limits)?;
        require(
            captured.sequence == control.sequence
                && banks.iter().enumerate().all(|(index, bank)| {
                    source_bank(&captured.lod[index], *bank)
                        && captured.lod[index]["preparing"].as_bool() == Some(false)
                }),
            "divergent readback did not preserve the settled source banks",
        )?;
        frame = captured
            .completed
            .iter()
            .flatten()
            .map(|stamp| stamp.frame)
            .max()
            .unwrap();
        divergent.push(captured);
    }
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
    let cleared = control.wait("residency-cleared", |report| {
        number(&report.allocations, "gpu_bytes") == 0
            && number(&report.cpu, "allocated") == number(&baseline.cpu, "allocated")
            && number(&report.cpu, "page_bytes") == 0
    })?;
    control.capture("residency-cleared", 0, ["none", "none"])?;
    std::fs::write(
        control.root.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "result":"pass", "source_points":TOTAL, "bank_points":BANK, "limits":limits,
            "baseline":baseline, "stages":stages, "transitions":transitions, "divergent":divergent, "cleared":cleared,
            "scope":"source eviction/reload, direct bank transitions and two divergent mixed-page views; not external-provider or every-frame continuous-motion acceptance"
        }))?,
    )?;
    Ok(())
}

#[test]
fn residency_readiness_rejects_the_previous_camera_bank() {
    for bank in 0..2 {
        let pages = u64::from(BANK / PAGE);
        let mut view = serde_json::json!({
            "source_page_bounds": [bank * pages, (bank + 1) * pages - 1],
            "pages": pages,
            "represented": BANK,
        });
        assert!(source_bank(&view, bank));
        assert!(!source_bank(&view, 1 - bank));
        view["pages"] = (pages - 1).into();
        assert!(!source_bank(&view, bank));
        view["pages"] = pages.into();
        view["source_page_bounds"] = serde_json::Value::Null;
        assert!(!source_bank(&view, bank));
    }
}

#[test]
fn residency_banks_exceed_gpu_capacity_and_preserve_unique_page_source_identity() {
    assert!(u64::from(TOTAL) * 48 > 256 << 20);
    for bank in 0..2 {
        for start in [0, BANK - PAGE] {
            let (descriptor, bytes) = fixture_page(bank, start).unwrap();
            assert_eq!(descriptor.address.scan_key, 0);
            assert_eq!(
                descriptor.address.page_id,
                bank * u64::from(BANK / PAGE) + u64::from(start / PAGE)
            );
            assert_eq!(descriptor.items, PAGE);
            assert_eq!(descriptor.crc32, page_crc32(&bytes));
            let decoded = descriptor
                .decode_cloud(&cloud_lod::schema(), &bytes)
                .unwrap();
            let ids = &decoded.columns[1].values;
            assert_eq!(
                u64::from_le_bytes(ids[..8].try_into().unwrap()),
                u64::MAX - bank * u64::from(BANK) - u64::from(start)
            );
            assert_eq!(
                u64::from_le_bytes(ids[ids.len() - 8..].try_into().unwrap()),
                u64::MAX - bank * u64::from(BANK) - u64::from(start + PAGE - 1)
            );
        }
    }
}
