use super::*;

const POINTS: u32 = 2048;
/// One view's LOD buffers with one page slot: view uniform, bins, head, dispatch, node
/// states, four head readbacks, the node table and its pick copy.
const VIEW_LOD_BYTES: u64 = 80 + 656 + 7 * 256 + 2 * 4096;
/// Bytes of one hierarchy node in a page's LOD tree.
const TREE_NODE_BYTES: u64 = 80;

/// The page's refinement order (a u32 per point) and a whole number of tree nodes.
fn tree_and_order(bytes: u64) -> bool {
    let order = u64::from(POINTS) * 4;
    bytes > order && (bytes - order) % TREE_NODE_BYTES == 0
}

fn number(value: &serde_json::Value, field: &str) -> u64 {
    value[field].as_u64().expect("required selection counter")
}

fn presentation(rgba: [u8; 4]) -> Presentation {
    Presentation {
        color: ColorMode::Constant { rgba },
        size: PointSize::Metres { radius: 0.8 },
        glow: 0.0,
        blur: 0.0,
        opacity: 1.0,
        ..Default::default()
    }
}

fn clusters(
    writer: &mut DrawDataWriter,
    key: &DataKey,
    base: Option<DatasetRevision>,
    rgba: [u8; 4],
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
        presentation: presentation(rgba),
    };
    let payload = CloudPage {
        count: POINTS,
        columns: vec![
            ColumnData {
                values: (0..POINTS)
                    .flat_map(|i| {
                        [if i < POINTS / 2 { -1.4f64 } else { 1.4 }, 25.0, 2.0]
                            .into_iter()
                            .flat_map(f64::to_le_bytes)
                    })
                    .collect(),
                validity: vec![],
            },
            ColumnData {
                values: (0..POINTS)
                    .flat_map(|i| (u64::MAX - u64::from(i)).to_le_bytes())
                    .collect(),
                validity: vec![],
            },
        ],
    }
    .encode(&schema)?;
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: POINTS,
        byte_len: payload.len() as u64,
        crc32: page_crc32(&payload),
        origin_m: [0.0; 3],
    };
    let scans = [ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: Some(NS.into()),
        frame: DataFrame::default(),
        source_count: u64::from(POINTS),
        organization: None,
    }];
    let pages = [DataUploadPage {
        descriptor: &page,
        bytes: &payload,
    }];
    let mut request = DataUpload::replace(&descriptor, &scans, &pages);
    request.base = base;
    let reply = writer.upload(&request, deadline(), None)?;
    Ok(DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    })
}

fn active(report: &Report, revision: DatasetRevision) -> bool {
    report.completed.iter().enumerate().all(|(index, stamp)| {
        stamp.is_some_and(|stamp| {
            stamp.incarnation == revision.incarnation
                && stamp.generation == revision.generation
                && stamp.source_samples == u64::from(POINTS)
                && stamp.selected_items == 2
                && report.lod[index]["coarse"].as_bool() == Some(false)
        })
    }) && number(&report.allocations, "staging_gpu_bytes") == 0
        && number(&report.allocations, "retiring_gpu_bytes") == 0
}

fn ready(report: &Report, revision: DatasetRevision) -> bool {
    active(report, revision)
        && (0..2).all(|index| report.lod[index]["preparing"].as_bool() == Some(false))
}

