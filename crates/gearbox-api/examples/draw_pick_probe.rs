//! Native data picking cases on a flat-terrain headless instance: each case aims
//! the camera so a known fixture sample sits at the viewport centre, clicks there
//! with scripted input and checks the exact identity the host answers with.
//!
//! Usage: `draw_pick_probe INSTANCE GEARBOX_BIN WIDTH HEIGHT`; prints one JSON
//! line per case and exits non-zero when any case fails.

use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};

use gearbox_api::peerbus::Subscriber;
use gearbox_api::wire::draw_data::*;
use gearbox_api::{
    Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawDataWriter, DrawEntity as E, Env,
    IdentitySource, SceneEvent, event_kind, interact_kind, next_sample, registry,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-test/pick";
const UI: &str = "vfx-test/pick-ui";
/// The wall: 11 × 11 samples 0.6 m apart in the east/up plane at this northing.
const WALL_N: f64 = 20.0;
const SIDE: usize = 11;
const STEP: f64 = 0.6;
const WALL_BOTTOM: f64 = 0.5;
const CENTRE: u64 = 1000 + (SIDE * (SIDE / 2) + SIDE / 2) as u64;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

struct Probe<'a> {
    client: &'a Client,
    bin: &'a str,
    instance: &'a str,
    home: [f64; 2],
    centre: [f64; 2],
    events: Subscriber<Env>,
    pointer: bool,
    inspect: bool,
    next: u32,
}

#[derive(Debug, Default)]
struct Answer {
    what: String,
    at: [f32; 3],
    props: HashMap<String, String>,
}

