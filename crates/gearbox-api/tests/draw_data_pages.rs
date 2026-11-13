use gearbox_api::wire::draw_data::*;

fn schema() -> CloudSchema {
    CloudSchema {
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
    }
}

fn cloud() -> CloudPage {
    CloudPage {
        count: 2,
        columns: vec![
            ColumnData {
                values: [1.0f32, 2.0, 3.0, 0.0, 0.0, 0.0]
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
    }
}

fn config() -> OctreeConfig {
    OctreeConfig {
        leaf_resolution_m: 0.125,
        max_depth: 3,
        free_max: 0.3,
        occupied_min: 0.7,
    }
}

fn leaves() -> Vec<OccupancyLeaf> {
    vec![
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
    ]
}

#[test]
fn cloud_page_bytes_are_exact_little_endian_columns_with_validity() {
    let bytes = cloud().encode(&schema()).unwrap();
    let expected: Vec<u8> = b"GBC1"
        .iter()
        .copied()
        .chain(
            [2u32, 2, 0, 24, 1, 16, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes),
        )
        .chain(
            [1.0f32, 2.0, 3.0, 0.0, 0.0, 0.0]
                .into_iter()
                .flat_map(f32::to_le_bytes),
        )
        .chain([1])
        .chain([0u64, u64::MAX].into_iter().flat_map(u64::to_le_bytes))
        .collect();
    assert_eq!(bytes, expected);
    assert_eq!(bytes.len(), 73);
    assert_eq!(CloudPage::decode(&schema(), &bytes).unwrap(), cloud());
    assert_eq!(
        CloudPage::decode(&schema(), &bytes)
            .unwrap()
            .encode(&schema())
            .unwrap(),
        bytes
    );
}

#[test]
fn cloud_decoder_rejects_all_truncations_and_extra_bytes() {
    let bytes = cloud().encode(&schema()).unwrap();
    for end in 0..bytes.len() {
        assert!(
            CloudPage::decode(&schema(), &bytes[..end]).is_err(),
            "prefix {end}"
        );
    }
    let mut extra = bytes;
    extra.push(0);
    assert!(CloudPage::decode(&schema(), &extra).is_err());
}

#[test]
fn cloud_declared_lengths_are_checked_before_copying_columns() {
    let bytes = cloud().encode(&schema()).unwrap();
    for (offset, value) in [
        (4, u32::MAX),
        (8, u32::MAX),
        (12, 1),
        (16, u32::MAX),
        (20, 2),
        (24, 0),
        (28, 1),
    ] {
        let mut bad = bytes.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(
            CloudPage::decode(&schema(), &bad).is_err(),
            "offset {offset}"
        );
    }
    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert!(CloudPage::decode(&schema(), &bad).is_err());
    let mut bad = bytes;
    bad[56] = 0xff;
    assert!(CloudPage::decode(&schema(), &bad).is_err());
}

#[test]
fn cloud_decoder_retains_semantic_validation() {
    let bytes = cloud().encode(&schema()).unwrap();
    for offset in [32, 44] {
        let mut bad = bytes.clone();
        bad[offset..offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(CloudPage::decode(&schema(), &bad).is_err());
    }
    let mut duplicate = bytes;
    duplicate[65..73].fill(0);
    assert!(CloudPage::decode(&schema(), &duplicate).is_err());
}

#[test]
fn occupancy_records_have_stable_layout_and_preserve_unknown() {
    let bytes = encode_occupancy_page(&config(), &leaves()).unwrap();
    assert_eq!(&bytes[..16], b"GBO1\x02\0\0\0\0\0\0\0\0\0\0\0");
    assert_eq!(bytes.len(), 16 + 48 * 2);
    assert_eq!(&bytes[16..24], &(-1i64).to_le_bytes());
    assert_eq!(&bytes[40..48], &4u64.to_le_bytes());
    assert_eq!(bytes[48], 1);
    assert_eq!(bytes[49], 7);
    assert_eq!(&bytes[52..56], &1f32.to_le_bytes());
    assert_eq!(bytes[97], 0);
    assert_eq!(&bytes[100..112], &[0; 12]);
    assert_eq!(decode_occupancy_page(&config(), &bytes).unwrap(), leaves());
}

#[test]
fn occupancy_decoder_rejects_truncation_reserved_bits_and_noncanonical_absence() {
    let bytes = encode_occupancy_page(&config(), &leaves()).unwrap();
    for end in 0..bytes.len() {
        assert!(decode_occupancy_page(&config(), &bytes[..end]).is_err());
    }
    for offset in [8, 49, 50, 97, 98, 100, 104, 108] {
        let mut bad = bytes.clone();
        bad[offset] = 0x80;
        assert!(
            decode_occupancy_page(&config(), &bad).is_err(),
            "offset {offset}"
        );
    }
    let mut bad = bytes.clone();
    bad.extend_from_slice(&[0; 48]);
    assert!(decode_occupancy_page(&config(), &bad).is_err());
    let mut bad = bytes;
    bad[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_occupancy_page(&config(), &bad).is_err());
}

#[test]
fn occupancy_decoder_validates_probability_and_topology() {
    let bytes = encode_occupancy_page(&config(), &leaves()).unwrap();
    let mut bad = bytes.clone();
    bad[52..56].copy_from_slice(&f32::INFINITY.to_le_bytes());
    assert!(decode_occupancy_page(&config(), &bad).is_err());
    let mut bad = bytes;
    let duplicate = bad[16..64].to_vec();
    bad[64..112].copy_from_slice(&duplicate);
    assert!(decode_occupancy_page(&config(), &bad).is_err());
}

#[test]
fn checksums_and_page_declarations_detect_corruption_and_wrong_counts() {
    assert_eq!(page_crc32(b"123456789"), 0xcbf43926);
    let bytes = cloud().encode(&schema()).unwrap();
    let descriptor = PageDescriptor {
        address: PageAddress {
            scan_key: 2,
            page_id: u64::MAX,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: 2,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [1_000_000.0, 0.0, 0.0],
    };
    assert_eq!(descriptor.decode_cloud(&schema(), &bytes).unwrap(), cloud());
    let mut bad = bytes.clone();
    bad[32] ^= 1;
    assert!(descriptor.decode_cloud(&schema(), &bad).is_err());
    assert!(descriptor.verify(&bytes[..bytes.len() - 1]).is_err());
    let mut wrong = descriptor.clone();
    wrong.items = 1;
    assert!(wrong.decode_cloud(&schema(), &bytes).is_err());
    wrong = descriptor;
    wrong.origin_m[0] = f64::NAN;
    assert!(wrong.validate().is_err());
}

#[test]
fn occupancy_page_addresses_are_not_scan_offsets() {
    let bytes = encode_occupancy_page(&config(), &leaves()).unwrap();
    let mut descriptor = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding: PageEncoding::OccupancyLeavesV1,
        items: 2,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    assert_eq!(
        descriptor.decode_occupancy(&config(), &bytes).unwrap(),
        leaves()
    );
    descriptor.address.scan_key = 1;
    assert!(descriptor.validate().is_err());
    descriptor.address.scan_key = 0;
    descriptor.origin_m[0] = 1.0;
    assert!(descriptor.validate().is_err());
}

#[test]
fn empty_pages_are_explicit_and_byte_bounded() {
    let page = CloudPage {
        count: 0,
        columns: vec![
            ColumnData {
                values: vec![],
                validity: vec![]
            };
            2
        ],
    };
    let encoded = page.encode(&schema()).unwrap();
    assert_eq!(encoded.len(), 32);
    assert_eq!(CloudPage::decode(&schema(), &encoded).unwrap(), page);
    let encoded = encode_occupancy_page(&config(), &[]).unwrap();
    assert_eq!(encoded.len(), 16);
    assert!(
        decode_occupancy_page(&config(), &encoded)
            .unwrap()
            .is_empty()
    );
    let oversized = vec![0; MAX_ENCODED_PAGE_BYTES + 1];
    assert!(CloudPage::decode(&schema(), &oversized).is_err());
    assert!(decode_occupancy_page(&config(), &oversized).is_err());
}
