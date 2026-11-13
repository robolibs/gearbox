//! Sparse volumes against analytical fields. Tolerances follow from the
//! fixture: values are stored as f32, so a linear field reads back to f32
//! rounding; a sphere's distance has curvature at most 1/d at distance d from
//! its centre, so trilinear reads err by at most 3h²/(8d) and central
//! differences one voxel apart by √3 times that over h plus h²/(2d²).

use gearbox_api::draw_data::*;
use gearbox_api::wire::draw_data::volume::*;
use gearbox_api::wire::draw_data::*;

const EPOCH: u64 = 29;
const PLANE_Z: f64 = 0.37;
const CENTRE: [f64; 3] = [0.1, -0.2, 0.3];
const RADIUS: f64 = 0.6;

fn plane(p: [f64; 3]) -> f64 {
    p[2] - PLANE_Z
}

fn distance(p: [f64; 3]) -> f64 {
    (0..3).map(|a| (p[a] - CENTRE[a]).powi(2)).sum::<f64>().sqrt()
}

fn sphere(p: [f64; 3]) -> f64 {
    distance(p) - RADIUS
}

fn esdf(voxel_m: f64) -> VolumeField {
    VolumeField { kind: FieldKind::Esdf { signed: true }, voxel_m, block_side: 8, colors: false }
}

/// Block `key` of `f`, valid where `known` holds, weighted 1 for a TSDF.
fn block(field: &VolumeField, key: [i32; 3], f: impl Fn([f64; 3]) -> f64, known: impl Fn([f64; 3]) -> bool) -> VolumeBlock {
    let side = usize::from(field.block_side);
    let n = side.pow(3);
    let mut out = VolumeBlock {
        key,
        values: vec![0.0; n],
        valid: vec![false; n],
        weights: field.weights().then(|| vec![0.0; n]),
        colors: field.colors.then(|| vec![[0; 4]; n]),
    };
    for i in 0..n {
        let local = [i % side, i / side % side, i / (side * side)];
        let p = [0, 1, 2].map(|a| ((i64::from(key[a]) * side as i64 + local[a] as i64) as f64 + 0.5) * field.voxel_m);
        if !known(p) {
            continue;
        }
        let mut value = f(p);
        if let FieldKind::Tsdf { truncation_m } = field.kind {
            value = value.clamp(-f64::from(truncation_m), f64::from(truncation_m));
        }
        out.valid[i] = true;
        out.values[i] = value as f32;
        if let Some(weights) = &mut out.weights {
            weights[i] = 1.0;
        }
        if let Some(colors) = &mut out.colors {
            colors[i] = [200, 100, 50, 255];
        }
    }
    out
}

fn keys(x: std::ops::RangeInclusive<i32>, y: std::ops::RangeInclusive<i32>, z: std::ops::RangeInclusive<i32>) -> Vec<[i32; 3]> {
    let mut out = Vec::new();
    for a in x.clone() {
        for b in y.clone() {
            for c in z.clone() {
                out.push([a, b, c]);
            }
        }
    }
    out
}

fn encoded(field: &VolumeField, blocks: &[VolumeBlock]) -> Vec<Vec<u8>> {
    blocks.iter().map(|b| encode_volume_block(field, b).unwrap()).collect()
}

fn volume<'a>(field: &'a VolumeField, bytes: &'a [Vec<u8>]) -> Volume<'a> {
    Volume::new(field, bytes.iter().map(|b| BlockView::new(field, b).unwrap()))
}