impl Probe<'_> {
    fn cli(&self, args: &[&str]) -> Result<String> {
        let output = Command::new(self.bin)
            .args(["--instance", self.instance, "--timeout", "30"])
            .args(args)
            .output()?;
        if !output.status.success() {
            return Err(
                format!("CLI {args:?}: {}", String::from_utf8_lossy(&output.stderr)).into(),
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Latitude and longitude of a point `east`, `north` metres from home.
    fn lat_lon(&self, east: f64, north: f64) -> (f64, f64) {
        let lat = self.home[0].to_radians();
        let (a, e2) = (6378137.0, 0.00669437999014);
        let d = 1.0 - e2 * lat.sin().powi(2);
        let m = a * (1.0 - e2) / d.powf(1.5);
        let n = a / d.sqrt();
        (
            self.home[0] + (north / m).to_degrees(),
            self.home[1] + (east / (n * lat.cos())).to_degrees(),
        )
    }

    fn look(
        &self,
        at: [f64; 3],
        bearing: f64,
        pitch: f64,
        distance: f64,
        over: Option<f64>,
    ) -> Result<()> {
        let (lat, lon) = self.lat_lon(at[0], at[1]);
        let mut args = vec![
            "instance".to_string(),
            "camera".into(),
            "look".into(),
            "--lat".into(),
            lat.to_string(),
            "--lon".into(),
            lon.to_string(),
            "--above".into(),
            at[2].to_string(),
            "--bearing".into(),
            bearing.to_string(),
            "--pitch".into(),
            pitch.to_string(),
            "--distance".into(),
            distance.to_string(),
        ];
        if let Some(over) = over {
            args.extend(["--over".into(), over.to_string()]);
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.cli(&args)?;
        if over.is_none() {
            self.settle()?;
        }
        Ok(())
    }

    /// Waits until the view stops easing: three equal reports in a row.
    fn settle(&self) -> Result<()> {
        let until = Instant::now() + Duration::from_secs(8);
        let (mut last, mut same) = (String::new(), 0);
        while Instant::now() < until && same < 3 {
            std::thread::sleep(Duration::from_millis(250));
            let now = self.cli(&["--json", "instance", "camera", "where"])?;
            same = if now == last { same + 1 } else { 0 };
            last = now;
        }
        std::thread::sleep(Duration::from_millis(300));
        Ok(())
    }

    /// A pick interaction; without a pointer, data picks ask for the viewport centre.
    fn ask(&mut self, kind: u32, prefix: &str) -> Result<String> {
        self.next += 1;
        let id = format!("ask-{}", self.next);
        let mut entity = E::interact(UI, &id, kind)
            .in_frame("world")
            .with_prop("prefix", prefix);
        if !self.pointer && kind == interact_kind::PICK_DATA {
            entity = entity.with_prop("ndc", "0 0");
        }
        if self.inspect {
            entity = entity.with_prop("inspect", "label");
        }
        let status = self.client.draw(&entity)?;
        if !status.is_ok() {
            return Err(status.message().into());
        }
        if self.pointer {
            std::thread::sleep(Duration::from_millis(300));
        }
        Ok(id)
    }

    fn ui(&self, steps: &str) -> Result<()> {
        self.cli(&["instance", "ui", steps]).map(|_| ())
    }

    fn click(&self) -> Result<()> {
        self.ui(&format!("click {} {}", self.centre[0], self.centre[1]))
    }

    fn answer(&mut self, id: &str, timeout: Duration) -> Result<Option<Answer>> {
        let until = Instant::now() + timeout;
        while Instant::now() < until {
            let Some(event) =
                next_sample::<SceneEvent>(&mut self.events, Duration::from_millis(200))?
            else {
                continue;
            };
            let props = event.props();
            if event.kind != event_kind::DRAW || props.get("id").as_deref() != Some(id) {
                continue;
            }
            let map: HashMap<String, String> = props.iter().into_iter().collect();
            return Ok(Some(Answer {
                what: map.get("what").cloned().unwrap_or_default(),
                at: [event.x, event.y, event.z],
                props: map,
            }));
        }
        Ok(None)
    }

    fn pick(&mut self, kind: u32, prefix: &str) -> Result<Answer> {
        let id = self.ask(kind, prefix)?;
        if self.pointer {
            self.click()?;
        }
        self.answer(&id, Duration::from_secs(10))?
            .ok_or_else(|| "no answer to the click".into())
    }
}

fn report(case: &str, pass: bool, detail: impl serde::Serialize) -> bool {
    println!(
        "{}",
        serde_json::json!({ "case": case, "result": if pass { "PASS" } else { "FAIL" }, "detail": detail })
    );
    pass
}

fn schema() -> CloudSchema {
    let field = |name: &str, scalar, components, semantic, units: &str, nullable| Attribute {
        name: name.into(),
        scalar,
        components,
        semantic,
        units: units.into(),
        normalization: Normalization::None,
        nullable,
    };
    CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            field(
                "position",
                ScalarType::F64,
                3,
                AttributeSemantic::Position,
                "m",
                false,
            ),
            field(
                "id",
                ScalarType::U64,
                1,
                AttributeSemantic::SourceId,
                "",
                false,
            ),
            field(
                "temperature",
                ScalarType::F32,
                1,
                AttributeSemantic::Custom,
                "degC",
                false,
            ),
        ],
    }
}

fn cloud(
    writer: &mut DrawDataWriter<'_>,
    id: &str,
    points: &[([f64; 3], u64)],
    clip: Option<ClipBounds>,
    base: Option<DatasetRevision>,
) -> Result<(DataDescriptor, DatasetRevision)> {
    let schema = schema();
    let mut columns = vec![
        ColumnData {
            values: Vec::new(),
            validity: Vec::new()
        };
        3
    ];
    for (xyz, source) in points {
        columns[0]
            .values
            .extend(xyz.iter().flat_map(|v| v.to_le_bytes()));
        columns[1].values.extend(source.to_le_bytes());
        columns[2]
            .values
            .extend((*source as f32 / 10.0).to_le_bytes());
    }
    let page = CloudPage {
        count: points.len() as u32,
        columns,
    };
    let bytes = page.encode(&schema)?;
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, id)?,
        shape: DataShape::Cloud { schema },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Constant {
                rgba: [240, 180, 60, 255],
            },
            size: PointSize::Metres { radius: 0.12 },
            clip,
            ..Default::default()
        },
    };
    let scan = ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: Some("pick fixture".into()),
        frame: DataFrame::default(),
        source_count: points.len() as u64,
        organization: None,
    };
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: points.len() as u32,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let pages = [DataUploadPage {
        descriptor: &page,
        bytes: &bytes,
    }];
    let mut upload = DataUpload::replace(&descriptor, std::slice::from_ref(&scan), &pages);
    upload.base = base;
    let reply = writer.upload(&upload, deadline(), None)?;
    Ok((
        descriptor,
        DatasetRevision {
            incarnation: reply.incarnation,
            generation: reply.stored_generation,
        },
    ))
}

