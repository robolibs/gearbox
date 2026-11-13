use gearbox_api::wire::{DrawEntity, Env, LidarScan, Props, draw_data::*, json, pack};
use serde_json::{Value, json};

#[allow(dead_code)]
#[path = "draw_data_contracts.rs"]
mod contracts;

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}

fn packet(name: &str, canonical: &str, env: Env) -> Value {
    json!({"name": name, "canonical": canonical, "type_hash": env.type_hash(),
        "wire_hex": hex(env.wire()), "json": json::env_to_json(&env).unwrap()})
}

pub fn fixture() -> Value {
    let mut packets = Vec::new();
    let names = [
        "capabilities",
        "open_writer",
        "close_writer",
        "begin",
        "declare_scan",
        "declare_page",
        "chunk",
        "remove_page",
        "remove_scan",
        "commit",
        "cancel",
        "patch_presentation",
        "delete",
        "clear",
        "list",
        "status",
        "attach_provider",
        "field_query",
    ];
    let descriptor = contracts::cloud();
    for (op, name) in names.iter().enumerate() {
        let op = op as u32;
        let mut r = DataRequest::new(op);
        if op != data_op::CAPABILITIES {
            r.host_epoch = u64::MAX;
        }
        if (2..=13).contains(&op) || op == data_op::ATTACH_PROVIDER {
            r.lease = u64::MAX - 1;
        }
        if (3..=10).contains(&op) {
            r.transaction = u64::MAX - 2;
        }
        if (3..=12).contains(&op) || op == 15 || op == data_op::ATTACH_PROVIDER {
            r = r.with_key(&descriptor.key).unwrap();
            r.incarnation = u64::MAX - 3;
            r.base_generation = u64::MAX - 4;
        }
        match op {
            data_op::BEGIN => {
                r.total_bytes = 73;
                r = r
                    .with_metadata(&BeginMetadata {
                        descriptor: descriptor.clone(),
                        mode: CommitMode::Delta,
                        scans: 1,
                        pages: 1,
                        removed_scans: 0,
                        removed_pages: 0,
                    })
                    .unwrap();
            }
            data_op::DECLARE_SCAN => {
                r.scan_key = 1;
                r = r.with_metadata(&contracts::scan(1)).unwrap();
            }
            data_op::DECLARE_PAGE => {
                r.scan_key = 1;
                r.page_id = u64::MAX;
                r = r
                    .with_metadata(&PageDescriptor {
                        address: PageAddress {
                            scan_key: 1,
                            page_id: u64::MAX,
                        },
                        encoding: PageEncoding::CloudColumnsV1,
                        items: 2,
                        byte_len: 73,
                        crc32: 0x12345678,
                        origin_m: [1e12, -1e12, 0.125],
                    })
                    .unwrap();
            }
            data_op::CHUNK => {
                r.scan_key = u64::MAX;
                r.page_id = u64::MAX;
                r.offset = 65_536;
                r.payload = vec![0, 127, 128, 255, 1];
            }
            data_op::REMOVE_PAGE => {
                r.scan_key = u64::MAX;
                r.page_id = u64::MAX;
            }
            data_op::REMOVE_SCAN => {
                r.scan_key = u64::MAX;
            }
            data_op::PATCH_PRESENTATION => r = r.with_metadata(&Presentation::default()).unwrap(),
            data_op::CLEAR => {
                r = r
                    .with_metadata(&NamespaceFilter {
                        namespace: "mapping".into(),
                    })
                    .unwrap()
            }
            data_op::LIST => {
                r = r
                    .with_metadata(&DataListQuery {
                        namespace: "mapping".into(),
                        limit: 128,
                        cursor: Some("opaque:é:1".into()),
                    })
                    .unwrap()
            }
            data_op::STATUS => {
                r.key = br#"{ "id":"cloud", "namespace":"mapping" }"#.to_vec();
                r.metadata =
                    br#"{ "generation":18446744073709551615, "view":{"id":9,"epoch":7} }"#.to_vec();
            }
            data_op::ATTACH_PROVIDER => {
                r = r
                    .with_metadata(&ProviderAttachment {
                        provider: "fixture/survey-heightfield".into(),
                    })
                    .unwrap()
            }
            data_op::FIELD_QUERY => {
                r = r
                    .with_key(&descriptor.key)
                    .unwrap()
                    .with_metadata(&FieldQuery {
                        points: vec![[-0.25, 1e6, 0.125]],
                        channel: Channel::Weight,
                        interpolation: Interpolation::Nearest,
                        gradient: true,
                        generation: Some(9),
                    })
                    .unwrap()
            }
            _ => {}
        }
        r.operation().unwrap();
        packets.push(packet(
            name,
            DataRequest::CANONICAL_NAME,
            r.pack_checked().unwrap(),
        ));
    }
    let mut octree = DataRequest::new(data_op::BEGIN)
        .with_key(&contracts::octree().key)
        .unwrap();
    octree.host_epoch = 1;
    octree.lease = 2;
    octree.transaction = 3;
    octree.total_bytes = 16;
    octree = octree
        .with_metadata(&BeginMetadata {
            descriptor: contracts::octree(),
            mode: CommitMode::Replace,
            scans: 0,
            pages: 1,
            removed_scans: 0,
            removed_pages: 0,
        })
        .unwrap();
    octree.operation().unwrap();
    packets.push(packet(
        "begin_octree",
        DataRequest::CANONICAL_NAME,
        octree.pack_checked().unwrap(),
    ));

    for stage in 0..=3 {
        let reply = DataReply {
            stage,
            lease: u64::MAX - 1,
            transaction: u64::MAX - 2,
            incarnation: 2,
            stored_generation: 9,
            visible_generation: if stage == 3 { 8 } else { 0 },
            view_id: if stage == 3 { 4 } else { 0 },
            view_epoch: if stage == 3 { 5 } else { 0 },
            metadata: br#"{ "exact":18446744073709551615, "empty":null }"#.to_vec(),
            message: "énergie".as_bytes().to_vec(),
            ..DataReply::new(u64::MAX)
        };
        packets.push(packet(
            &format!("stage_{stage}"),
            DataReply::CANONICAL_NAME,
            reply.pack_checked().unwrap(),
        ));
    }
    for code in 1..=data_code::STALE_CURSOR {
        let reply = DataReply::error(u64::MAX, code, "bounded failure");
        packets.push(packet(
            &format!("error_{code}"),
            DataReply::CANONICAL_NAME,
            reply.pack_checked().unwrap(),
        ));
    }

    let mut legacy = DrawEntity::lines("golden", "route", &[[1.25, -2.5, 3.0], [4.0, 5.0, 6.0]]);
    legacy.color = 0x4488ccff;
    legacy.width = 0.125;
    packets.push(packet(
        "legacy_lines",
        DrawEntity::CANONICAL_NAME,
        pack(&legacy),
    ));
    let mut lidar = LidarScan {
        sim_time_s: 2.125,
        theta_min: -0.5,
        theta_max: 0.5,
        phi_min: 0.0,
        phi_max: 0.0,
        max_range_m: 20.0,
        link_index: 3,
        rows: 1,
        cols: 2,
        pulse_offset: 0,
        pulse_count: 2,
        hits: 2,
        stamp_ms: 125,
        sample: u32::MAX,
        ranges: vec![1.0, 2.0],
        points: vec![1.0, 0.0, 0.0, 2.0, 0.0, 0.0],
        props: Props::from_pairs(&[("name", "lidar")]).into_bytes(),
    };
    packets.push(packet(
        "legacy_lidar",
        LidarScan::CANONICAL_NAME,
        pack(&lidar),
    ));
    let mut transform = contracts::scan(1).frame.capture_transform.unwrap();
    transform.stamp.simulation_time_s = lidar.sim_time_s;
    transform.stamp.sample = u64::MAX;
    transform.stamp.scene_epoch = u64::MAX;
    transform.stamp.source_time_ns = Some(u64::MAX);
    lidar
        .set_native_capture(&NativeLidarCapture {
            revision: 1,
            machine: "robot".into(),
            link: "lidar".into(),
            convention: CaptureConvention::RightHandedZUpMetres,
            transform,
        })
        .unwrap();
    packets.push(packet(
        "lidar_capture",
        LidarScan::CANONICAL_NAME,
        pack(&lidar),
    ));

    json!({"revision": 1, "request_header_bytes": 112, "reply_header_bytes": 96,
        "packets": packets, "pages": pages()})
}

