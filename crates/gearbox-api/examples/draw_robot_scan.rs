//! Robot scan: a driven rover's native LiDAR sweeps grouped into organized typed
//! scans, placed by the pose each was captured at and accumulated within bounds,
//! beside an occupancy map the caller supplies (it is not built from the cloud).
//!
//! Usage: `draw_robot_scan INSTANCE GEARBOX_BIN [SECONDS]`.


use std::process::Command;
use std::time::{Duration, Instant};

use gearbox_api::wire::draw_data::lidar::{Completion, LidarAssembler, LidarLimits, LidarSweep};
use gearbox_api::wire::{draw::rgba, draw_data::*, draw_widgets::Place};
use gearbox_api::{
    Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawDataWriter, DrawEntity as E,
    IdentitySource, LidarScan, clock_op, colormap, draw_style, next_sample, registry, topics,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-gallery/plan/robot-scan";
const ROVER: &str = "scan_rover";
const LINK: &str = "lidar";
/// Scene centre, east and north of home in metres.
const AT: [f64; 2] = [-76.0, 0.0];
const MAP_AT: [f64; 3] = [-104.0, -4.0, 0.2];
const MAX_SCANS: u32 = 50;
/// Returns nearer than this come off the robot's own body.
const SELF_RANGE_M: f32 = 2.5;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

fn cli(bin: &str, instance: &str, args: &[String]) -> Result<()> {
    let output = Command::new("timeout")
        .args(["120", bin, "--instance", instance, "--timeout", "30"])
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!("CLI {args:?}: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(())
}

/// The rover on its circle and three parked machines for it to scan.
fn spawn(client: &Client, bin: &str, instance: &str) -> Result<()> {
    let listed: Vec<String> = client.machines()?.iter().map(|m| m.machine_id()).collect();
    let usd = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/machines/usd");
    for (file, id, east, north, yaw) in [
        ("regolith_excavator", ROVER, AT[0] + 6.7, AT[1], 0.0),
        ("fendt_tractor", "scan_tractor", AT[0], AT[1] + 12.5, 90.0),
        ("claas_trailer", "scan_trailer", AT[0], AT[1] - 12.5, 90.0),
        ("clearpath_husky", "scan_husky", AT[0], AT[1], 30.0),
    ] {
        if listed.iter().any(|m| m == id) {
            continue;
        }
        let args = [
            "spawn", "machine", &format!("{usd}/{file}.usdz"), "--machine", id,
            "--at", &north.to_string(), &east.to_string(),
            "--yaw", &yaw.to_string(), "--wait-timeout", "120",
        ];
        cli(bin, instance, &args.map(String::from))?;
    }
    Ok(())
}

fn sweeps_descriptor() -> Result<DataDescriptor> {
    Ok(DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, "sweeps")?,
        shape: DataShape::Cloud { schema: LidarSweep::schema() },
        stream: StreamMode::Accumulate {
            max_scans: MAX_SCANS,
            max_age_s: 1e12,
            max_bytes: 64 << 20,
            age_clock: AgeClock::Simulation,
        },
        lifetime_s: None,
        presentation: Presentation {
            color: ColorMode::Scalar {
                name: "range".into(),
                colormap: colormap::TURBO,
                range: [1.0, 28.0],
                missing: [0; 4],
            },
            size: PointSize::Metres { radius: 0.04 },
            glow: 0.2,
            ..Default::default()
        },
    })
}

fn latest_descriptor() -> Result<DataDescriptor> {
    Ok(DataDescriptor {
        key: DataKey::new(NS, "latest")?,
        stream: StreamMode::Latest,
        presentation: Presentation {
            color: ColorMode::Constant { rgba: [120, 235, 255, 255] },
            size: PointSize::Metres { radius: 0.03 },
            glow: 0.45,
            ..Default::default()
        },
        ..sweeps_descriptor()?
    })
}

fn pages(sweep: &LidarSweep, scan_key: u64) -> Result<Vec<(PageDescriptor, Vec<u8>)>> {
    let schema = LidarSweep::schema();
    sweep
        .pages()
        .iter()
        .enumerate()
        .map(|(id, page)| {
            let bytes = page.encode(&schema)?;
            Ok((
                PageDescriptor {
                    address: PageAddress { scan_key, page_id: id as u64 },
                    encoding: PageEncoding::CloudColumnsV1,
                    items: page.count,
                    byte_len: bytes.len() as u64,
                    crc32: page_crc32(&bytes),
                    origin_m: [0.0; 3],
                },
                bytes,
            ))
        })
        .collect()
}