fn wall(east: f64) -> Vec<([f64; 3], u64)> {
    (0..SIDE * SIDE)
        .map(|i| {
            let (row, col) = (i / SIDE, i % SIDE);
            let x = east + (col as f64 - (SIDE / 2) as f64) * STEP;
            (
                [x, WALL_N, WALL_BOTTOM + row as f64 * STEP],
                1000 + i as u64,
            )
        })
        .collect()
}

fn wall_centre(east: f64) -> [f64; 3] {
    [east, WALL_N, WALL_BOTTOM + (SIDE / 2) as f64 * STEP]
}

fn octree_config() -> OctreeConfig {
    OctreeConfig {
        leaf_resolution_m: 0.1,
        max_depth: 8,
        free_max: 0.3,
        occupied_min: 0.7,
    }
}

/// West of home, where `look --above` and the flat physics ground agree.
const OCTREE_AT: [f64; 3] = [-16.0, WALL_N, 0.0];

fn cell_key(cell: [i128; 3]) -> Result<VoxelKey> {
    let config = octree_config();
    Ok(VoxelKey::from_grid_cell(cell.map(|c| c << 3), 5, &config)?)
}

/// Centre of the south face of depth-5 cell `cell`, in world metres.
fn face(cell: [i128; 3]) -> [f64; 3] {
    [
        OCTREE_AT[0] + (cell[0] as f64 + 0.5) * 0.8,
        OCTREE_AT[1] + cell[1] as f64 * 0.8,
        OCTREE_AT[2] + (cell[2] as f64 + 0.5) * 0.8,
    ]
}

fn octree(writer: &mut DrawDataWriter<'_>) -> Result<DatasetRevision> {
    let config = octree_config();
    let mut leaves = Vec::new();
    for x in 0..3 {
        for z in 0..5 {
            leaves.push(OccupancyLeaf {
                key: cell_key([x, 0, z])?,
                value: OccupancyValue {
                    probability: Some(0.9),
                    confidence: None,
                    class_id: None,
                },
            });
        }
    }
    leaves.push(OccupancyLeaf {
        key: cell_key([4, 0, 2])?,
        value: OccupancyValue {
            probability: None,
            confidence: None,
            class_id: None,
        },
    });
    let bytes = encode_occupancy_page(&config, &leaves)?;
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding: PageEncoding::OccupancyLeavesV1,
        items: leaves.len() as u32,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, "octree")?,
        shape: DataShape::Octree {
            config,
            frame: DataFrame {
                origin_m: OCTREE_AT,
                ..Default::default()
            },
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            octree: Some(OctreeDisplay {
                show_unknown: true,
                roi: Some(ClipBounds {
                    min_m: [0.0; 3],
                    max_m: [4.0, 0.8, 4.0],
                    ..Default::default()
                }),
                unknown_cell_budget: 2_000,
                ..OctreeDisplay::default()
            }),
            ..Presentation::for_octree()
        },
    };
    let pages = [DataUploadPage {
        descriptor: &page,
        bytes: &bytes,
    }];
    let reply = writer.upload(
        &DataUpload::replace(&descriptor, &[], &pages),
        deadline(),
        None,
    )?;
    Ok(DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    })
}

/// Surfel, mesh and volume fixtures on the wall line, each facing the camera north of it.
const SURFEL_AT: [f64; 3] = [40.0, WALL_N, 1.7];
const MESH_AT: [f64; 3] = [48.0, WALL_N, 1.7];
const VOLUME_AT: [f64; 3] = [-34.0, WALL_N - 0.4, 0.0];
const SURFEL_CENTRE: u64 = 512;

fn send(writer: &mut DrawDataWriter<'_>, descriptor: &DataDescriptor, page: PageDescriptor, bytes: &[u8]) -> Result<DatasetRevision> {
    let pages = [DataUploadPage { descriptor: &page, bytes }];
    let reply = writer.upload(&DataUpload::replace(descriptor, &[], &pages), deadline(), None)?;
    Ok(DatasetRevision { incarnation: reply.incarnation, generation: reply.stored_generation })
}

fn framed(id: &str, shape: DataShape, presentation: Presentation) -> Result<DataDescriptor> {
    Ok(DataDescriptor { revision: SCHEMA_REVISION, key: DataKey::new(NS, id)?, shape, stream: StreamMode::RetainedMap, lifetime_s: None, presentation })
}

