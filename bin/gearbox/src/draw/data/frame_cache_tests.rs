use super::*;

fn reframe(budget: &CpuBudget, entry: &Retained, change: impl FnOnce(&mut DataFrame)) -> Retained {
    let mut scan = entry.generation.scans()[0].value().clone();
    change(&mut scan.frame);
    let scan = ScanMetadata::new(budget, &scan).unwrap();
    Retained {
        lifetime: entry.lifetime.clone(),
        revision: DatasetRevision {
            generation: entry.revision.generation + 1,
            ..entry.revision
        },
        generation: DatasetStorage::from_resident(
            budget,
            Generation::build(
                budget,
                DatasetMetadata::new(budget, entry.generation.descriptor()).unwrap(),
                &[scan],
                entry.generation.resident().unwrap().pages(),
            )
            .unwrap(),
        )
        .unwrap(),
    }
}

fn stamp(time: u64) -> CaptureStamp {
    CaptureStamp {
        simulation_time_s: 0.0,
        source_time_ns: Some(time),
        source_clock: Some("fixture".into()),
        sample: 1,
        scene_epoch: 1,
    }
}

#[test]
fn frame_metadata_edits_reuse_constant_records_and_recompute_clipping() {
    let budget = CpuBudget::new(16 << 20);
    let serial = AtomicU64::new(1);
    let mut entry = retained(&budget);
    let mut style = entry.generation.descriptor().presentation.clone();
    style.clip = Some(ClipBounds {
        min_m: [-0.1, -0.1, 0.9],
        max_m: [0.1, 0.1, 1.1],
        ..Default::default()
    });
    entry.generation = entry.generation.restyle(&budget, &style).unwrap();
    let first = prepare_dataset(&budget, &entry, None, 1, &serial).unwrap();
    let next = reframe(&budget, &entry, |frame| {
        frame.pose.translation_m[0] = 10.0;
        frame.capture = Some(stamp(100));
    });
    let second = prepare_dataset(&budget, &next, Some(&first), 1, &serial).unwrap();
    assert!(Arc::ptr_eq(
        &first.pages[0].appearance,
        &second.pages[0].appearance
    ));
    assert!(!Arc::ptr_eq(&first.pages[0], &second.pages[0]));
    assert_eq!(first.pages[0].clip.selected, 1);
    assert_eq!(second.pages[0].clip.selected, 0);
    assert_eq!(
        second.pages[0].scan.value().frame.pose.translation_m[0],
        10.0
    );
    drop((entry, next, first, second));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn source_axis_pose_edits_rebuild_values_but_share_positions() {
    let budget = CpuBudget::new(16 << 20);
    let serial = AtomicU64::new(1);
    let mut entry = retained(&budget);
    let mut style = entry.generation.descriptor().presentation.clone();
    style.color = ColorMode::Axis {
        axis: 2,
        colormap: gearbox_api::colormap::VIRIDIS,
        range: [0.0, 8.0],
    };
    entry.generation = entry.generation.restyle(&budget, &style).unwrap();
    let first = prepare_dataset(&budget, &entry, None, 1, &serial).unwrap();
    let next = reframe(&budget, &entry, |frame| frame.pose.translation_m[2] = 2.0);
    let second = prepare_dataset(&budget, &next, Some(&first), 1, &serial).unwrap();
    assert!(Arc::ptr_eq(
        &first.pages[0].appearance.geometry,
        &second.pages[0].appearance.geometry
    ));
    assert!(!Arc::ptr_eq(
        &first.pages[0].appearance,
        &second.pages[0].appearance
    ));
    assert_eq!(first.pages[0].appearance.samples()[0].value, 0.125);
    assert_eq!(second.pages[0].appearance.samples()[0].value, 0.375);
}

#[test]
fn changed_acquisition_origin_rechecks_offset_overflow() {
    let budget = CpuBudget::new(16 << 20);
    let serial = AtomicU64::new(1);
    let base = retained(&budget);
    let DataShape::Cloud { mut schema } = base.generation.descriptor().shape.clone() else {
        panic!()
    };
    let original = base.generation.resident().unwrap().pages()[0].bytes();
    let mut cloud = CloudPage::decode(&schema, original).unwrap();
    schema.attributes.push(Attribute {
        name: "offset".into(),
        scalar: ScalarType::U64,
        components: 1,
        semantic: AttributeSemantic::AcquisitionOffset,
        units: "ns".into(),
        normalization: Normalization::None,
        nullable: false,
    });
    cloud.columns.push(ColumnData {
        values: [u64::MAX, 0]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect(),
        validity: vec![],
    });
    let page = super::super::super::tests::stored(&budget, &schema, &cloud, [0.0; 3]);
    let mut desc = base.generation.descriptor().clone();
    desc.shape = DataShape::Cloud { schema };
    desc.presentation.color = ColorMode::Age {
        clock: AgeClock::Source,
        colormap: gearbox_api::colormap::VIRIDIS,
        range: [0.0, 1.0],
    };
    let mut scan = base.generation.scans()[0].value().clone();
    scan.frame.capture = Some(stamp(0));
    let entry = Retained {
        lifetime: base.lifetime.clone(),
        revision: base.revision,
        generation: DatasetStorage::from_resident(
            &budget,
            Generation::build(
                &budget,
                DatasetMetadata::new(&budget, &desc).unwrap(),
                &[ScanMetadata::new(&budget, &scan).unwrap()],
                &[page],
            )
            .unwrap(),
        )
        .unwrap(),
    };
    let first = prepare_dataset(&budget, &entry, None, 1, &serial).unwrap();
    let placed = reframe(&budget, &entry, |frame| frame.pose.translation_m[0] = 5.0);
    let second = prepare_dataset(&budget, &placed, Some(&first), 1, &serial).unwrap();
    assert!(Arc::ptr_eq(
        &first.pages[0].appearance,
        &second.pages[0].appearance
    ));
    let overflow = reframe(&budget, &placed, |frame| {
        frame.capture.as_mut().unwrap().source_time_ns = Some(1)
    });
    let result = prepare_dataset(&budget, &overflow, Some(&second), 1, &serial);
    assert!(
        matches!(result, Err(error) if error.to_string().contains("source acquisition time overflow"))
    );
}