fn upload(
    writer: &mut DrawDataWriter<'_>,
    descriptor: &DataDescriptor,
    scan: ScanDescriptor,
    pages: &[(PageDescriptor, Vec<u8>)],
    mode: CommitMode,
    base: Option<DatasetRevision>,
) -> Result<DatasetRevision> {
    let borrowed: Vec<_> = pages.iter().map(|(d, b)| DataUploadPage { descriptor: d, bytes: b }).collect();
    let scans = [scan];
    let mut upload = DataUpload::replace(descriptor, &scans, &borrowed);
    upload.base = base;
    if base.is_some() {
        upload.mode = mode;
    }
    let reply = writer.upload(&upload, deadline(), None)?;
    Ok(DatasetRevision { incarnation: reply.incarnation, generation: reply.stored_generation })
}

/// The supplied map: a walled pen with a doorway, its observed-free floor and a
/// few uncertain cells, in 0.8 m wall cells and 1.6 m free cells.
fn supplied_map() -> Result<(DataDescriptor, Vec<(PageDescriptor, Vec<u8>)>)> {
    let config = OctreeConfig { leaf_resolution_m: 0.1, max_depth: 8, free_max: 0.3, occupied_min: 0.7 };
    let leaf = |depth: u8, cell: [i128; 3], probability| -> Result<OccupancyLeaf> {
        let finest = cell.map(|c| c << (config.max_depth - depth));
        Ok(OccupancyLeaf {
            key: VoxelKey::from_grid_cell(finest, depth, &config)?,
            value: OccupancyValue { probability, confidence: None, class_id: None },
        })
    };
    let mut leaves = Vec::new();
    for z in 0..3 {
        for x in 0..10 {
            for y in [0, 9] {
                if !(y == 0 && (4..6).contains(&x) && z < 2) {
                    leaves.push(leaf(5, [x, y, z], Some(0.94))?);
                }
            }
        }
        for y in 1..9 {
            for x in [0, 9] {
                leaves.push(leaf(5, [x, y, z], Some(0.94))?);
            }
        }
    }
    for x in 1..4 {
        for y in 1..4 {
            leaves.push(leaf(4, [x, y, 0], Some(0.06))?);
        }
    }
    for (x, y) in [(1, 5), (8, 3), (5, 8)] {
        leaves.push(leaf(5, [x, y, 0], Some(0.5))?);
    }
    let bytes = encode_occupancy_page(&config, &leaves)?;
    let page = PageDescriptor {
        address: PageAddress { scan_key: 0, page_id: 0 },
        encoding: PageEncoding::OccupancyLeavesV1,
        items: leaves.len() as u32,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: DataKey::new(NS, "supplied-map")?,
        shape: DataShape::Octree {
            config,
            frame: DataFrame { origin_m: MAP_AT, ..Default::default() },
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation {
            octree: Some(OctreeDisplay { show_free: true, ..OctreeDisplay::default() }),
            ..Presentation::for_octree()
        },
    };
    Ok((descriptor, vec![(page, bytes)]))
}

fn labels(client: &Client) -> Result<()> {
    let place = Place::default().no_drape();
    let ink = rgba(225, 235, 242, 255);
    let label = |id: &str, at: [f64; 3], text: &str, size: f32, tint: u32, category: Option<&str>| {
        let mut entity = place
            .put(E::text(&format!("{NS}/labels"), id, &[at.map(|v| v as f32)], text))
            .with_style(draw_style::LABEL)
            .with_width(size)
            .with_color(ink)
            .with_prop("accent", &tint.to_string());
        if let Some(category) = category {
            entity = entity.with_prop("label_category", category);
        }
        entity
    };
    let scan = rgba(80, 200, 255, 255);
    let map = rgba(230, 150, 60, 255);
    for entity in [
        label("scan-title", [AT[0], AT[1] + 3.0, 6.5], "ROBOT SCAN / CAPTURE-TIME LIDAR", -17.0, scan, Some("TYPED CLOUD")),
        label(
            "scan-detail",
            [AT[0], AT[1] - 3.0, 5.0],
            "RANGE COLOUR: LAST 50 SWEEPS AT THEIR CAPTURE POSE / CYAN: LATEST SWEEP ON ITS LIVE LINK / RIGID SWEEPS, NO DESKEW / OWN-BODY RETURNS UNDER 2.5 M INVALID",
            -11.0,
            scan,
            None,
        ),
        label("map-title", [MAP_AT[0] + 4.0, MAP_AT[1] + 4.0, 4.6], "SUPPLIED OCCUPANCY MAP", -17.0, map, Some("TYPED OCTREE")),
        label(
            "map-detail",
            [MAP_AT[0] + 4.0, MAP_AT[1] - 1.0, 0.6],
            "PROVIDED BY THE CALLER / NOT BUILT FROM THIS CLOUD",
            -11.0,
            map,
            None,
        ),
    ] {
        let status = client.draw(&entity)?;
        if !status.is_ok() {
            return Err(status.message().into());
        }
    }
    Ok(())
}

/// Hits on the parked husky in each sweep: their centroid placed by each sweep's
/// own capture pose, and for contrast by the latest pose as a current-pose
/// placement of an accumulated scan would put them.
#[derive(Default)]
struct Stationary {
    sweeps: Vec<(Vec<[f64; 3]>, [f64; 3])>,
    latest: Option<RigidPose>,
}

fn centroid(points: impl Iterator<Item = [f64; 3]>) -> Option<[f64; 3]> {
    let (mut sum, mut n) = ([0.0; 3], 0.0);
    for p in points {
        (0..3).for_each(|k| sum[k] += p[k]);
        n += 1.0;
    }
    (n > 0.0).then(|| sum.map(|s| s / n))
}

/// Largest horizontal distance of a centroid from their mean.
fn spread(centroids: &[[f64; 3]]) -> f64 {
    let Some(mean) = centroid(centroids.iter().copied()) else { return 0.0 };
    centroids.iter().map(|c| (c[0] - mean[0]).hypot(c[1] - mean[1])).fold(0.0, f64::max)
}

impl Stationary {
    fn observe(&mut self, sweep: &LidarSweep) {
        let Some(capture) = &sweep.capture else { return };
        let pose = &capture.transform.pose;
        let around: Vec<([f64; 3], [f64; 3])> = (0..sweep.pulses())
            .filter(|&i| sweep.hit(i))
            .filter_map(|i| {
                let local = sweep.points[i].map(f64::from);
                Some((local, pose.transform_point(local).ok()?))
            })
            .filter(|(_, w)| (w[0] - AT[0]).abs() < 1.5 && (w[1] - AT[1]).abs() < 1.5)
            .collect();
        let Some(ground) = around.iter().map(|(_, w)| w[2]).min_by(f64::total_cmp) else { return };
        let (local, world): (Vec<_>, Vec<_>) = around.into_iter().filter(|(_, w)| w[2] > ground + 0.15).unzip();
        if local.len() >= 5 {
            let at = centroid(world.into_iter()).unwrap();
            self.sweeps.push((local, at));
        }
        self.latest = Some(pose.clone());
    }

    /// Spread of the husky's centroid at capture poses and at the latest pose.
    fn spreads(&self) -> (f64, f64, usize) {
        let captured: Vec<_> = self.sweeps.iter().map(|(_, c)| *c).collect();
        let current: Vec<_> = self
            .latest
            .as_ref()
            .map(|pose| {
                self.sweeps
                    .iter()
                    .filter_map(|(local, _)| centroid(local.iter().filter_map(|p| pose.transform_point(*p).ok())))
                    .collect()
            })
            .unwrap_or_default();
        (spread(&captured), spread(&current), captured.len())
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [instance, bin, rest @ ..] = args.as_slice() else {
        return Err("usage: draw_robot_scan INSTANCE GEARBOX_BIN [SECONDS]".into());
    };
    let seconds: f64 = rest.first().map_or(Ok(30.0), |s| s.parse())?;
    let entry = registry::find(instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "robot-scan")?;
    spawn(&client, bin, instance)?;
    let mut sensor = client.subscribe_env(&topics::machine_sensor_topic(ROVER, LINK))?;
    let mut rover = client.machine(ROVER);
    let session = rover.claim(3_000, true)?.session;
    let mut data = client.draw_data()?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(4, deadline())?;
    let mut clear = DataRequest { lease: writer.lease(), ..DataRequest::new(data_op::CLEAR) }
        .with_metadata(&NamespaceFilter { namespace: NS.into() })?;
    clear.host_epoch = data.capabilities().host_epoch;
    data.request_with_deadline(&clear, deadline())?;
    labels(&client)?;
    let (map, map_pages) = supplied_map()?;
    let borrowed: Vec<_> = map_pages.iter().map(|(d, b)| DataUploadPage { descriptor: d, bytes: b }).collect();
    writer.upload(&DataUpload::replace(&map, &[], &borrowed), deadline(), None)?;

    let sweeps = sweeps_descriptor()?;
    let latest = latest_descriptor()?;
    let idle_limit = Duration::from_millis(data.capabilities().limits.lease_timeout_ms / 2);
    let mut last_upload = Instant::now();
    client.clock(clock_op::PLAY)?;
    let mut assembler = LidarAssembler::new(LidarLimits::default(), Completion::CompleteOnly);
    let mut stationary = Stationary::default();
    let (mut base, mut latest_base, mut scan_key, mut points, mut upload_s) = (None, None, 0u64, 0u64, 0.0f64);
    let debug = std::env::var_os("GEARBOX_SCAN_DEBUG").is_some();
    let started = Instant::now();
    let mut last_drive = Instant::now() - Duration::from_secs(1);
    while started.elapsed().as_secs_f64() < seconds {
        if last_drive.elapsed() > Duration::from_millis(100) {
            let status = rover.cmd_vel(session, 0.6, 0.45)?;
            if debug && !status.is_ok() {
                eprintln!("cmd_vel refused: {}", status.message());
            }
            last_drive = Instant::now();
        }
        let Some(chunk) = next_sample::<LidarScan>(&mut sensor, Duration::from_millis(50))? else {
            continue;
        };
        if debug {
            eprintln!(
                "{:.2}s chunk sample={} t={:.3} offset={} count={}",
                started.elapsed().as_secs_f64(),
                chunk.sample,
                chunk.sim_time_s,
                chunk.pulse_offset,
                chunk.pulse_count
            );
        }
        for mut sweep in assembler.push(ROVER, &chunk)? {
            sweep.drop_near(SELF_RANGE_M);
            scan_key += 1;
            if debug && scan_key % 10 == 0 {
                let pose = sweep.capture.as_ref().map(|c| c.transform.pose.clone()).unwrap_or_default();
                let near = (0..sweep.pulses()).filter(|&i| sweep.hit(i) && sweep.ranges[i] < 3.0).count();
                let far = (0..sweep.pulses()).filter(|&i| sweep.hit(i) && sweep.ranges[i] >= 3.0).count();
                eprintln!("sweep {scan_key} t={:.2} at {:?} q {:?} near {near} far {far}", sweep.sim_time_s, pose.translation_m, pose.rotation_wxyz);
            }
            stationary.observe(&sweep);
            if last_upload.elapsed() > idle_limit {
                writer.close(deadline())?;
                writer = pipeline.writer(4, deadline())?;
            }
            let begun = Instant::now();
            let pages = pages(&sweep, scan_key)?;
            let snapshot = sweep.scan_descriptor(scan_key, Placement::Snapshot)?;
            base = Some(upload(&mut writer, &sweeps, snapshot, &pages, CommitMode::Delta, base)?);
            let live = sweep.scan_descriptor(scan_key, Placement::Live)?;
            latest_base = Some(upload(&mut writer, &latest, live, &pages, CommitMode::Replace, latest_base)?);
            upload_s += begun.elapsed().as_secs_f64();
            last_upload = Instant::now();
            points += (0..sweep.pulses()).filter(|&i| sweep.hit(i)).count() as u64;
        }
    }
    rover.cmd_vel(session, 0.0, 0.0)?;
    rover.release(session)?;
    writer.close(deadline())?;
    let (captured, current, seen) = stationary.spreads();
    let report = serde_json::json!({
        "sweeps": scan_key,
        "abandoned": assembler.abandoned,
        "hits": points,
        "upload_ms_per_sweep": 1e3 * upload_s / scan_key.max(1) as f64,
        "sweeps_seeing_parked_husky": seen,
        "husky_centroid_spread_m_at_capture_pose": captured,
        "husky_centroid_spread_m_at_latest_pose": current,
    });
    println!("{report}");
    if scan_key < 3 || seen < 3 {
        return Err("too few complete sweeps saw the parked husky".into());
    }
    if captured > 1.0 || captured * 4.0 > current {
        return Err("the parked husky did not stay put at its capture poses".into());
    }
    Ok(())
}
