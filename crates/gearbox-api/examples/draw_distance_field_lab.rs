//! Distance-field lab: sparse volumes shown as slices. A TSDF of a sphere seen
//! by one sensor shows its sign with the zero crossing marked, its integration
//! weight, and the caller's supplied isosurface mesh; an ESDF of three
//! obstacles shows clearance on a level slice and on an oblique, banded one.
//! Unobserved space is unknown and blocks never sent are unallocated; both draw
//! as a checker, never as a distance. Probes read the field through the host.
//!
//! Usage: `draw_distance_field_lab INSTANCE [sweep SECONDS]`; `sweep` moves
//! the oblique slice by presentation updates alone.

use std::time::{Duration, Instant};

use gearbox_api::wire::draw_data::mesh::{MeshBlock, encode_mesh_block};
use gearbox_api::wire::draw_data::volume::{VolumeBlock, encode_volume_block};
use gearbox_api::wire::{draw::rgba, draw_data::*};
use gearbox_api::{Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawDataWriter, IdentitySource, colormap, registry};

#[path = "draw_lab/mod.rs"]
mod lab;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-gallery/plan/fields";
/// South-west corner of the lab, east of the surface lab.
const AT: [f64; 2] = [135.0, -30.0];
const VOXEL: f64 = 0.1;
const SIDE: u8 = 8;
const TRUNCATION: f32 = 0.3;
const SPHERE: ([f64; 3], f64) = ([1.6, 1.6, 1.6], 1.1);
const SENSOR: [f64; 3] = [-2.5, 1.6, 1.6];
const UNKNOWN: [u8; 4] = [96, 40, 112, 255];

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