#[test]
fn volume_blocks_roundtrip_and_refuse_malformed_samples() {
    let tsdf = VolumeField { kind: FieldKind::Tsdf { truncation_m: 0.2 }, voxel_m: 0.05, block_side: 8, colors: true };
    let good = block(&tsdf, [-1, 2, 0], sphere, |p| p[0] > -0.3);
    let bytes = encode_volume_block(&tsdf, &good).unwrap();
    assert_eq!(decode_volume_block(&tsdf, &bytes).unwrap(), good);
    let layout = volume_layout(&tsdf, &bytes).unwrap();
    assert_eq!(layout.valid_count as usize, good.valid.iter().filter(|v| **v).count());
    let first = good.valid.iter().position(|v| *v).unwrap();
    let cases: Vec<(&str, VolumeBlock)> = vec![
        ("short", VolumeBlock { values: vec![0.0; 511], ..good.clone() }),
        ("missing weights", VolumeBlock { weights: None, ..good.clone() }),
        ("missing colours", VolumeBlock { colors: None, ..good.clone() }),
        ("beyond truncation", {
            let mut b = good.clone();
            b.values[first] = 0.5;
            b
        }),
        ("zero weight", {
            let mut b = good.clone();
            b.weights.as_mut().unwrap()[first] = 0.0;
            b
        }),
        ("NaN", {
            let mut b = good.clone();
            b.values[first] = f32::NAN;
            b
        }),
    ];
    for (case, block) in cases {
        assert!(encode_volume_block(&tsdf, &block).is_err(), "{case} was accepted");
    }
    let unsigned = VolumeField { kind: FieldKind::Esdf { signed: false }, ..esdf(0.1) };
    assert!(encode_volume_block(&unsigned, &block(&unsigned, [0, 0, 0], plane, |_| true)).is_err(), "negative unsigned clearance");
    encode_volume_block(&esdf(0.1), &block(&esdf(0.1), [0, 0, 0], plane, |_| true)).unwrap();
    let mut noisy = bytes.clone();
    let unknown = good.valid.iter().position(|v| !*v).unwrap();
    noisy[24 + unknown * 4] = 1;
    assert!(volume_layout(&tsdf, &noisy).is_err(), "an unknown sample must hold canonical zeros");
    let mut other = bytes.clone();
    other[16] = 16;
    assert!(volume_layout(&tsdf, &other).is_err(), "block side differs from the field");
}

#[test]
fn page_ids_pack_negative_keys_and_round_trip() {
    for key in [[0, 0, 0], [-1, 0, 5], [MAX_BLOCK_KEY, -MAX_BLOCK_KEY - 1, 7], [-3, -4, -5]] {
        let id = volume_page_id(key).unwrap();
        assert_eq!(id >> 63, 0);
        assert_eq!(volume_block_key(id), key);
    }
    assert!(volume_page_id([MAX_BLOCK_KEY + 1, 0, 0]).is_none());
    assert_ne!(volume_page_id([-1, 0, 0]), volume_page_id([0, -1, 0]));
}

#[test]
fn a_plane_reads_exactly_across_block_boundaries_negative_coordinates_and_non_unit_voxels() {
    let field = esdf(0.05);
    let blocks: Vec<_> = keys(-2..=1, -2..=1, -1..=2).into_iter().map(|k| block(&field, k, plane, |_| true)).collect();
    let bytes = encoded(&field, &blocks);
    let volume = volume(&field, &bytes);
    let points = [[0.0, 0.0, 0.37], [-0.4, -0.4, 0.0], [-0.4001, 0.39, 0.4], [0.2, -0.61, -0.2], [-0.72, 0.13, 0.81]];
    for p in points {
        let sample = volume.sample(p, Channel::Value, Interpolation::Trilinear, true);
        assert_eq!(sample.support, Support::Valid, "{p:?}");
        assert!((sample.value.unwrap() - plane(p)).abs() < 1e-6, "{p:?}: {:?} vs {}", sample.value, plane(p));
        let g = sample.gradient.unwrap();
        assert!(g[0].abs() < 1e-4 && g[1].abs() < 1e-4 && (g[2] - 1.0).abs() < 1e-4, "{p:?}: {g:?}");
    }
    let nearest = volume.at([-0.401, 0.0, 0.0], Channel::Value, Interpolation::Nearest).unwrap();
    assert!((nearest - plane([-0.425, 0.025, 0.025])).abs() < 1e-6, "nearest reads the containing voxel centre");
}

