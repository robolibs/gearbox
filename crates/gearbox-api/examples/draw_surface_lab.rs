//! Surface lab: four fixtures (plane, curved shell, step discontinuity, thin
//! wall), each shown as raw points, supplied surfels and a supplied mesh side
//! by side, clearly labelled. Nothing here is reconstructed: every surfel normal
//! and every mesh triangle is the caller's own, and gaps stay gaps.
//!
//! Usage: `draw_surface_lab INSTANCE`.

use std::time::{Duration, Instant};

use gearbox_api::wire::draw_data::mesh::{MeshBlock, encode_mesh_block};
use gearbox_api::wire::{draw::rgba, draw_data::*};
use gearbox_api::{Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawDataWriter, IdentitySource, colormap, registry};

#[path = "draw_lab/mod.rs"]
mod lab;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-gallery/plan/surfaces";
/// South-west corner of the lab, east and north of home.
const AT: [f64; 2] = [110.0, -30.0];
const SPACING: f64 = 0.1;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

/// A surface sampled on a u × v grid: position and unit normal per sample, and
/// whether two neighbours are joined by real surface (false across a gap).
struct Fixture {
    name: &'static str,
    title: &'static str,
    detail: &'static str,
    width: usize,
    height: usize,
    sample: fn(f64, f64) -> Option<([f64; 3], [f64; 3])>,
    joined: fn([f64; 2], [f64; 2]) -> bool,
}

fn plane(u: f64, v: f64) -> Option<([f64; 3], [f64; 3])> {
    Some(([u, v, 1.0], [0.0, 0.0, 1.0]))
}

fn curved(u: f64, v: f64) -> Option<([f64; 3], [f64; 3])> {
    let (r, a) = (1.8, (u - 1.5) / 1.8);
    Some(([1.5 + r * a.sin(), v, 0.4 + r * a.cos() - 0.2], [a.sin(), 0.0, a.cos()]))
}

fn step(u: f64, v: f64) -> Option<([f64; 3], [f64; 3])> {
    if (1.4..1.6).contains(&u) {
        return None;
    }
    Some(([u, v, if u < 1.5 { 0.8 } else { 1.4 }], [0.0, 0.0, 1.0]))
}

fn step_joined(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] < 1.5) == (b[0] < 1.5)
}

/// Two faces of a 6 cm wall: u < 1.5 is the west face, the rest the east face.
fn wall(u: f64, v: f64) -> Option<([f64; 3], [f64; 3])> {
    if u < 1.5 {
        Some(([1.47, v, 0.3 + u], [-1.0, 0.0, 0.0]))
    } else {
        Some(([1.53, v, 0.3 + (u - 1.5)], [1.0, 0.0, 0.0]))
    }
}

fn always(_: [f64; 2], _: [f64; 2]) -> bool {
    true
}

fn fixtures() -> [Fixture; 4] {
    [
        Fixture { name: "plane", title: "PLANE", detail: "ONE SUPPORTED SURFACE", width: 30, height: 30, sample: plane, joined: always },
        Fixture { name: "curved", title: "CURVED SHELL", detail: "NORMALS FOLLOW THE CURVE", width: 30, height: 30, sample: curved, joined: always },
        Fixture { name: "step", title: "STEP DISCONTINUITY", detail: "THE GAP STAYS OPEN", width: 30, height: 30, sample: step, joined: step_joined },
        Fixture { name: "wall", title: "THIN WALL", detail: "TWO FACES 6 CM APART, NEVER FUSED", width: 30, height: 30, sample: wall, joined: step_joined },
    ]
}

/// Fixture coordinates moved so the 3 m square is centred on the dataset origin.
fn centred(p: [f64; 3]) -> [f64; 3] {
    [p[0] - 1.5, p[1] - 1.5, p[2]]
}

fn samples(f: &Fixture, every: usize) -> Vec<([f64; 3], [f64; 3], [f64; 2])> {
    let mut out = Vec::new();
    for j in (0..f.height).step_by(every) {
        for i in (0..f.width).step_by(every) {
            let uv = [i as f64 * SPACING, j as f64 * SPACING];
            if let Some((p, n)) = (f.sample)(uv[0], uv[1]) {
                out.push((centred(p), n, uv));
            }
        }
    }
    out
}

fn field(name: &str, scalar: ScalarType, components: u8, semantic: AttributeSemantic, units: &str, nullable: bool) -> Attribute {
    Attribute { name: name.into(), scalar, components, semantic, units: units.into(), normalization: Normalization::None, nullable }
}