fn length(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn sphere_distance(p: [f64; 3]) -> f64 {
    length(sub(p, SPHERE.0)) - SPHERE.1
}

/// Where the sensor's ray to `p` first meets the sphere, if it does.
fn first_hit(p: [f64; 3]) -> Option<f64> {
    let d = sub(p, SENSOR);
    let range = length(d);
    let dir = d.map(|v| v / range);
    let oc = sub(SENSOR, SPHERE.0);
    let b = oc[0] * dir[0] + oc[1] * dir[1] + oc[2] * dir[2];
    let c = length(oc).powi(2) - SPHERE.1 * SPHERE.1;
    let disc = b * b - c;
    (disc >= 0.0).then(|| -b - disc.sqrt()).filter(|t| *t > 0.0)
}

/// TSDF sample at `p`: clamped distance and weight where the sensor saw it, unknown behind.
fn tsdf(p: [f64; 3]) -> Option<(f32, f32)> {
    let d = sub(p, SENSOR);
    let range = length(d);
    if d[0] / range < 50f64.to_radians().cos() {
        return None;
    }
    if first_hit(p).is_some_and(|hit| range > hit + f64::from(TRUNCATION)) {
        return None;
    }
    let value = sphere_distance(p).clamp(-f64::from(TRUNCATION), f64::from(TRUNCATION)) as f32;
    Some((value, (1.0 / (1.0 + 0.08 * range * range)) as f32))
}

fn box_distance(p: [f64; 3], centre: [f64; 3], half: [f64; 3]) -> f64 {
    let q = [0, 1, 2].map(|a| (p[a] - centre[a]).abs() - half[a]);
    let outside = length(q.map(|v| v.max(0.0)));
    outside + q[0].max(q[1]).max(q[2]).min(0.0)
}

/// Signed clearance to the obstacles: two boxes and a standing cylinder.
fn esdf(p: [f64; 3]) -> f64 {
    let a = box_distance(p, [1.6, 1.6, 0.6], [0.5, 0.5, 0.6]);
    let b = box_distance(p, [4.4, 2.0, 0.4], [0.3, 0.9, 0.4]);
    let radial = ((p[0] - 2.4).powi(2) + (p[1] - 4.6).powi(2)).sqrt() - 0.5;
    let vertical = (p[2] - 0.6).abs() - 0.6;
    let c = length([radial.max(0.0), vertical.max(0.0), 0.0]) + radial.max(vertical).min(0.0);
    a.min(b).min(c)
}

/// `p` moved by `by` metres in x and y: volumes are written centred on their bays.
fn moved(p: [f64; 3], by: f64) -> [f64; 3] {
    [p[0] + by, p[1] + by, p[2]]
}

/// Half the TSDF and ESDF volume widths: centred coordinates plus these are the fields' own.
const TSDF_HALF: f64 = 1.6;
const ESDF_HALF: f64 = 3.2;

fn unobserved(p: [f64; 3]) -> bool {
    (4.8..6.4).contains(&p[0]) && (4.0..5.6).contains(&p[1])
}

fn sample_block(field: &VolumeField, key: [i32; 3], sample: impl Fn([f64; 3]) -> Option<(f32, f32)>) -> VolumeBlock {
    let side = usize::from(field.block_side);
    let n = side.pow(3);
    let mut block = VolumeBlock { key, values: vec![0.0; n], valid: vec![false; n], weights: field.weights().then(|| vec![0.0; n]), colors: None };
    for i in 0..n {
        let local = [i % side, i / side % side, i / (side * side)];
        let p = [0, 1, 2].map(|a| ((i64::from(key[a]) * side as i64 + local[a] as i64) as f64 + 0.5) * field.voxel_m);
        if let Some((value, weight)) = sample(p) {
            block.valid[i] = true;
            block.values[i] = value;
            if let Some(weights) = &mut block.weights {
                weights[i] = weight;
            }
        }
    }
    block
}

/// Encoded blocks; with `observed_only`, blocks holding no valid sample are not sent.
fn pages(field: &VolumeField, blocks: &[VolumeBlock], observed_only: bool) -> Result<Vec<(PageDescriptor, Vec<u8>)>> {
    let mut out = Vec::new();
    for block in blocks.iter().filter(|b| !observed_only || b.valid.iter().any(|v| *v)) {
        let bytes = encode_volume_block(field, block)?;
        out.push((
            PageDescriptor {
                address: PageAddress { scan_key: 0, page_id: volume_page_id(block.key).ok_or("block key out of range")? },
                encoding: PageEncoding::VolumeBlockV1,
                items: field.samples() as u32,
                byte_len: bytes.len() as u64,
                crc32: page_crc32(&bytes),
                origin_m: [0.0; 3],
            },
            bytes,
        ));
    }
    Ok(out)
}

fn upload(writer: &mut DrawDataWriter<'_>, descriptor: &DataDescriptor, pages: &[(PageDescriptor, Vec<u8>)]) -> Result<DatasetRevision> {
    let borrowed: Vec<_> = pages.iter().map(|(d, b)| DataUploadPage { descriptor: d, bytes: b }).collect();
    let reply = writer.upload(&DataUpload::replace(descriptor, &[], &borrowed), deadline(), None)?;
    Ok(DatasetRevision { incarnation: reply.incarnation, generation: reply.stored_generation })
}

fn view(slice: VolumeSlice, contours: Vec<f64>, band: Option<[f64; 2]>) -> VolumeView {
    VolumeView {
        slices: vec![slice],
        interpolation: Interpolation::Trilinear,
        band,
        contours,
        contour_rgba: [255, 255, 255, 255],
        unknown_rgba: UNKNOWN,
        frame_rgba: Some(TINT),
    }
}

/// A volume standing on the highest ground within `radius` of `origin`.
fn volume(key: &str, field: &VolumeField, origin: [f64; 3], radius: f64, color: ColorMode, view: VolumeView) -> Result<DataDescriptor> {
    Ok(DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, key)?,
        shape: DataShape::Volume { field: field.clone(), frame: DataFrame { origin_m: origin, ground_radius_m: Some(radius), ..Default::default() } },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation { color, volume: Some(view), glow: 0.5, ..Default::default() },
    })
}

fn scalar(name: &str, colormap: u32, range: [f64; 2]) -> ColorMode {
    ColorMode::Scalar { name: name.into(), colormap, range, missing: UNKNOWN }
}