pub(super) fn run(
    control: &mut Control,
    data: &mut DrawDataClient,
    writer: &mut DrawDataWriter,
) -> Result<()> {
    control.set("configure", true, 0)?;
    let baseline = control.wait("baseline", |report| {
        report.views.iter().all(Option::is_some) && number(&report.allocations, "gpu_bytes") == 0
    })?;
    let key = DataKey::new(NS, "selection")?;
    let first = clusters(writer, &key, None, [255, 24, 24, 255])?;
    let initial = control.wait("indexed-initial", |report| ready(report, first))?;
    require(
        tree_and_order(number(&initial.uploads, "indices")),
        "the page uploads exactly its LOD tree and refinement order",
    )?;
    require(
        number(&initial.allocations, "indices") == number(&initial.uploads, "indices"),
        "both views share the page's one LOD tree and refinement order",
    )?;
    require(
        number(&initial.allocations, "lod_views") == 2 * VIEW_LOD_BYTES,
        "each view owns one page slot of LOD buffers",
    )?;
    require(
        number(&initial.allocations, "selected_points") == 4,
        "two per-view cuts must each select two points",
    )?;
    for view in initial.views {
        data.wait_visible_with_deadline(&key, first, view, deadline())?;
    }
    control.capture("indexed-initial", 0, ["red", "red"])?;
    let patch = call(
        data,
        DataRequest {
            lease: writer.lease(),
            incarnation: first.incarnation,
            base_generation: first.generation,
            ..DataRequest::new(data_op::PATCH_PRESENTATION)
        }
        .with_key(&key)?
        .with_metadata(&presentation([24, 24, 255, 255]))?,
    )?;
    let second = DatasetRevision {
        incarnation: patch.incarnation,
        generation: patch.stored_generation,
    };
    let restyled = control.wait("indexed-restyled", |report| ready(report, second))?;
    for field in ["positions", "attributes", "indices"] {
        require(
            number(&restyled.uploads, field) == number(&initial.uploads, field),
            "constant color changes must not reupload source records or LOD trees",
        )?;
    }
    control.capture("indexed-restyled", 0, ["blue", "blue"])?;
    control.gpu_headroom = Some(0);
    control.set("configure", true, 0)?;
    let third = clusters(writer, &key, Some(second), [255, 24, 255, 255])?;
    let until = deadline();
    while status(data, &key, Some(third), None)?.failure.is_none() {
        require(
            Instant::now() < until,
            "replacement allocation failure was not reported",
        )?;
        std::thread::sleep(Duration::from_millis(100));
    }
    let frame = restyled.completed[0].unwrap().frame;
    control.wait("indexed-replacement-denied", |report| {
        active(report, second)
            && report
                .completed
                .iter()
                .all(|stamp| stamp.unwrap().frame > frame + 5)
    })?;
    require(
        status(data, &key, Some(third), None)?.failure.is_some(),
        "replacement allocation failure must be visible",
    )?;
    control.capture("indexed-replacement-denied", 0, ["blue", "blue"])?;
    control.gpu_headroom = None;
    control.set("configure", true, 0)?;
    let recovered = control.wait("indexed-replacement-recovered", |report| {
        ready(report, third)
    })?;
    require(
        number(&recovered.uploads, "indices") == 2 * number(&initial.uploads, "indices"),
        "replacement geometry uploads exactly one new LOD tree and refinement order",
    )?;
    require(
        number(&recovered.allocations, "indices") == number(&initial.allocations, "indices"),
        "the superseded LOD tree and refinement order must retire",
    )?;
    require(
        number(&recovered.allocations, "lod_views") == 2 * VIEW_LOD_BYTES,
        "replacement keeps each view's one page slot",
    )?;
    require(
        number(&recovered.allocations, "max_frame_upload")
            <= data.capabilities().limits.upload_bytes_per_frame,
        "LOD tree uploads exceeded the shared frame cap",
    )?;
    control.capture("indexed-replacement-recovered", 0, ["magenta", "magenta"])?;
    control.gpu_headroom = Some(0);
    control.cpu_headroom = Some(0);
    control.set("configure", true, 0)?;
    clear(data, writer.lease())?;
    let released = control.wait("indexed-clear-under-pressure", |report| {
        number(&report.allocations, "indices") == 0
            && number(&report.allocations, "lod_views") == 0
            && number(&report.allocations, "positions") == 0
            && number(&report.allocations, "attributes") == 0
            && number(&report.allocations, "retiring_gpu_bytes") == 0
            && number(&report.cpu, "page_bytes") == number(&baseline.cpu, "page_bytes")
            && number(&report.cpu, "allocated") <= number(&baseline.cpu, "allocated")
    })?;
    control.capture("indexed-cleared", 0, ["none", "none"])?;
    control.gpu_headroom = None;
    control.cpu_headroom = None;
    control.set("configure", true, 0)?;
    let final_state = control.wait("indexed-final-clear", |report| {
        number(&report.allocations, "gpu_bytes") == 0
            && number(&report.cpu, "allocated") <= number(&baseline.cpu, "allocated")
    })?;
    std::fs::write(
        control.root.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "result":"pass", "source_points":POINTS, "selected_per_view":2,
            "baseline":baseline, "initial":initial, "restyled":restyled, "recovered":recovered,
            "cleared_under_pressure":released, "final":final_state,
        }))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_uploads_hold_the_refinement_order_and_whole_tree_nodes() {
        assert!(tree_and_order(u64::from(POINTS) * 4 + 3 * TREE_NODE_BYTES));
        assert!(!tree_and_order(u64::from(POINTS) * 4));
        assert!(!tree_and_order(u64::from(POINTS) * 4 + TREE_NODE_BYTES / 2));
        assert!(!tree_and_order(16));
    }

    #[test]
    fn blocked_replacement_keeps_old_active_cut_but_is_not_a_settled_new_cut() {
        let revision = DatasetRevision {
            incarnation: 1,
            generation: 2,
        };
        let stamp = Stamp {
            view: ViewIdentity { id: 1, epoch: 1 },
            incarnation: 1,
            generation: 2,
            frame: 100,
            source_samples: u64::from(POINTS),
            selected_items: 2,
        };
        let mut report: Report = serde_json::from_value(serde_json::json!({
            "sequence": 1, "views": [null, null], "samples": [4, 1],
            "completed": [stamp, stamp], "delayed_pages": 0, "secondary_alive": true,
            "active": true, "gpu_reserved": 0, "cpu": null,
            "allocations": {"staging_gpu_bytes": 0, "retiring_gpu_bytes": 0},
            "uploads": null, "legacy": null, "scheduler": null, "backdrops": null, "motion": null,
            "lod": [{"coarse": false, "preparing": true}, {"coarse": false, "preparing": true}]
        }))
        .unwrap();
        assert!(active(&report, revision));
        assert!(!ready(&report, revision));
        assert!(!active(
            &report,
            DatasetRevision {
                generation: 3,
                ..revision
            }
        ));
        report.lod[0]["preparing"] = false.into();
        report.lod[1]["preparing"] = false.into();
        assert!(ready(&report, revision));
        report.lod[1]["coarse"] = true.into();
        assert!(!active(&report, revision));
        report.lod[1]["coarse"] = false.into();
        report.allocations["retiring_gpu_bytes"] = 16.into();
        assert!(!active(&report, revision));
    }
}