fn upload(writer: &mut DrawDataWriter<'_>, descriptor: &DataDescriptor, scans: &[ScanDescriptor], pages: &[(PageDescriptor, Vec<u8>)]) -> Result<()> {
    let borrowed: Vec<_> = pages.iter().map(|(d, b)| DataUploadPage { descriptor: d, bytes: b }).collect();
    writer.upload(&DataUpload::replace(descriptor, scans, &borrowed), deadline(), None)?;
    Ok(())
}

fn page(encoding: PageEncoding, id: u64, items: u32, bytes: Vec<u8>) -> (PageDescriptor, Vec<u8>) {
    (
        PageDescriptor {
            address: PageAddress { scan_key: 0, page_id: id },
            encoding,
            items,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [0.0; 3],
        },
        bytes,
    )
}

fn raw(writer: &mut DrawDataWriter<'_>, f: &Fixture, origin: [f64; 3]) -> Result<()> {
    let schema = CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            field("position", ScalarType::F32, 3, AttributeSemantic::Position, "m", false),
            field("id", ScalarType::U64, 1, AttributeSemantic::SourceId, "", false),
        ],
    };
    let points = samples(f, 1);
    let columns = vec![
        ColumnData { values: points.iter().flat_map(|(p, ..)| p.map(|v| v as f32)).flat_map(f32::to_le_bytes).collect(), validity: vec![] },
        ColumnData { values: (0..points.len() as u64).flat_map(u64::to_le_bytes).collect(), validity: vec![] },
    ];
    let bytes = CloudPage { count: points.len() as u32, columns }.encode(&schema)?;
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, &format!("{}-raw", f.name))?,
        shape: DataShape::Cloud { schema },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation { color: ColorMode::Constant { rgba: [215, 220, 230, 255] }, size: PointSize::Metres { radius: 0.02 }, glow: 0.5, ..Default::default() },
    };
    let scan = ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: Some(format!("surface-lab/{}", f.name)),
        frame: DataFrame { origin_m: origin, ground_radius_m: Some(2.2), ..Default::default() },
        source_count: points.len() as u64,
        organization: None,
    };
    upload(writer, &descriptor, &[scan], &[page(PageEncoding::CloudColumnsV1, 0, points.len() as u32, bytes)])
}

fn surfels(writer: &mut DrawDataWriter<'_>, f: &Fixture, origin: [f64; 3]) -> Result<()> {
    let schema = CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            field("position", ScalarType::F32, 3, AttributeSemantic::Position, "m", false),
            field("id", ScalarType::U64, 1, AttributeSemantic::SourceId, "", false),
            field("normal", ScalarType::F32, 3, AttributeSemantic::Normal, "", true),
            field("radius", ScalarType::F32, 1, AttributeSemantic::Radius, "m", false),
            field("confidence", ScalarType::F32, 1, AttributeSemantic::Confidence, "", false),
        ],
    };
    let points = samples(f, 2);
    let mut columns = vec![ColumnData { values: vec![], validity: vec![] }; 5];
    for (i, (p, n, uv)) in points.iter().enumerate() {
        columns[0].values.extend(p.iter().flat_map(|v| (*v as f32).to_le_bytes()));
        columns[1].values.extend((i as u64).to_le_bytes());
        columns[2].values.extend(n.iter().flat_map(|v| (*v as f32).to_le_bytes()));
        columns[3].values.extend(0.12f32.to_le_bytes());
        columns[4].values.extend(((uv[1] / 3.0) as f32).clamp(0.0, 1.0).to_le_bytes());
    }
    let bytes = CloudPage { count: points.len() as u32, columns }.encode(&schema)?;
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, &format!("{}-surfels", f.name))?,
        shape: DataShape::Surfels { schema, frame: DataFrame { origin_m: origin, ground_radius_m: Some(2.2), ..Default::default() } },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Scalar { name: "confidence".into(), colormap: colormap::VIRIDIS, range: [0.0, 1.0], missing: [120, 120, 120, 255] },
            normals: Some(NormalArrows { every: 9, length_m: 0.35, rgba: [255, 240, 200, 255] }),
            glow: 0.6,
            ..Default::default()
        },
    };
    upload(writer, &descriptor, &[], &[page(PageEncoding::CloudColumnsV1, 0, points.len() as u32, bytes)])
}