/// The caller's own isosurface of the sphere: a latitude-longitude shell.
fn isosurface() -> MeshBlock {
    let (rows, columns) = (24usize, 48usize);
    let mut block = MeshBlock { normals: Some(vec![]), ..MeshBlock::default() };
    for r in 0..=rows {
        let theta = std::f64::consts::PI * r as f64 / rows as f64;
        for c in 0..columns {
            let phi = std::f64::consts::TAU * c as f64 / columns as f64;
            let n = [theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos()];
            let centre = moved(SPHERE.0, -TSDF_HALF);
            block.positions.push([0, 1, 2].map(|a| (centre[a] + SPHERE.1 * n[a]) as f32));
            block.normals.as_mut().unwrap().push(n.map(|v| v as f32));
        }
    }
    for r in 0..rows {
        for c in 0..columns {
            let at = |r: usize, c: usize| (r * columns + c % columns) as u32;
            block.indices.extend([at(r, c), at(r + 1, c), at(r + 1, c + 1), at(r, c), at(r + 1, c + 1), at(r, c + 1)]);
        }
    }
    block
}

const TINT: [u8; 4] = [150, 120, 255, 255];

fn labels(client: &Client) -> Result<()> {
    let tint = rgba(TINT[0], TINT[1], TINT[2], TINT[3]);
    let ns = format!("{NS}/labels");
    let bay = |id, dx: f64, dy: f64, size, number: u32, title: &str, detail| lab::Bay {
        id,
        at: [AT[0] + dx, AT[1] + dy],
        size,
        title: format!("{number:02} / {title}"),
        detail,
        category: "SPARSE VOLUME",
        tint,
    };
    let tsdf = [4.2, 4.2, 3.4];
    let esdf = [7.4, 7.4, 2.0];
    lab::bays(
        client,
        &ns,
        &[
            bay("tsdf-sign", 0.0, 0.0, tsdf, 1, "TSDF SIGN", "RED INSIDE / GREEN OUTSIDE / WHITE AT THE ZERO CROSSING"),
            bay("tsdf-weight", 5.0, 0.0, tsdf, 2, "TSDF WEIGHT", "THE SHADOW BEHIND THE SURFACE IS UNKNOWN"),
            bay("tsdf-mesh", 10.0, 0.0, tsdf, 3, "SUPPLIED ISOSURFACE", "THE CALLER'S MESH, NOT RECONSTRUCTED"),
            bay("esdf-level", 1.6, 9.5, esdf, 4, "ESDF CLEARANCE", "LEVEL SLICE / RINGS AT 0.25 AND 0.5 M / CHECKER = UNKNOWN"),
            bay("esdf-oblique", 11.0, 9.5, esdf, 5, "ARBITRARY SLICE", "TILTED 30 DEG / BAND <= 0.6 M"),
        ],
    )?;
    lab::label(client, &ns, "title", [AT[0] + 5.5, AT[1] + 15.5, 3.8], "DISTANCE-FIELD LAB / SPARSE VOLUMES", -17.0, tint, Some("TYPED VOLUMES"))
}

fn probe(data: &mut gearbox_api::DrawDataClient, key: &str, points: Vec<[f64; 3]>) -> Result<serde_json::Value> {
    let mut request = DataRequest::new(data_op::FIELD_QUERY)
        .with_key(&DataKey::new(NS, key)?)?
        .with_metadata(&FieldQuery { points, channel: Channel::Value, interpolation: Interpolation::Trilinear, gradient: true, generation: None })?;
    request.host_epoch = data.capabilities().host_epoch;
    let reply = data.request_with_deadline(&request, deadline())?;
    if reply.code != data_code::OK {
        return Err(String::from_utf8_lossy(&reply.message).into_owned().into());
    }
    Ok(serde_json::from_slice(&reply.metadata)?)
}