fn field(name: &str, scalar: ScalarType, components: u8, semantic: AttributeSemantic, units: &str, nullable: bool) -> Attribute {
    Attribute { name: name.into(), scalar, components, semantic, units: units.into(), normalization: Normalization::None, nullable }
}

/// A 5 x 5 grid of north-facing discs centred on `SURFEL_AT`, IDs from 500.
fn surfels(writer: &mut DrawDataWriter<'_>) -> Result<DatasetRevision> {
    let schema = CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            field("position", ScalarType::F64, 3, AttributeSemantic::Position, "m", false),
            field("id", ScalarType::U64, 1, AttributeSemantic::SourceId, "", false),
            field("normal", ScalarType::F32, 3, AttributeSemantic::Normal, "", true),
            field("radius", ScalarType::F32, 1, AttributeSemantic::Radius, "m", false),
        ],
    };
    let mut columns = vec![ColumnData { values: vec![], validity: vec![] }; 4];
    for j in 0..5 {
        for i in 0..5 {
            let p = [SURFEL_AT[0] + (i as f64 - 2.0) * 0.6, SURFEL_AT[1], SURFEL_AT[2] + (j as f64 - 2.0) * 0.6];
            columns[0].values.extend(p.iter().flat_map(|v| v.to_le_bytes()));
            columns[1].values.extend((500 + (j * 5 + i) as u64).to_le_bytes());
            columns[2].values.extend([0.0f32, 1.0, 0.0].iter().flat_map(|v| v.to_le_bytes()));
            columns[3].values.extend(0.28f32.to_le_bytes());
        }
    }
    let bytes = CloudPage { count: 25, columns }.encode(&schema)?;
    let descriptor = framed(
        "surfels",
        DataShape::Surfels { schema, frame: DataFrame::default() },
        Presentation { color: ColorMode::Constant { rgba: [120, 220, 170, 255] }, ..Default::default() },
    )?;
    let page = PageDescriptor { address: PageAddress { scan_key: 0, page_id: 0 }, encoding: PageEncoding::CloudColumnsV1, items: 25, byte_len: bytes.len() as u64, crc32: page_crc32(&bytes), origin_m: [0.0; 3] };
    send(writer, &descriptor, page, &bytes)
}

/// One north-facing quad of two triangles without normals, centred on `MESH_AT`.
fn mesh(writer: &mut DrawDataWriter<'_>) -> Result<DatasetRevision> {
    use gearbox_api::wire::draw_data::mesh::{MeshBlock, encode_mesh_block};
    let corner = |dx: f64, dz: f64| [(MESH_AT[0] + dx) as f32, MESH_AT[1] as f32, (MESH_AT[2] + dz) as f32];
    let block = MeshBlock {
        positions: vec![corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)],
        indices: vec![0, 1, 2, 0, 2, 3],
        ..MeshBlock::default()
    };
    let bytes = encode_mesh_block(&block)?;
    let descriptor = framed(
        "mesh",
        DataShape::MeshBlocks { frame: DataFrame::default() },
        Presentation { color: ColorMode::Constant { rgba: [140, 170, 240, 255] }, ..Default::default() },
    )?;
    let page = PageDescriptor { address: PageAddress { scan_key: 0, page_id: 0 }, encoding: PageEncoding::MeshBlockV1, items: 2, byte_len: bytes.len() as u64, crc32: page_crc32(&bytes), origin_m: [0.0; 3] };
    send(writer, &descriptor, page, &bytes)
}

