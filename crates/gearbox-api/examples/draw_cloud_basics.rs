use std::time::{Duration, Instant};

use gearbox_api::wire::{draw::rgba, draw_data::*, draw_widgets::Place};
use gearbox_api::{
    Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawEntity as E, IdentitySource,
    Instance, colormap, draw_flag, draw_style, registry, shape_kind,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[path = "draw_cloud_basics/clocks.rs"]
mod clocks;
#[path = "draw_cloud_basics/precision.rs"]
mod precision;
const NS: &str = "vfx-gallery/cloud-basics";
const CENTERS: [[f64; 2]; 4] = [[-4.0, 4.2], [4.0, 4.2], [-4.0, -4.2], [4.0, -4.2]];

/// Namespace from `GEARBOX_CLOUD_NS`, so several variants can share one scene.
fn ns() -> &'static str {
    static VALUE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| std::env::var("GEARBOX_CLOUD_NS").unwrap_or_else(|_| NS.into()))
}

/// View centers shifted by `GEARBOX_CLOUD_OFFSET=x,y` metres.
fn centers() -> [[f64; 2]; 4] {
    let offset = std::env::var("GEARBOX_CLOUD_OFFSET")
        .ok()
        .and_then(|value| {
            let (x, y) = value.split_once(',')?;
            Some([x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?])
        })
        .unwrap_or([0.0; 2]);
    CENTERS.map(|[x, y]| [x + offset[0], y + offset[1]])
}
const COLORS: [[u8; 4]; 4] = [
    [69, 205, 224, 255],
    [240, 178, 79, 255],
    [154, 132, 240, 255],
    [92, 218, 158, 255],
];
const PALETTE_VALUES: [f32; 16] = [
    -1.0, 0.0, 1.0, 25.0, 50.0, 75.0, 98.0, 99.0, 100.0, 101.0, 127.0, 128.0, 200.0, 254.0, 255.0,
    256.0,
];

fn palette_cloud() -> CloudPage {
    let mut columns = vec![
        ColumnData {
            values: Vec::new(),
            validity: Vec::new()
        };
        5
    ];
    for (band, value) in PALETTE_VALUES.into_iter().enumerate() {
        for z in 0..32 {
            for x in 0..6 {
                let xyz = [
                    -2.2 + band as f64 * 4.4 / 16.0 + x as f64 * 0.035,
                    0.0,
                    1.0 + z as f64 * 0.065,
                ];
                columns[0]
                    .values
                    .extend(xyz.into_iter().flat_map(f64::to_le_bytes));
                let ordinal = (band * 32 * 6 + z * 6 + x) as u64;
                columns[1].values.extend((u64::MAX - ordinal).to_le_bytes());
                columns[2].values.extend([255; 4]);
                columns[3].values.extend(value.to_le_bytes());
                columns[4].values.extend((band as u32).to_le_bytes());
            }
        }
    }
    CloudPage {
        count: 16 * 32 * 6,
        columns,
    }
}

fn palette_presentation(index: usize) -> Presentation {
    Presentation {
        color: ColorMode::Scalar {
            name: "intensity".into(),
            colormap: [
                colormap::COSTMAP,
                colormap::MAP,
                colormap::RAW,
                colormap::OKABE,
            ][index],
            range: [0.0, 256.0],
            missing: [128, 128, 128, 255],
        },
        size: PointSize::Metres { radius: 0.021 },
        glow: 0.22,
        ..Default::default()
    }
}

fn schema() -> CloudSchema {
    let fields = [
        (
            "position",
            ScalarType::F64,
            3,
            AttributeSemantic::Position,
            "m",
        ),
        (
            "source_id",
            ScalarType::U64,
            1,
            AttributeSemantic::SourceId,
            "",
        ),
        ("rgba", ScalarType::U8, 4, AttributeSemantic::Color, ""),
        (
            "intensity",
            ScalarType::F32,
            1,
            AttributeSemantic::Intensity,
            "",
        ),
        ("class", ScalarType::U32, 1, AttributeSemantic::Category, ""),
    ];
    CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: fields
            .into_iter()
            .map(|(name, scalar, components, semantic, units)| Attribute {
                name: name.into(),
                scalar,
                components,
                semantic,
                units: units.into(),
                normalization: if semantic == AttributeSemantic::Color {
                    Normalization::Unorm
                } else {
                    Normalization::None
                },
                nullable: false,
            })
            .collect(),
    }
}

