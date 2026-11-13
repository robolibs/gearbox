use gearbox_api::draw_data::*;
use gearbox_api::wire::draw_data::mesh::{MAX_MESH_VERTICES, MeshBlock, decode_mesh_block, encode_mesh_block, mesh_layout};
use gearbox_api::wire::draw_data::*;

const EPOCH: u64 = 23;

fn square() -> MeshBlock {
    MeshBlock {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        normals: Some(vec![[0.0, 0.0, 1.0]; 4]),
        colors: Some(vec![[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [9, 9, 9, 255]]),
        confidence: Some(vec![0.0, 0.25, 0.5, 1.0]),
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

fn field(name: &str, scalar: ScalarType, components: u8, semantic: AttributeSemantic, units: &str) -> Attribute {
    Attribute {
        name: name.into(),
        scalar,
        components,
        semantic,
        units: units.into(),
        normalization: Normalization::None,
        nullable: semantic == AttributeSemantic::Normal,
    }
}

fn surfel_schema() -> CloudSchema {
    CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            field("p", ScalarType::F32, 3, AttributeSemantic::Position, "m"),
            field("id", ScalarType::U64, 1, AttributeSemantic::SourceId, ""),
            field("n", ScalarType::F32, 3, AttributeSemantic::Normal, ""),
            field("r", ScalarType::F32, 1, AttributeSemantic::Radius, "m"),
        ],
    }
}

fn descriptor(shape: DataShape) -> DataDescriptor {
    DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new("vfx-test/surfaces", "lab").unwrap(),
        shape,
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation::default(),
    }
}

#[test]
fn mesh_blocks_roundtrip_and_report_where_their_arrays_lie() {
    let block = square();
    let bytes = encode_mesh_block(&block).unwrap();
    assert_eq!(decode_mesh_block(&bytes).unwrap(), block);
    let layout = mesh_layout(&bytes).unwrap();
    assert_eq!((layout.vertices, layout.triangles()), (4, 2));
    let bare = MeshBlock { normals: None, colors: None, confidence: None, ..square() };
    let bytes = encode_mesh_block(&bare).unwrap();
    let layout = mesh_layout(&bytes).unwrap();
    assert!(layout.normals.is_none() && layout.colors.is_none());
    assert_eq!(bytes.len(), 16 + 4 * 12 + 6 * 4);
}

#[test]
fn malformed_mesh_blocks_are_refused() {
    let cases: Vec<(&str, MeshBlock)> = vec![
        ("index outside", MeshBlock { indices: vec![0, 1, 4], ..square() }),
        ("unfinished triangle", MeshBlock { indices: vec![0, 1], ..square() }),
        ("no triangles", MeshBlock { indices: vec![], ..square() }),
        ("non-unit normal", MeshBlock { normals: Some(vec![[0.0, 0.0, 2.0]; 4]), ..square() }),
        ("NaN normal", MeshBlock { normals: Some(vec![[f32::NAN, 0.0, 1.0]; 4]), ..square() }),
        ("confidence above one", MeshBlock { confidence: Some(vec![1.5; 4]), ..square() }),
        ("short colours", MeshBlock { colors: Some(vec![[0; 4]; 3]), ..square() }),
        ("infinite vertex", MeshBlock { positions: vec![[f32::INFINITY, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]], ..square() }),
        ("too few vertices", MeshBlock { positions: vec![[0.0; 3]; 2], normals: None, colors: None, confidence: None, indices: vec![0, 1, 1] }),
    ];
    for (case, block) in cases {
        assert!(encode_mesh_block(&block).is_err(), "{case} was accepted");
    }
    let mut bytes = encode_mesh_block(&square()).unwrap();
    bytes[4..8].copy_from_slice(&(MAX_MESH_VERTICES + 1).to_le_bytes());
    assert!(mesh_layout(&bytes).is_err(), "vertex count above the limit");
    let mut short = encode_mesh_block(&square()).unwrap();
    short.pop();
    assert!(mesh_layout(&short).is_err(), "truncated block");
}