#[test]
fn a_sphere_reads_within_the_interpolation_bound() {
    let field = esdf(0.05);
    let h = field.voxel_m;
    let blocks: Vec<_> = keys(-3..=2, -3..=2, -2..=3).into_iter().map(|k| block(&field, k, sphere, |_| true)).collect();
    let bytes = encoded(&field, &blocks);
    let volume = volume(&field, &bytes);
    let mut checked = 0;
    for step in 0..200 {
        let t = f64::from(step) * 0.731;
        let p = [CENTRE[0] + 0.8 * t.sin() * (t * 0.3).cos(), CENTRE[1] + 0.8 * (t * 0.7).sin(), CENTRE[2] + 0.8 * t.cos() * (t * 0.4).cos()];
        let d = distance(p);
        if d < 0.3 {
            continue;
        }
        let bound = 3.0 * h * h / (8.0 * (d - 2.0 * h)) + 1e-6;
        let sample = volume.sample(p, Channel::Value, Interpolation::Trilinear, true);
        assert!((sample.value.unwrap() - sphere(p)).abs() <= bound, "{p:?}");
        let exact = [0, 1, 2].map(|a| (p[a] - CENTRE[a]) / d);
        let g = sample.gradient.unwrap();
        let error = (0..3).map(|a| (g[a] - exact[a]).powi(2)).sum::<f64>().sqrt();
        assert!(error <= 3f64.sqrt() * (bound / h + h * h / (2.0 * (d - 2.0 * h).powi(2))), "{p:?}: {error}");
        checked += 1;
    }
    assert!(checked > 100);
}

#[test]
fn unknown_support_never_reads_as_a_distance() {
    let field = esdf(0.1);
    let band = |p: [f64; 3]| !(0.3..0.5).contains(&p[2]);
    let blocks: Vec<_> = keys(0..=0, 0..=0, 0..=1).into_iter().map(|k| block(&field, k, plane, band)).collect();
    let bytes = encoded(&field, &blocks);
    let volume = volume(&field, &bytes);
    let at_zero = volume.sample([0.4, 0.4, PLANE_Z], Channel::Value, Interpolation::Trilinear, true);
    assert_eq!((at_zero.value, at_zero.support, at_zero.gradient), (None, Support::Unknown, None), "the plane lies in the unknown band");
    let edge = volume.sample([0.4, 0.4, 0.27], Channel::Value, Interpolation::Trilinear, false);
    assert_eq!(edge.support, Support::Unknown, "one trilinear corner is unknown");
    assert!(volume.at([0.4, 0.4, 0.27], Channel::Value, Interpolation::Nearest).is_ok(), "the containing voxel itself is known");
    let outside = volume.sample([0.4, 0.4, 1.75], Channel::Value, Interpolation::Trilinear, false);
    assert_eq!(outside.support, Support::Unallocated, "no block was ever sent there");
    let beyond = volume.sample([-0.05, 0.4, 0.1], Channel::Value, Interpolation::Trilinear, false);
    assert_eq!(beyond.support, Support::Unallocated, "a corner lies in the absent block at x < 0");
    let weights = volume.sample([0.4, 0.4, 0.1], Channel::Weight, Interpolation::Trilinear, false);
    assert_eq!(weights.value, None, "an ESDF carries no weights");
}

#[test]
fn field_and_view_descriptors_validate_their_own_rules() {
    let bad = [
        VolumeField { voxel_m: 0.0, ..esdf(0.1) },
        VolumeField { block_side: 4, ..esdf(0.1) },
        VolumeField { kind: FieldKind::Tsdf { truncation_m: 0.0 }, ..esdf(0.1) },
        VolumeField { kind: FieldKind::Scalar { units: "K".into(), range: [3.0, 3.0] }, ..esdf(0.1) },
    ];
    for field in bad {
        assert!(field.validate().is_err(), "{field:?}");
    }
    let slice = VolumeSlice::across(2, [0.0, 0.0, 0.4], [4.0, 4.0], 0.05);
    assert_eq!(slice.normal(), [0.0, 0.0, 1.0]);
    let view = VolumeView {
        slices: vec![slice.clone()],
        interpolation: Interpolation::Trilinear,
        band: None,
        contours: vec![0.0],
        contour_rgba: [255; 4],
        unknown_rgba: [80, 0, 80, 255],
        frame_rgba: None,
    };
    view.validate(&esdf(0.1)).unwrap();
    let skew = VolumeSlice { v: [0.6, 0.8, 0.0], ..slice.clone() };
    let tiny = VolumeSlice { cell_m: 0.01, ..slice.clone() };
    let huge = VolumeSlice { size_m: [100.0, 100.0], ..slice.clone() };
    for (case, slice) in [("skew axes", skew), ("cells under a quarter voxel", tiny), ("too many cells", huge)] {
        assert!(VolumeView { slices: vec![slice], ..view.clone() }.validate(&esdf(0.1)).is_err(), "{case}");
    }
    assert!(VolumeView { band: Some([1.0, 0.0]), ..view.clone() }.validate(&esdf(0.1)).is_err());
    let descriptor = |presentation: Presentation| DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new("vfx-test/volumes", "field").unwrap(),
        shape: DataShape::Volume { field: esdf(0.1), frame: DataFrame::default() },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation,
    };
    let scalar = |name: &str| ColorMode::Scalar { name: name.into(), colormap: gearbox_api::colormap::TURBO, range: [-1.0, 1.0], missing: [0; 4] };
    descriptor(Presentation { color: scalar("value"), volume: Some(view.clone()), ..Presentation::default() }).validate().unwrap();
    assert!(descriptor(Presentation { color: scalar("weight"), ..Presentation::default() }).validate().is_err(), "an ESDF has no weight to colour by");
    let cloud = DataDescriptor {
        shape: DataShape::Cloud { schema: volume_schema(&esdf(0.1)) },
        presentation: Presentation { volume: Some(view), ..Presentation::default() },
        ..descriptor(Presentation::default())
    };
    assert!(cloud.validate().is_err(), "volume views need a volume dataset");
}