fn cloud() -> CloudPage {
    let mut points = Vec::new();
    for y in 0..48 {
        for x in 0..64 {
            let x = x as f64 / 63.0 * 4.4 - 2.2;
            let y = y as f64 / 47.0 * 3.6 - 1.8;
            points.push(([x, y, 1.0 + 0.12 * (x * 1.8).sin()], 0u32));
        }
    }
    for z in 0..36 {
        for x in 0..64 {
            let x = x as f64 / 63.0 * 4.4 - 2.2;
            points.push(([x, 1.45 + 0.15 * x.cos(), 1.0 + z as f64 / 35.0 * 2.2], 1));
        }
    }
    for ring in 1..36 {
        let phi = std::f64::consts::PI * ring as f64 / 36.0;
        for angle in 0..72 {
            let theta = std::f64::consts::TAU * angle as f64 / 72.0;
            points.push((
                [
                    0.85 * phi.sin() * theta.cos(),
                    -0.3 + 0.85 * phi.sin() * theta.sin(),
                    1.9 + 0.85 * phi.cos(),
                ],
                2,
            ));
        }
    }
    let mut columns = vec![
        ColumnData {
            values: Vec::new(),
            validity: Vec::new()
        };
        5
    ];
    for (index, (xyz, class)) in points.iter().enumerate() {
        for value in xyz {
            columns[0].values.extend_from_slice(&value.to_le_bytes());
        }
        columns[1]
            .values
            .extend_from_slice(&(u64::MAX - index as u64).to_le_bytes());
        let color = [
            [66, 139, 205, 255],
            [235, 161, 74, 255],
            [94, 220, 158, 255],
        ][*class as usize];
        columns[2].values.extend_from_slice(&color);
        columns[3]
            .values
            .extend_from_slice(&((xyz[0] + 2.2) as f32 / 4.4).to_le_bytes());
        columns[4].values.extend_from_slice(&class.to_le_bytes());
    }
    CloudPage {
        count: points.len() as u32,
        columns,
    }
}

fn presentation(index: usize) -> Presentation {
    let color = match index {
        0 => ColorMode::Constant { rgba: COLORS[0] },
        1 => ColorMode::Scalar {
            name: "intensity".into(),
            colormap: colormap::VIRIDIS,
            range: [0.0, 1.0],
            missing: [128, 128, 128, 255],
        },
        2 => ColorMode::Category {
            name: "class".into(),
            entries: [
                [89, 172, 231, 255],
                [226, 160, 79, 255],
                [173, 131, 237, 255],
            ]
            .into_iter()
            .enumerate()
            .map(|(value, rgba)| PaletteEntry {
                value: value as u32,
                rgba,
            })
            .collect(),
            missing: [128, 128, 128, 255],
        },
        _ => ColorMode::Attribute {
            name: "rgba".into(),
            space: ColorSpace::Srgb,
        },
    };
    Presentation {
        color,
        size: if index == 0 {
            PointSize::Pixels { radius: 1.4 }
        } else {
            PointSize::Metres { radius: 0.028 }
        },
        glow: 0.22,
        blur: if index == 3 { 0.45 } else { 0.0 },
        opacity: if index == 3 { 0.88 } else { 1.0 },
        ..Default::default()
    }
}

fn manifest() -> serde_json::Value {
    serde_json::json!({"overview":{"focus":[0.0,0.0,1.8],"distance":29,"pitch":45,"bearing":180,"bounds":{"min":[-7,-7,0.7],"max":[7,7,4.4]}},"sections":[
        {"id":"near","focus":[4,-4.2,1.9],"distance":7,"pitch":28,"bearing":150},
        {"id":"grazing","focus":[4,-4.2,1.9],"distance":8,"pitch":9,"bearing":185}
    ]})
}

fn clip_bounds(index: usize) -> Option<ClipBounds> {
    let [x, y] = centers()[index];
    match index {
        0 => None,
        1 => Some(ClipBounds {
            min_m: [x, y - 3.0, 0.0],
            max_m: [x + 3.0, y + 3.0, 5.0],
            ..Default::default()
        }),
        2 => Some(ClipBounds {
            min_m: [x + 20.0, y, 0.0],
            max_m: [x + 21.0, y + 1.0, 5.0],
            ..Default::default()
        }),
        _ => Some(ClipBounds {
            min_m: [x - 3.0, y - 3.0, 1.8],
            max_m: [x + 3.0, y + 3.0, 2.6],
            ..Default::default()
        }),
    }
}