#[test]
fn surfel_and_mesh_descriptors_validate_their_own_rules() {
    let frame = DataFrame::default();
    descriptor(DataShape::Surfels { schema: surfel_schema(), frame: frame.clone() }).validate().unwrap();
    let mut no_radius = surfel_schema();
    no_radius.attributes.pop();
    assert!(descriptor(DataShape::Surfels { schema: no_radius, frame: frame.clone() }).validate().is_err());
    let mesh = descriptor(DataShape::MeshBlocks { frame: frame.clone() });
    mesh.validate().unwrap();
    let colored = DataDescriptor {
        presentation: Presentation {
            color: ColorMode::Attribute { name: "color".into(), space: ColorSpace::Srgb },
            normals: Some(NormalArrows { every: 4, length_m: 0.2, rgba: [255, 255, 255, 255] }),
            ..Presentation::default()
        },
        ..mesh.clone()
    };
    colored.validate().unwrap();
    let bad_arrows = DataDescriptor {
        presentation: Presentation { normals: Some(NormalArrows { every: 0, length_m: 0.2, rgba: [0; 4] }), ..Presentation::default() },
        ..mesh.clone()
    };
    assert!(bad_arrows.validate().is_err());
    let accumulating = DataDescriptor {
        stream: StreamMode::Accumulate { max_scans: 4, max_age_s: 10.0, max_bytes: 1 << 20, age_clock: AgeClock::Simulation },
        ..mesh
    };
    assert!(accumulating.validate().is_err(), "only clouds accumulate scans");
    assert_eq!(DataShape::MeshBlocks { frame: frame.clone() }.kind(), DatasetKind::MeshBlocks);
    assert_eq!(DataShape::Surfels { schema: surfel_schema(), frame }.encoding(), PageEncoding::CloudColumnsV1);
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

fn block_page(id: u64, block: &MeshBlock) -> (PageDescriptor, Vec<u8>) {
    let bytes = encode_mesh_block(block).unwrap();
    (
        PageDescriptor {
            address: PageAddress { scan_key: 0, page_id: id },
            encoding: PageEncoding::MeshBlockV1,
            items: block.triangles() as u32,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [0.0; 3],
        },
        bytes,
    )
}

fn transaction(store: &mut StreamingStore, lease: u64, id: u64, mode: CommitMode, base: Option<DatasetRevision>, pages: &[(PageDescriptor, Vec<u8>)]) -> DataReply {
    let descriptor = descriptor(DataShape::MeshBlocks { frame: DataFrame::default() });
    let bytes: u64 = pages.iter().map(|(d, _)| d.byte_len).sum();
    let begin = DataRequest {
        lease,
        transaction: id,
        incarnation: base.map_or(0, |b| b.incarnation),
        base_generation: base.map_or(0, |b| b.generation),
        total_bytes: bytes,
        ..req(data_op::BEGIN)
    }
    .with_key(&descriptor.key)
    .unwrap()
    .with_metadata(&BeginMetadata { descriptor, mode, scans: 0, pages: pages.len() as u32, removed_scans: 0, removed_pages: 0 })
    .unwrap();
    ok(store, &begin);
    for (page, bytes) in pages {
        let declare = DataRequest { scan_key: 0, page_id: page.address.page_id, ..edit(&begin, data_op::DECLARE_PAGE) }.with_metadata(page).unwrap();
        ok(store, &declare);
        let chunk = DataRequest { scan_key: 0, page_id: page.address.page_id, offset: 0, payload: bytes.clone(), ..edit(&begin, data_op::CHUNK) };
        let reply = store.handle(&chunk, 0);
        if reply.code != data_code::OK {
            return reply;
        }
    }
    store.handle(&edit(&begin, data_op::COMMIT), 0)
}

#[test]
fn mesh_blocks_commit_and_a_delta_replaces_only_its_block() {
    let mut store = StreamingStore::new(EPOCH, DataLimits::reference()).unwrap();
    let lease = ok(&mut store, &req(data_op::OPEN_WRITER)).lease;
    let first = transaction(&mut store, lease, 1, CommitMode::Replace, None, &[block_page(0, &square()), block_page(1, &square())]);
    assert_eq!(first.code, data_code::OK, "{}", String::from_utf8_lossy(&first.message));
    let base = DatasetRevision { incarnation: first.incarnation, generation: first.stored_generation };
    let moved = MeshBlock { positions: square().positions.iter().map(|p| [p[0] + 3.0, p[1], p[2]]).collect(), ..square() };
    let delta = transaction(&mut store, lease, 2, CommitMode::Delta, Some(base), &[block_page(1, &moved)]);
    assert_eq!(delta.code, data_code::OK, "{}", String::from_utf8_lossy(&delta.message));
    let key = DataKey::new("vfx-test/surfaces", "lab").unwrap();
    let registry = store.registry();
    let registry = registry.lock().unwrap();
    let entry = registry.get(&(key.namespace.clone(), key.id.clone())).unwrap();
    assert_eq!(entry.kind, DatasetKind::MeshBlocks);
    let generation = entry.payload.resident().unwrap();
    let pages = generation.pages();
    assert_eq!(pages.len(), 2);
    assert!(pages[0].mesh_layout().is_some());
    let first_vertex = f32::from_le_bytes(pages[1].bytes()[16..20].try_into().unwrap());
    assert_eq!(first_vertex, 3.0, "the delta replaced block 1");
    drop(registry);
    let mut bytes = encode_mesh_block(&square()).unwrap();
    let wrong = (PageDescriptor { items: 1, ..block_page(2, &square()).0 }, std::mem::take(&mut bytes));
    let refused = transaction(&mut store, lease, 3, CommitMode::Delta, Some(DatasetRevision { incarnation: delta.incarnation, generation: delta.stored_generation }), &[wrong]);
    assert_ne!(refused.code, data_code::OK, "a block whose descriptor miscounts its triangles");
}

#[test]
fn the_rust_and_python_encoders_write_the_same_block() {
    let golden = include_str!("fixtures/draw_data/mesh_block_square.hex").trim();
    let bytes = encode_mesh_block(&square()).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, golden);
}