fn req(op: u32) -> DataRequest {
    DataRequest { host_epoch: EPOCH, ..DataRequest::new(op) }
}

fn ok(store: &mut StreamingStore, request: &DataRequest) -> DataReply {
    let reply = store.handle(request, 0);
    assert_eq!(reply.code, data_code::OK, "{}", String::from_utf8_lossy(&reply.message));
    reply
}

fn edit(begin: &DataRequest, op: u32) -> DataRequest {
    DataRequest {
        lease: begin.lease,
        transaction: begin.transaction,
        incarnation: begin.incarnation,
        base_generation: begin.base_generation,
        key: begin.key.clone(),
        ..req(op)
    }
}

fn key() -> DataKey {
    DataKey::new("vfx-test/volumes", "field").unwrap()
}

fn page(field: &VolumeField, block: &VolumeBlock) -> (PageDescriptor, Vec<u8>) {
    let bytes = encode_volume_block(field, block).unwrap();
    (
        PageDescriptor {
            address: PageAddress { scan_key: 0, page_id: volume_page_id(block.key).unwrap() },
            encoding: PageEncoding::VolumeBlockV1,
            items: field.samples() as u32,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [0.0; 3],
        },
        bytes,
    )
}

#[allow(clippy::too_many_arguments)]
fn transaction(
    store: &mut StreamingStore,
    lease: u64,
    id: u64,
    field: &VolumeField,
    mode: CommitMode,
    base: Option<DatasetRevision>,
    pages: &[(PageDescriptor, Vec<u8>)],
    removed: &[[i32; 3]],
) -> DataReply {
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: key(),
        shape: DataShape::Volume { field: field.clone(), frame: DataFrame::default() },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation::default(),
    };
    let begin = DataRequest {
        lease,
        transaction: id,
        incarnation: base.map_or(0, |b| b.incarnation),
        base_generation: base.map_or(0, |b| b.generation),
        total_bytes: pages.iter().map(|(d, _)| d.byte_len).sum(),
        ..req(data_op::BEGIN)
    }
    .with_key(&descriptor.key)
    .unwrap()
    .with_metadata(&BeginMetadata {
        descriptor,
        mode,
        scans: 0,
        pages: pages.len() as u32,
        removed_scans: 0,
        removed_pages: removed.len() as u32,
    })
    .unwrap();
    ok(store, &begin);
    for (page, bytes) in pages {
        let declare = DataRequest { page_id: page.address.page_id, ..edit(&begin, data_op::DECLARE_PAGE) }.with_metadata(page).unwrap();
        ok(store, &declare);
        for (at, chunk) in bytes.chunks(DATA_CHUNK_BYTES).enumerate() {
            let chunk = DataRequest { page_id: page.address.page_id, offset: (at * DATA_CHUNK_BYTES) as u64, payload: chunk.to_vec(), ..edit(&begin, data_op::CHUNK) };
            let reply = store.handle(&chunk, 0);
            if reply.code != data_code::OK {
                return reply;
            }
        }
    }
    for key in removed {
        ok(store, &DataRequest { page_id: volume_page_id(*key).unwrap(), ..edit(&begin, data_op::REMOVE_PAGE) });
    }
    store.handle(&edit(&begin, data_op::COMMIT), 0)
}