fn oblique(angle: f64) -> VolumeSlice {
    let (s, c) = angle.to_radians().sin_cos();
    VolumeSlice { center_m: [0.0, 0.0, 0.8], u: [c, 0.0, s], v: [0.0, 1.0, 0.0], size_m: [3.2, 6.4], cell_m: 0.05 }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let instance = args.next().ok_or("usage: draw_distance_field_lab INSTANCE [sweep SECONDS]")?;
    let sweep = match (args.next().as_deref(), args.next()) {
        (Some("sweep"), Some(seconds)) => Some(seconds.parse::<f64>()?),
        (None, _) => None,
        _ => return Err("usage: draw_distance_field_lab INSTANCE [sweep SECONDS]".into()),
    };
    let entry = registry::find(&instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "distance-field-lab")?;
    let mut data = client.draw_data()?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(4, deadline())?;
    let mut clear = DataRequest { lease: writer.lease(), ..DataRequest::new(data_op::CLEAR) }.with_metadata(&NamespaceFilter { namespace: NS.into() })?;
    clear.host_epoch = data.capabilities().host_epoch;
    data.request_with_deadline(&clear, deadline())?;
    let _ = client.draw_clear(&format!("{NS}/labels"));
    labels(&client)?;

    let tsdf_field = VolumeField { kind: FieldKind::Tsdf { truncation_m: TRUNCATION }, voxel_m: VOXEL, block_side: SIDE, colors: false };
    let tsdf_blocks: Vec<_> = (-2..2)
        .flat_map(|x| (-2..2).flat_map(move |y| (0..4).map(move |z| [x, y, z])))
        .map(|k| sample_block(&tsdf_field, k, |p| tsdf(moved(p, TSDF_HALF))))
        .collect();
    let tsdf_pages = pages(&tsdf_field, &tsdf_blocks, true)?;
    let across = VolumeSlice::across(1, [0.0, 0.0, 1.6], [3.2, 3.2], 0.025);
    let origin = |dx: f64, dy: f64| [AT[0] + dx, AT[1] + dy, 0.0];
    let sign = volume("tsdf-sign", &tsdf_field, origin(0.0, 0.0), 2.3, scalar("value", colormap::RDYLGN, [-0.3, 0.3]), view(across.clone(), vec![0.0], None))?;
    let weight = volume("tsdf-weight", &tsdf_field, origin(5.0, 0.0), 2.3, scalar("weight", colormap::VIRIDIS, [0.0, 0.6]), view(across, vec![], None))?;
    let mut revisions = vec![("tsdf-sign", upload(&mut writer, &sign, &tsdf_pages)?), ("tsdf-weight", upload(&mut writer, &weight, &tsdf_pages)?)];
    let shell = isosurface();
    let bytes = encode_mesh_block(&shell)?;
    let mesh = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, "tsdf-mesh")?,
        shape: DataShape::MeshBlocks { frame: DataFrame { origin_m: origin(10.0, 0.0), ground_radius_m: Some(2.3), ..Default::default() } },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation { color: ColorMode::Constant { rgba: [140, 190, 235, 255] }, glow: 0.6, opacity: 0.75, ..Default::default() },
    };
    let page = PageDescriptor {
        address: PageAddress { scan_key: 0, page_id: 0 },
        encoding: PageEncoding::MeshBlockV1,
        items: shell.triangles() as u32,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    revisions.push(("tsdf-mesh", upload(&mut writer, &mesh, &[(page, bytes)])?));

    let esdf_field = VolumeField { kind: FieldKind::Esdf { signed: true }, voxel_m: VOXEL, block_side: SIDE, colors: false };
    let esdf_sample = |p: [f64; 3]| {
        let q = moved(p, ESDF_HALF);
        (!unobserved(q)).then(|| (esdf(q) as f32, 0.0))
    };
    let esdf_blocks: Vec<_> = (-4..4)
        .flat_map(|x| (-4..4).flat_map(move |y| (0..2).map(move |z| [x, y, z])))
        .filter(|k| !(k[0] == 3 && k[1] == 3))
        .map(|k| sample_block(&esdf_field, k, esdf_sample))
        .collect();
    let esdf_pages = pages(&esdf_field, &esdf_blocks, false)?;
    let level = volume(
        "esdf-level",
        &esdf_field,
        origin(1.6, 9.5),
        4.6,
        scalar("value", colormap::TURBO, [-0.2, 1.5]),
        view(VolumeSlice::across(2, [0.0, 0.0, 0.5], [6.4, 6.4], 0.05), vec![0.25, 0.5], None),
    )?;
    let tilted = volume(
        "esdf-oblique",
        &esdf_field,
        origin(11.0, 9.5),
        4.6,
        scalar("value", colormap::RDYLGN, [-0.3, 0.6]),
        view(oblique(30.0), vec![0.0], Some([-1.0, 0.6])),
    )?;
    revisions.push(("esdf-level", upload(&mut writer, &level, &esdf_pages)?));
    revisions.push(("esdf-oblique", upload(&mut writer, &tilted, &esdf_pages)?));

    let probes = serde_json::json!({
        "tsdf_surface": probe(&mut data, "tsdf-sign", vec![moved([SPHERE.0[0] - SPHERE.1, 1.6, 1.6], -TSDF_HALF)])?,
        "tsdf_just_inside": probe(&mut data, "tsdf-sign", vec![moved([SPHERE.0[0] - SPHERE.1 + 0.15, 1.6, 1.6], -TSDF_HALF)])?,
        "tsdf_free": probe(&mut data, "tsdf-sign", vec![moved([SPHERE.0[0] - SPHERE.1 - 0.35, 1.6, 1.6], -TSDF_HALF)])?,
        "tsdf_centre_unseen": probe(&mut data, "tsdf-sign", vec![moved(SPHERE.0, -TSDF_HALF)])?,
        "tsdf_shadow": probe(&mut data, "tsdf-sign", vec![moved([SPHERE.0[0] + SPHERE.1 + 0.4, 1.6, 1.6], -TSDF_HALF)])?,
        "esdf_clear_of_box": probe(&mut data, "esdf-level", vec![moved([2.7, 1.6, 0.5], -ESDF_HALF)])?,
        "esdf_unobserved": probe(&mut data, "esdf-level", vec![moved([5.5, 4.8, 0.5], -ESDF_HALF)])?,
        "esdf_never_sent": probe(&mut data, "esdf-level", vec![moved([6.0, 6.0, 0.5], -ESDF_HALF)])?,
    });
    if let Some(seconds) = sweep {
        let start = Instant::now();
        let mut revision = revisions.iter().find(|(k, _)| *k == "esdf-oblique").unwrap().1;
        while start.elapsed().as_secs_f64() < seconds {
            let angle = 30.0 * (start.elapsed().as_secs_f64() * 0.8).cos();
            let mut presentation = tilted.presentation.clone();
            presentation.volume.as_mut().unwrap().slices = vec![oblique(angle)];
            let mut patch = DataRequest {
                lease: writer.lease(),
                incarnation: revision.incarnation,
                base_generation: revision.generation,
                ..DataRequest::new(data_op::PATCH_PRESENTATION)
            }
            .with_key(&tilted.key)?
            .with_metadata(&presentation)?;
            patch.host_epoch = data.capabilities().host_epoch;
            let reply = data.request_with_deadline(&patch, deadline())?;
            if reply.code != data_code::OK {
                return Err(String::from_utf8_lossy(&reply.message).into_owned().into());
            }
            revision.generation = reply.stored_generation;
            std::thread::sleep(Duration::from_millis(120));
        }
    }
    writer.close(deadline())?;
    let revisions: serde_json::Map<String, serde_json::Value> = revisions
        .iter()
        .map(|(key, r)| (key.to_string(), serde_json::json!({ "incarnation": r.incarnation, "generation": r.generation })))
        .collect();
    println!(
        "{}",
        serde_json::json!({
            "scene": "distance-field-lab",
            "namespace": NS,
            "blocks": { "tsdf": tsdf_pages.len(), "esdf": esdf_pages.len() },
            "revisions": revisions,
            "probes": probes,
        })
    );
    Ok(())
}