/// A field equal to height in the volume frame, unknown east of x = 0.8, sliced on the wall line.
fn volume(writer: &mut DrawDataWriter<'_>) -> Result<DatasetRevision> {
    use gearbox_api::wire::draw_data::volume::{VolumeBlock, encode_volume_block};
    let field = VolumeField { kind: FieldKind::Esdf { signed: true }, voxel_m: 0.1, block_side: 8, colors: false };
    let mut pages = Vec::new();
    for key in [[0, 0, 0], [1, 0, 0], [0, 0, 1], [1, 0, 1], [0, 0, 2], [1, 0, 2]] {
        let mut block = VolumeBlock { key, values: vec![0.0; 512], valid: vec![false; 512], weights: None, colors: None };
        for n in 0..512 {
            let local = [n % 8, n / 8 % 8, n / 64];
            let p = [0, 1, 2].map(|a| ((key[a] as usize * 8 + local[a]) as f64 + 0.5) * 0.1);
            block.valid[n] = p[0] < 0.8;
            block.values[n] = if block.valid[n] { p[2] as f32 } else { 0.0 };
        }
        let bytes = encode_volume_block(&field, &block)?;
        let page = PageDescriptor {
            address: PageAddress { scan_key: 0, page_id: volume_page_id(key).ok_or("key")? },
            encoding: PageEncoding::VolumeBlockV1,
            items: 512,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [0.0; 3],
        };
        pages.push((page, bytes));
    }
    let view = VolumeView {
        slices: vec![VolumeSlice::across(1, [0.8, 0.4, 1.2], [2.0, 2.2], 0.05)],
        interpolation: Interpolation::Trilinear,
        band: None,
        contours: vec![],
        contour_rgba: [255; 4],
        unknown_rgba: [96, 40, 112, 255],
        frame_rgba: None,
    };
    let descriptor = framed(
        "volume",
        DataShape::Volume { field, frame: DataFrame { origin_m: VOLUME_AT, ..Default::default() } },
        Presentation { color: ColorMode::Scalar { name: "value".into(), colormap: gearbox_api::colormap::TURBO, range: [0.0, 2.4], missing: [0; 4] }, volume: Some(view), ..Default::default() },
    )?;
    let borrowed: Vec<_> = pages.iter().map(|(d, b)| DataUploadPage { descriptor: d, bytes: b }).collect();
    let reply = writer.upload(&DataUpload::replace(&descriptor, &[], &borrowed), deadline(), None)?;
    Ok(DatasetRevision { incarnation: reply.incarnation, generation: reply.stored_generation })
}

/// A 4 m elevation tile standing 1 m high, with a 0.8 m hole at its centre.
const TERRAIN_AT: [f64; 3] = [64.0, WALL_N - 2.0, 0.0];