fn pages() -> Value {
    let schema = CloudSchema {
        revision: 1,
        attributes: vec![
            Attribute {
                name: "xyz".into(),
                scalar: ScalarType::F32,
                components: 3,
                semantic: AttributeSemantic::Position,
                units: "m".into(),
                normalization: Normalization::None,
                nullable: true,
            },
            Attribute {
                name: "id".into(),
                scalar: ScalarType::U64,
                components: 1,
                semantic: AttributeSemantic::SourceId,
                units: "".into(),
                normalization: Normalization::None,
                nullable: false,
            },
        ],
    };
    let cloud = CloudPage {
        count: 2,
        columns: vec![
            ColumnData {
                values: [1.0f32, -2.0, 3.0, 0.0, 0.0, 0.0]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect(),
                validity: vec![1],
            },
            ColumnData {
                values: [0u64, u64::MAX]
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect(),
                validity: vec![],
            },
        ],
    };
    let config = OctreeConfig {
        leaf_resolution_m: 0.125,
        max_depth: 3,
        free_max: 0.3,
        occupied_min: 0.7,
    };
    let leaves = vec![
        OccupancyLeaf {
            key: VoxelKey {
                tile: [-1, 0, 2],
                depth: 1,
                morton: 4,
            },
            value: OccupancyValue {
                probability: Some(1.0),
                confidence: Some(0.5),
                class_id: Some(u32::MAX),
            },
        },
        OccupancyLeaf {
            key: VoxelKey {
                tile: [-1, 0, 2],
                depth: 2,
                morton: 1,
            },
            value: OccupancyValue {
                probability: None,
                confidence: None,
                class_id: None,
            },
        },
    ];
    let cloud_bytes = cloud.encode(&schema).unwrap();
    let occupancy_bytes = encode_occupancy_page(&config, &leaves).unwrap();
    json!([
        {"name":"cloud", "schema":schema, "count":2,
            "positions":[[1.0,-2.0,3.0],[0.0,0.0,0.0]], "position_validity":[true,false],
            "source_ids":[0,u64::MAX], "wire_hex":hex(&cloud_bytes), "crc32":page_crc32(&cloud_bytes)},
        {"name":"occupancy", "config":config, "leaves":leaves,
            "wire_hex":hex(&occupancy_bytes), "crc32":page_crc32(&occupancy_bytes)},
        typed_page()
    ])
}