fn mesh(writer: &mut DrawDataWriter<'_>, f: &Fixture, origin: [f64; 3]) -> Result<()> {
    let (w, h) = (f.width, f.height);
    let mut index = vec![None; w * h];
    let mut block = MeshBlock { normals: Some(vec![]), confidence: Some(vec![]), ..MeshBlock::default() };
    for j in 0..h {
        for i in 0..w {
            let uv = [i as f64 * SPACING, j as f64 * SPACING];
            if let Some((p, n)) = (f.sample)(uv[0], uv[1]) {
                index[j * w + i] = Some((block.positions.len() as u32, uv));
                block.positions.push(centred(p).map(|v| v as f32));
                block.normals.as_mut().unwrap().push(n.map(|v| v as f32));
                block.confidence.as_mut().unwrap().push(((uv[0] / 3.0) as f32).clamp(0.0, 1.0));
            }
        }
    }
    for j in 0..h - 1 {
        for i in 0..w - 1 {
            let quad = [index[j * w + i], index[j * w + i + 1], index[(j + 1) * w + i + 1], index[(j + 1) * w + i]];
            let [Some(a), Some(b), Some(c), Some(d)] = quad else { continue };
            if !(f.joined)(a.1, b.1) || !(f.joined)(a.1, c.1) || !(f.joined)(a.1, d.1) {
                continue;
            }
            block.indices.extend([a.0, b.0, c.0, a.0, c.0, d.0]);
        }
    }
    let triangles = block.triangles() as u32;
    let bytes = encode_mesh_block(&block)?;
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, &format!("{}-mesh", f.name))?,
        shape: DataShape::MeshBlocks { frame: DataFrame { origin_m: origin, ground_radius_m: Some(2.2), ..Default::default() } },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Scalar { name: "confidence".into(), colormap: colormap::VIRIDIS, range: [0.0, 1.0], missing: [120, 120, 120, 255] },
            glow: 0.6,
            opacity: 0.75,
            ..Default::default()
        },
    };
    upload(writer, &descriptor, &[], &[page(PageEncoding::MeshBlockV1, 0, triangles, bytes)])
}

/// Bay centre of fixture row `row`; columns sit 5 m apart west to east.
fn bay_at(row: usize) -> [f64; 2] {
    [AT[0] + 5.0, AT[1] + row as f64 * 8.0]
}

fn labels(client: &Client) -> Result<()> {
    let tints = [rgba(110, 200, 235, 255), rgba(120, 220, 160, 255), rgba(235, 160, 80, 255), rgba(160, 130, 255, 255)];
    let tint = tints[1];
    let ns = format!("{NS}/labels");
    let bays: Vec<lab::Bay<'_>> = fixtures()
        .iter()
        .enumerate()
        .map(|(row, f)| lab::Bay {
            id: f.name,
            at: bay_at(row),
            size: [15.0, 4.0, 2.6],
            title: format!("{:02} / {}", row + 1, f.title),
            detail: f.detail,
            category: "TYPED SURFACE",
            tint: tints[row],
        })
        .collect();
    lab::bays(client, &ns, &bays)?;
    let top = bay_at(3);
    for (column, name) in ["RAW POINTS", "SUPPLIED SURFELS", "SUPPLIED MESH"].iter().enumerate() {
        let at = [top[0] - 5.0 + column as f64 * 5.0, top[1] + 3.0, 4.6];
        lab::label(client, &ns, &format!("column-{column}"), at, name, -13.0, tint, None)?;
    }
    lab::label(client, &ns, "title", [top[0], top[1] + 5.0, 6.2], "SURFACE LAB / SUPPLIED SURFELS AND MESH", -17.0, tint, Some("TYPED SURFACES"))
}

fn main() -> Result<()> {
    let instance = std::env::args().nth(1).ok_or("usage: draw_surface_lab INSTANCE")?;
    let entry = registry::find(&instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "surface-lab")?;
    let mut data = client.draw_data()?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(4, deadline())?;
    let mut clear = DataRequest { lease: writer.lease(), ..DataRequest::new(data_op::CLEAR) }
        .with_metadata(&NamespaceFilter { namespace: NS.into() })?;
    clear.host_epoch = data.capabilities().host_epoch;
    data.request_with_deadline(&clear, deadline())?;
    let _ = client.draw_clear(&format!("{NS}/labels"));
    labels(&client)?;
    let mut uploaded = Vec::new();
    for (row, f) in fixtures().iter().enumerate() {
        let [x, y] = bay_at(row);
        raw(&mut writer, f, [x - 5.0, y, 0.0])?;
        surfels(&mut writer, f, [x, y, 0.0])?;
        mesh(&mut writer, f, [x + 5.0, y, 0.0])?;
        uploaded.push(f.name);
    }
    writer.close(deadline())?;
    println!("{}", serde_json::json!({ "scene": "surface-lab", "fixtures": uploaded, "columns": ["raw", "surfels", "mesh"] }));
    Ok(())
}