fn terrain(writer: &mut DrawDataWriter<'_>) -> Result<DatasetRevision> {
    use gearbox_api::wire::draw_data::elevation::encode_elevation_tile;
    let grid = ElevationGrid { cell_m: 0.25, tile_side: 16, layers: vec![] };
    let mut tile = ElevationTile { key: [0, 0], layer: 0, values: vec![0.0; 256], valid: vec![false; 256] };
    for n in 0..256 {
        let p = [(n % 16) as f64 * 0.25 + 0.125, (n / 16) as f64 * 0.25 + 0.125];
        if (p[0] - 2.0).hypot(p[1] - 2.0) > 0.8 {
            tile.valid[n] = true;
            tile.values[n] = 1.0;
        }
    }
    let bytes = encode_elevation_tile(&grid, &tile)?;
    let descriptor = framed(
        "terrain",
        DataShape::Elevation { grid, frame: DataFrame { origin_m: TERRAIN_AT, ..Default::default() } },
        Presentation { color: ColorMode::Constant { rgba: [150, 200, 120, 255] }, ..Default::default() },
    )?;
    let page = PageDescriptor {
        address: PageAddress { scan_key: 0, page_id: elevation_page_id([0, 0], 0).ok_or("key")? },
        encoding: PageEncoding::ElevationTileV1,
        items: 256,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    send(writer, &descriptor, page, &bytes)
}

fn hit(answer: &Answer, id: &str, revision: DatasetRevision, source: Option<u64>) -> bool {
    let p = |k: &str| answer.props.get(k).map(String::as_str);
    answer.what == "data"
        && p("data_ns") == Some(NS)
        && p("data_id") == Some(id)
        && p("generation") == Some(&revision.generation.to_string())
        && p("incarnation") == Some(&revision.incarnation.to_string())
        && source.is_none_or(|s| p("source_id") == Some(&s.to_string()))
}

fn near(a: [f32; 3], b: [f64; 3], tolerance: f64) -> bool {
    (0..3).all(|k| (f64::from(a[k]) - b[k]).abs() <= tolerance)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (pointer, args) = match args.split_last() {
        Some((last, rest)) if last == "pointer" => (true, rest),
        _ => (false, args.as_slice()),
    };
    let [instance, bin, width, height] = args else {
        return Err("usage: draw_pick_probe INSTANCE GEARBOX_BIN WIDTH HEIGHT [pointer]".into());
    };
    let (width, height): (f64, f64) = (width.parse()?, height.parse()?);
    let entry = registry::find(instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "pick-probe")?;
    let home: Vec<f64> = client
        .info()?
        .props()
        .get("home")
        .ok_or("host has no home datum")?
        .split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    let mut probe = Probe {
        client: &client,
        bin,
        instance,
        home: [home[0], home[1]],
        centre: [width / 2.0, height / 2.0],
        events: client.events()?,
        pointer,
        inspect: false,
        next: 0,
    };
    if let Ok(prefix) = std::env::var("GEARBOX_PICK_ONCE") {
        let a = probe.pick(interact_kind::PICK_DATA, &prefix)?;
        println!("{}", serde_json::json!({ "what": a.what, "at": a.at, "props": a.props }));
        return Ok(());
    }
    let mut data = client.draw_data()?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(4, deadline())?;
    let mut clear = DataRequest {
        lease: writer.lease(),
        ..DataRequest::new(data_op::CLEAR)
    }
    .with_metadata(&NamespaceFilter {
        namespace: NS.into(),
    })?;
    clear.host_epoch = data.capabilities().host_epoch;
    data.request_with_deadline(&clear, deadline())?;
    if pointer {
        let passed = pointer_cases(&mut probe, &mut writer)?;
        let _ = writer.close(deadline());
        let mut writer = pipeline.writer(4, deadline())?;
        clear.lease = writer.lease();
        data.request_with_deadline(&clear, deadline())?;
        writer.close(deadline())?;
        return if passed {
            Ok(())
        } else {
            Err("a pointer case failed".into())
        };
    }
    let (_, wall_rev) = cloud(&mut writer, "wall", &wall(0.0), None, None)?;
    let buried = [
        30.0,
        WALL_N + 3.0 * 30f64.to_radians().cos(),
        0.6 - 3.0 * 30f64.to_radians().sin(),
    ];
    let (_, _) = cloud(&mut writer, "buried", &[(buried, 7)], None, None)?;
    let centre_z = wall_centre(15.0)[2];
    let below = ClipBounds {
        min_m: [10.0, 15.0, -1.0],
        max_m: [20.0, 25.0, centre_z - 0.3],
        ..Default::default()
    };
    let (_, clipped) = cloud(&mut writer, "clipped", &wall(15.0), Some(below), None)?;
    let octree_rev = octree(&mut writer)?;
    let surfel_rev = surfels(&mut writer)?;
    let mesh_rev = mesh(&mut writer)?;
    let volume_rev = volume(&mut writer)?;
    let terrain_rev = terrain(&mut writer)?;
    writer.close(deadline())?;
    std::thread::sleep(Duration::from_secs(2));

    let mut passed = true;
    probe.look(wall_centre(0.0), 180.0, 2.0, 12.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    let unit = a.props.get("unit.temperature").map(String::as_str);
    let temperature = a
        .props
        .get("attr.temperature")
        .and_then(|v| v.parse::<f32>().ok());
    passed &= report(
        "click",
        hit(&a, "wall", wall_rev, Some(CENTRE))
            && near(a.at, wall_centre(0.0), 1e-3)
            && a.props.get("lod").map(String::as_str) == Some("full")
            && unit == Some("degC")
            && temperature == Some(CENTRE as f32 / 10.0),
        &a.props,
    );
    for (case, distance) in [("near camera", 4.0), ("far camera", 60.0)] {
        probe.look(wall_centre(0.0), 180.0, 2.0, distance, None)?;
        let a = probe.pick(interact_kind::PICK_DATA, NS)?;
        passed &= report(case, hit(&a, "wall", wall_rev, Some(CENTRE)), &a.props);
    }

    probe.look([30.0, WALL_N, 0.6], 180.0, 30.0, 10.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    passed &= report("occluded point", a.what == "no_data", &a.props);

    probe.look(wall_centre(15.0), 180.0, 2.0, 12.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    passed &= report("clipped point", a.what == "no_data", &a.props);
    let mut writer = pipeline.writer(4, deadline())?;
    let around = ClipBounds {
        min_m: [10.0, 15.0, centre_z - 0.2],
        max_m: [20.0, 25.0, centre_z + 0.2],
        ..Default::default()
    };
    let (_, inverted) = cloud(
        &mut writer,
        "clipped",
        &wall(15.0),
        Some(around),
        Some(clipped),
    )?;
    writer.close(deadline())?;
    std::thread::sleep(Duration::from_secs(2));
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    passed &= report(
        "clip inversion",
        hit(&a, "clipped", inverted, Some(CENTRE)),
        &a.props,
    );

    let target = face([1, 0, 2]);
    probe.look([target[0], target[1], target[2]], 180.0, 2.0, 10.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    let key = cell_key([1, 0, 2])?;
    let leaf = format!(
        "{},{},{}/{}/{}",
        key.tile[0], key.tile[1], key.tile[2], key.depth, key.morton
    );
    passed &= report(
        "octree leaf",
        hit(&a, "octree", octree_rev, None)
            && a.props.get("leaf") == Some(&leaf)
            && a.props.get("state").map(String::as_str) == Some("occupied")
            && a.props.get("lod").map(String::as_str) == Some("leaf"),
        (&a.props, a.at),
    );
    let target = face([4, 0, 2]);
    probe.look([target[0], target[1], target[2]], 180.0, 2.0, 10.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    let key = cell_key([4, 0, 2])?;
    let leaf = format!(
        "{},{},{}/{}/{}",
        key.tile[0], key.tile[1], key.tile[2], key.depth, key.morton
    );
    passed &= report(
        "unknown leaf",
        hit(&a, "octree", octree_rev, None)
            && a.props.get("leaf") == Some(&leaf)
            && a.props.get("state").map(String::as_str) == Some("unknown")
            && a.props.get("probability").map(String::as_str) == Some("none"),
        (&a.props, a.at),
    );

    probe.look(SURFEL_AT, 180.0, 2.0, 8.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    let prop = |a: &Answer, k: &str| a.props.get(k).cloned().unwrap_or_default();
    passed &= report(
        "surfel",
        hit(&a, "surfels", surfel_rev, Some(SURFEL_CENTRE)) && prop(&a, "kind") == "surfels" && prop(&a, "normal") == "supplied",
        &a.props,
    );
    probe.look(MESH_AT, 180.0, 2.0, 8.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    passed &= report(
        "mesh triangle",
        hit(&a, "mesh", mesh_rev, None)
            && prop(&a, "kind") == "mesh"
            && prop(&a, "normal") == "missing"
            && ["0", "1"].contains(&prop(&a, "triangle").as_str()),
        &a.props,
    );
    probe.look([VOLUME_AT[0] + 0.4, VOLUME_AT[1] + 0.4, 1.2], 180.0, 2.0, 6.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    let value = prop(&a, "value").parse::<f64>().ok();
    passed &= report(
        "volume slice value",
        hit(&a, "volume", volume_rev, None)
            && prop(&a, "kind") == "volume"
            && prop(&a, "support") == "valid"
            && value.is_some_and(|v| (v - 1.2).abs() < 0.03),
        &a.props,
    );
    probe.look([VOLUME_AT[0] + 1.2, VOLUME_AT[1] + 0.4, 1.2], 180.0, 2.0, 6.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    passed &= report(
        "volume unknown cell",
        hit(&a, "volume", volume_rev, None) && prop(&a, "support") == "unknown" && prop(&a, "value") == "unknown",
        &a.props,
    );

    probe.look([TERRAIN_AT[0] + 0.6, TERRAIN_AT[1] + 0.6, 1.0], 180.0, 75.0, 6.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    let elevation = prop(&a, "elevation").parse::<f64>().ok();
    passed &= report(
        "terrain quad",
        hit(&a, "terrain", terrain_rev, None) && prop(&a, "kind") == "elevation" && elevation.is_some_and(|e| (e - 1.0).abs() < 1e-6),
        &a.props,
    );
    probe.look([TERRAIN_AT[0] + 2.0, TERRAIN_AT[1] + 2.0, 1.0], 180.0, 75.0, 6.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    passed &= report("terrain hole", a.what == "no_data", &a.props);

    probe.look(wall_centre(0.0), 180.0, 2.0, 12.0, None)?;
    let a = probe.pick(interact_kind::PICK_DATA, "vfx-test/elsewhere")?;
    passed &= report("namespace isolation", a.what == "no_data", &a.props);
    let a = probe.pick(interact_kind::PICK_DATA, "vfx-test/pi")?;
    passed &= report("prefix is a namespace", a.what == "no_data", &a.props);

    probe.inspect = true;
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    probe.inspect = false;
    let id = a.props.get("id").cloned().unwrap_or_default();
    let label = client
        .draw_list(UI)?
        .into_iter()
        .find(|e| e.id() == format!("{id}-inspect"))
        .and_then(|e| e.props().get("text"));
    passed &= report(
        "inspection label",
        hit(&a, "wall", wall_rev, Some(CENTRE))
            && label.as_deref().is_some_and(|t| t.contains("#1060") && t.contains("temperature 106 degC")),
        &label,
    );
    let status = client.draw_clear(UI)?;
    passed &= report(
        "label cleared with its namespace",
        status.is_ok() && client.draw_list(UI)?.is_empty(),
        status.message(),
    );

    if !passed {
        return Err("a picking case failed".into());
    }
    Ok(())
}

/// A dense wall where real clicks are checked against the API pick at the
/// centre, then drag, cancel, a gliding camera and the ground tool.
const DENSE_AT: [f64; 2] = [-60.0, -40.0];

fn dense() -> Vec<([f64; 3], u64)> {
    let side = 61;
    (0..side * side)
        .map(|i| {
            let (row, col) = (i / side, i % side);
            let x = DENSE_AT[0] + (col as f64 - 30.0) * 0.1;
            ([x, DENSE_AT[1], 0.5 + row as f64 * 0.1], 5000 + i as u64)
        })
        .collect()
}

fn pointer_cases(probe: &mut Probe<'_>, writer: &mut DrawDataWriter<'_>) -> Result<bool> {
    let (_, revision) = cloud(writer, "dense", &dense(), None, None)?;
    std::thread::sleep(Duration::from_secs(2));
    let target = [DENSE_AT[0], DENSE_AT[1], 3.5];
    let mut passed = true;
    probe.look(target, 180.0, 2.0, 12.0, None)?;
    probe.pointer = false;
    let reference = probe.pick(interact_kind::PICK_DATA, NS)?;
    probe.pointer = true;
    let expected = reference
        .props
        .get("source_id")
        .and_then(|s| s.parse::<u64>().ok());
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    let centred = a.props.get("ndc").is_some_and(|ndc| {
        ndc.split_whitespace()
            .filter_map(|v| v.parse::<f32>().ok())
            .all(|v| v.abs() < 0.01)
    });
    passed &= report(
        "pointer click",
        expected.is_some() && hit(&a, "dense", revision, expected) && centred,
        (&a.props, &reference.props),
    );

    let id = probe.ask(interact_kind::PICK_DATA, NS)?;
    let (x, y) = (probe.centre[0], probe.centre[1]);
    probe.ui(&format!(
        "down {x} {y}; move {} {y}; move {} {y}; up {} {y}",
        x + 30.0,
        x + 60.0,
        x + 60.0
    ))?;
    let first = probe
        .answer(&id, Duration::from_secs(10))?
        .unwrap_or_default();
    let second = probe.answer(&id, Duration::from_secs(1))?;
    passed &= report(
        "drag",
        hit(&first, "dense", revision, expected) && second.is_none(),
        &first.props,
    );

    let id = probe.ask(interact_kind::PICK_DATA, NS)?;
    probe.ui("key Escape")?;
    let a = probe
        .answer(&id, Duration::from_secs(10))?
        .unwrap_or_default();
    passed &= report("cancel", a.what == "cancel", &a.props);

    probe.look(target, 120.0, 20.0, 30.0, Some(4.0))?;
    std::thread::sleep(Duration::from_millis(700));
    let a = probe.pick(interact_kind::PICK_DATA, NS)?;
    passed &= report(
        "stale camera",
        a.what == "stale" && a.props.get("reason").is_some_and(|r| r.contains("camera")),
        &a.props,
    );
    std::thread::sleep(Duration::from_secs(4));

    probe.look(
        [DENSE_AT[0] - 12.0, DENSE_AT[1], 0.0],
        180.0,
        30.0,
        10.0,
        None,
    )?;
    let a = probe.pick(interact_kind::PICK_POINT, "")?;
    passed &= report("ground pick unchanged", a.what == "point", [a.at]);
    Ok(passed)
}