fn query(store: &mut StreamingStore, points: Vec<[f64; 3]>, generation: Option<u64>) -> Result<FieldAnswer, DataReply> {
    let request = req(data_op::FIELD_QUERY)
        .with_key(&key())
        .unwrap()
        .with_metadata(&FieldQuery { points, channel: Channel::Value, interpolation: Interpolation::Trilinear, gradient: true, generation })
        .unwrap();
    let reply = store.handle(&request, 0);
    if reply.code != data_code::OK {
        return Err(reply);
    }
    Ok(serde_json::from_slice(&reply.metadata).unwrap())
}

#[test]
fn the_store_answers_field_queries_and_a_block_delta_replaces_or_deletes_only_its_blocks() {
    let field = esdf(0.1);
    let mut store = StreamingStore::new(EPOCH, DataLimits::reference()).unwrap();
    let lease = ok(&mut store, &req(data_op::OPEN_WRITER)).lease;
    let column: Vec<_> = keys(-1..=0, 0..=0, 0..=0).into_iter().map(|k| page(&field, &block(&field, k, plane, |_| true))).collect();
    let first = transaction(&mut store, lease, 1, &field, CommitMode::Replace, None, &column, &[]);
    assert_eq!(first.code, data_code::OK, "{}", String::from_utf8_lossy(&first.message));
    let base = DatasetRevision { incarnation: first.incarnation, generation: first.stored_generation };
    let answer = query(&mut store, vec![[-0.05, 0.4, 0.37], [0.4, 0.4, 0.2]], None).unwrap();
    assert_eq!((answer.revision, answer.spacing_m), (base, 0.1));
    assert!((answer.samples[0].value.unwrap()).abs() < 1e-6, "the plane, read across the x = 0 block boundary");
    assert!((answer.samples[1].value.unwrap() + 0.17).abs() < 1e-6);
    assert!(answer.samples.iter().all(|s| s.gradient.is_some_and(|g| (g[2] - 1.0).abs() < 1e-4)));
    let shifted = page(&field, &block(&field, [0, 0, 0], |p| plane(p) + 1.0, |_| true));
    let delta = transaction(&mut store, lease, 2, &field, CommitMode::Delta, Some(base), &[shifted], &[[-1, 0, 0]]);
    assert_eq!(delta.code, data_code::OK, "{}", String::from_utf8_lossy(&delta.message));
    let answer = query(&mut store, vec![[0.4, 0.4, 0.2], [-0.4, 0.4, 0.2]], None).unwrap();
    assert!((answer.samples[0].value.unwrap() - 0.83).abs() < 1e-6, "the replaced block");
    assert_eq!(answer.samples[1].support, Support::Unallocated, "the deleted block is gone, not free space");
    let stale = query(&mut store, vec![[0.4, 0.4, 0.2]], Some(base.generation)).unwrap_err();
    assert_eq!(stale.code, data_code::SUPERSEDED);
    let registry = store.registry();
    let registry = registry.lock().unwrap();
    let entry = registry.get(&(key().namespace, key().id)).unwrap();
    assert_eq!((entry.kind, entry.payload.resident().unwrap().pages().len()), (DatasetKind::Volume, 1));
    drop(registry);
    let (mut wrong, bytes) = page(&field, &block(&field, [3, 0, 0], plane, |_| true));
    wrong.address.page_id = volume_page_id([4, 0, 0]).unwrap();
    let revision = DatasetRevision { incarnation: delta.incarnation, generation: delta.stored_generation };
    let refused = transaction(&mut store, lease, 3, &field, CommitMode::Delta, Some(revision), &[(wrong, bytes)], &[]);
    assert_ne!(refused.code, data_code::OK, "a block filed under another key's page id");
}

#[test]
fn the_rust_and_python_encoders_write_the_same_block() {
    let golden = include_str!("fixtures/draw_data/volume_block_tsdf.hex").trim();
    let field = VolumeField { kind: FieldKind::Tsdf { truncation_m: 0.3 }, voxel_m: 0.25, block_side: 8, colors: false };
    let block = block(&field, [-1, 0, 2], |p| p[2] - 4.9 - 0.1 * f64::from(p[0] > -1.0), |p| p[1] < 1.0);
    let hex: String = encode_volume_block(&field, &block).unwrap().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, golden);
}