fn annotations(
    client: &Client,
    clipped: bool,
    palettes: bool,
    clocks: bool,
    precision: bool,
) -> Result<()> {
    let place = Place::default().no_drape();
    for (index, center) in centers().iter().enumerate() {
        let [x, y] = center.map(|v| v as f32);
        let [r, g, b, a] = COLORS[index];
        let tint = rgba(r, g, b, a);
        let title = if precision {
            precision::TITLES[index]
        } else if clocks {
            [
                "01 / AGE VIRIDIS",
                "02 / AGE TURBO",
                "03 / AGE OKABE",
                "04 / AGE GREY",
            ][index]
        } else if palettes {
            [
                "01 / COSTMAP",
                "02 / OCCUPANCY MAP",
                "03 / RAW BYTE",
                "04 / OKABE INDEX",
            ][index]
        } else {
            [
                "01 / RAW POINTS",
                "02 / INTENSITY",
                "03 / EXACT CLASSES",
                "04 / MEASURED RGB",
            ][index]
        };
        let subtitle = if precision {
            precision::DETAILS[index]
        } else if clocks {
            "A / B ROWS MATCH / INDEPENDENT CLOCKS"
        } else if palettes {
            [
                "ZERO IS CLEAR / UNKNOWN IS GRAY",
                "FREE TO OCCUPIED / RAW VALUES",
                "CLAMPED BYTE / NO RANGE REMAP",
                "DISCRETE COLORS / NO INTERPOLATION",
            ][index]
        } else if clipped {
            [
                "UNCLIPPED REFERENCE",
                "HALF-SPACE / SOURCE X",
                "FULLY CLIPPED / ZERO SELECTED",
                "HEIGHT SLICE / FROST 0.45",
            ][index]
        } else {
            [
                "PIXEL RADIUS / OPAQUE",
                "METRIC RADIUS / VIRIDIS",
                "INTEGER PALETTE / OPAQUE",
                "SOURCE COLORS / FROST 0.45",
            ][index]
        };
        let objects = [
            place
                .put(E::text(
                    ns(),
                    &format!("title-{index}"),
                    &[[x, y + 2.3, 3.9]],
                    title,
                ))
                .with_style(draw_style::LABEL)
                .with_width(-17.0)
                .with_color(rgba(224, 235, 242, 255))
                .with_prop("accent", &tint.to_string())
                .with_prop("label_category", "TYPED CLOUD"),
            place
                .put(E::text(
                    ns(),
                    &format!("detail-{index}"),
                    &[[x, y - 2.2, 0.85]],
                    subtitle,
                ))
                .with_style(draw_style::LABEL)
                .with_width(-11.0)
                .with_color(rgba(214, 226, 236, 255))
                .with_prop("accent", &tint.to_string()),
            place
                .put(E::shapes(
                    ns(),
                    &format!("bounds-{index}"),
                    shape_kind::BOX,
                    &[Instance::at([x, y, 2.0], [5.0, 4.2, 2.6])],
                ))
                .with_style(draw_style::HOLO)
                .with_fill(draw_flag::FILL_WIRE)
                .with_width(-0.8)
                .with_color(tint)
                .with_glow(0.18)
                .with_blur(0.2),
            place
                .put(E::lines(
                    ns(),
                    &format!("rim-{index}"),
                    &[
                        [x - 2.5, y - 2.1, 0.7],
                        [x + 2.5, y - 2.1, 0.7],
                        [x + 2.5, y + 2.1, 0.7],
                        [x - 2.5, y + 2.1, 0.7],
                        [x - 2.5, y - 2.1, 0.7],
                    ],
                ))
                .with_style(draw_style::LIGHT_WALL)
                .with_width(0.02)
                .with_beam(0.16)
                .with_color(tint)
                .with_glow(0.3)
                .with_blur(0.5)
                .with_flow(0.4, 0.7),
        ];
        for object in objects {
            let status = client.draw(&object)?;
            if !status.is_ok() {
                return Err(status.message().into());
            }
        }
        if palettes {
            for band in [0, 1, 7, 8, 14] {
                let label = place
                    .put(E::text(
                        ns(),
                        &format!("value-{index}-{band}"),
                        &[[x - 2.2 + band as f32 * 4.4 / 16.0 + 0.09, y, 0.75]],
                        &PALETTE_VALUES[band].to_string(),
                    ))
                    .with_style(draw_style::LABEL)
                    .with_width(-10.0)
                    .with_color(rgba(224, 235, 242, 255))
                    .with_prop("accent", &tint.to_string());
                let status = client.draw(&label)?;
                if !status.is_ok() {
                    return Err(status.message().into());
                }
            }
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let instance = std::env::args()
        .nth(1)
        .ok_or("provide INSTANCE or manifest")?;
    if instance == "manifest" {
        println!("{}", manifest());
        return Ok(());
    }
    let entry = registry::find(&instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "typed-cloud-basics")?;
    let deadline = || Instant::now() + Duration::from_secs(30);
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(8, deadline())?;
    let clear = DataRequest {
        host_epoch: pipeline.host_epoch(),
        lease: writer.lease(),
        ..DataRequest::new(data_op::CLEAR)
    }
    .with_metadata(&NamespaceFilter {
        namespace: ns().into(),
    })?;
    let reply = pipeline
        .submit(&clear, deadline(), gearbox_api::DataAdmission::Wait)?
        .wait(deadline())?;
    if reply.code != data_code::OK {
        return Err(format!("clear failed: {}", String::from_utf8_lossy(&reply.message)).into());
    }
    let shared = std::env::var_os("GEARBOX_CLOUD_NS").is_some();
    let clear = client.draw_clear(if shared { ns() } else { "vfx-gallery" })?;
    if !clear.is_ok() {
        return Err(clear.message().into());
    }
    let precision = std::env::var("GEARBOX_CLOUD_PRECISION").is_ok_and(|value| value == "1");
    let clocks = precision || std::env::var("GEARBOX_CLOUD_CLOCKS").is_ok_and(|value| value == "1");
    let palettes =
        !clocks && std::env::var("GEARBOX_CLOUD_PALETTES").is_ok_and(|value| value == "1");
    let schema = if precision {
        precision::schema()
    } else {
        schema()
    };
    let cloud = if precision {
        precision::page(0, 0, 0)
    } else if clocks {
        clocks::page(0, 0)
    } else if palettes {
        palette_cloud()
    } else {
        cloud()
    };
    let clock_pages = (clocks && !precision)
        .then(|| clocks::pages(&schema))
        .transpose()?;
    let clock_datum = clocks.then(|| clocks::datum(&client)).transpose()?;
    let points_per_view = if clocks { clocks::POINTS } else { cloud.count };
    let bytes = cloud.encode(&schema)?;
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: cloud.count,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let mut revisions = Vec::new();
    let mut targets = Vec::new();
    let clipped =
        !clocks && !palettes && std::env::var("GEARBOX_CLOUD_CLIP").is_ok_and(|value| value == "1");
    for (index, [x, y]) in centers().into_iter().enumerate() {
        let descriptor = DataDescriptor {
            revision: SCHEMA_REVISION,
            key: DataKey::new(ns(), &format!("view-{index}"))?,
            shape: DataShape::Cloud {
                schema: schema.clone(),
            },
            stream: if precision {
                precision::stream()
            } else if clocks {
                clocks::stream()
            } else {
                StreamMode::RetainedMap
            },
            lifetime_s: None,
            presentation: Presentation {
                clip: if clipped { clip_bounds(index) } else { None },
                ..if precision {
                    precision::presentation(index)
                } else if clocks {
                    clocks::presentation(index)
                } else if palettes {
                    palette_presentation(index)
                } else {
                    presentation(index)
                }
            },
        };
        let scan = ScanDescriptor {
            revision: SCHEMA_REVISION,
            scan_key: 0,
            source: Some(
                if palettes {
                    "synthetic/raw-palette-bands"
                } else {
                    "synthetic/room-and-sphere"
                }
                .into(),
            ),
            frame: DataFrame {
                origin_m: [x, y, 0.0],
                ..Default::default()
            },
            source_count: cloud.count as u64,
            organization: None,
        };
        let scans = if let Some(datum) = &clock_datum {
            if precision {
                precision::scans(index, datum)
            } else {
                clocks::scans(index, datum)
            }
        } else {
            vec![scan]
        };
        descriptor.validate_scans(&scans)?;
        let precision_pages = precision
            .then(|| precision::pages(&schema, index))
            .transpose()?;
        let pages: Vec<_> =
            if let Some(clock_pages) = precision_pages.as_ref().or(clock_pages.as_ref()) {
                clock_pages
                    .iter()
                    .map(|(descriptor, bytes)| DataUploadPage { descriptor, bytes })
                    .collect()
            } else {
                vec![DataUploadPage {
                    descriptor: &page,
                    bytes: &bytes,
                }]
            };
        let reply = writer.upload(
            &DataUpload::replace(&descriptor, &scans, &pages),
            deadline(),
            None,
        )?;
        targets.push((
            descriptor.key.clone(),
            DatasetRevision {
                incarnation: reply.incarnation,
                generation: reply.stored_generation,
            },
        ));
        revisions.push(serde_json::json!({"key":descriptor.key,"incarnation":reply.incarnation,"stored_generation":reply.stored_generation}));
    }
    writer.close(deadline())?;
    annotations(&client, clipped, palettes, clocks, precision)?;
    let mut data = client.draw_data()?;
    let mut visible = Vec::new();
    for (key, revision) in targets {
        let completion = data.wait_visible_with_deadline(&key, revision, None, deadline())?;
        visible.push(serde_json::json!({"key":key, "visibility":completion.visibility()}));
    }
    println!(
        "{}",
        serde_json::json!({"scene":"cloud-basics","clipped":clipped,"palettes":palettes,"clocks":clocks,"precision":precision,"transport":"public typed writer","points_per_view":points_per_view,"datasets":revisions,"visible_generation_ack":visible})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_preview_uses_raw_values_and_all_four_indexed_maps() {
        let cloud = palette_cloud();
        let schema = schema();
        cloud.encode(&schema).unwrap();
        assert_eq!(cloud.count, 3072);
        for (index, value) in PALETTE_VALUES.into_iter().enumerate() {
            assert_eq!(
                &cloud.columns[3].values[index * 192 * 4..index * 192 * 4 + 4],
                &value.to_le_bytes()
            );
        }
        for index in 0..4 {
            palette_presentation(index)
                .validate(Some(&schema), None, false)
                .unwrap();
        }
    }
    #[test]
    fn scene_uses_valid_typed_columns_and_separated_frames() {
        let schema = schema();
        let cloud = cloud();
        schema.validate().unwrap();
        assert_eq!(cloud.count, 7896);
        cloud.encode(&schema).unwrap();
        for index in 0..4 {
            presentation(index)
                .validate(Some(&schema), None, false)
                .unwrap();
        }
        assert!(CENTERS.windows(2).all(|pair| pair[0] != pair[1]));
        assert_eq!(
            u64::from_le_bytes(cloud.columns[1].values[..8].try_into().unwrap()),
            u64::MAX
        );
    }

    #[test]
    fn clipping_preview_has_unchanged_partial_culled_and_height_slice_cases() {
        let cloud = cloud();
        let mut counts = Vec::new();
        for index in 0..4 {
            let bounds = clip_bounds(index);
            if let Some(bounds) = bounds {
                bounds.validate().unwrap();
            }
            let [x, y] = CENTERS[index];
            let selected = cloud.columns[0]
                .values
                .chunks_exact(24)
                .filter(|bytes| {
                    let point = std::array::from_fn::<_, 3, _>(|axis| {
                        f64::from_le_bytes(bytes[axis * 8..axis * 8 + 8].try_into().unwrap())
                            + [x, y, 0.0][axis]
                    });
                    bounds.is_none_or(|bounds| {
                        (0..3).all(|axis| {
                            point[axis] >= bounds.min_m[axis] && point[axis] <= bounds.max_m[axis]
                        })
                    })
                })
                .count();
            counts.push(selected);
        }
        assert_eq!(counts[0], cloud.count as usize);
        assert_eq!(counts[2], 0);
        assert!(counts[1] > 0 && counts[1] < counts[0]);
        assert!(counts[3] > 0 && counts[3] < counts[0]);
    }
}