fn typed_page() -> Value {
    let mut schema = contracts::schema();
    for (name, scalar) in [("signed16", ScalarType::I16), ("signed32", ScalarType::I32)] {
        schema.attributes.push(Attribute {
            name: name.into(),
            scalar,
            components: 1,
            semantic: AttributeSemantic::Custom,
            units: "".into(),
            normalization: Normalization::Snorm,
            nullable: false,
        });
    }
    schema.attributes.push(Attribute {
        name: "time_offset".into(),
        scalar: ScalarType::U64,
        components: 1,
        semantic: AttributeSemantic::AcquisitionOffset,
        units: "ns".into(),
        normalization: Normalization::None,
        nullable: false,
    });
    let positions = [1e12 + 0.125f64, -1e12, 3.5, 0.0, 0.0, 0.0];
    let ids = [0u64, u64::MAX];
    let rgba = [0u8, 127, 255, 128, 255, 0, 127, 255];
    let intensity = [0u16, u16::MAX];
    let classes = [0u32, u32::MAX];
    let radii = [0.125f32, 0.5];
    let signed16 = [i16::MIN, i16::MAX];
    let signed32 = [i32::MIN, i32::MAX];
    let offsets = [0u64, u64::MAX];
    let mut columns = vec![];
    let mut samples = vec![];
    macro_rules! column {
        ($values:ident, $ty:ident, $validity:expr) => {
            columns.push(ColumnData {
                values: $values.into_iter().flat_map($ty::to_le_bytes).collect(),
                validity: $validity,
            });
            samples.push(json!($values));
        };
    }
    column!(positions, f64, vec![1]);
    column!(ids, u64, vec![]);
    column!(rgba, u8, vec![]);
    column!(intensity, u16, vec![]);
    column!(classes, u32, vec![]);
    column!(radii, f32, vec![]);
    column!(signed16, i16, vec![]);
    column!(signed32, i32, vec![]);
    column!(offsets, u64, vec![]);
    let page = CloudPage { count: 2, columns };
    let bytes = page.encode(&schema).unwrap();
    let validity: Vec<_> = page.columns.iter().map(|c| c.validity.clone()).collect();
    json!({"name":"typed_cloud", "schema":schema, "count":2, "samples":samples,
        "validity":validity, "wire_hex":hex(&bytes), "crc32":page_crc32(&bytes)})
}
