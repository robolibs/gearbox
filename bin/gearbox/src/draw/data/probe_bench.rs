//! `GEARBOX_DRAW_BENCH=1` with the resource probe appends one line per frame to
//! `<probe dir>/frames.jsonl`: wall time, CPU frame time, LOD selection, uploads,
//! owned allocations, process RSS and the main view's MSAA.

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use mara::ui::modules::bevy::ChaseCamera;

use super::{Driver, ProbeState};

#[derive(Resource)]
pub(super) struct Bench {
    out: std::io::BufWriter<std::fs::File>,
    frame: u64,
}

pub(super) fn install(app: &mut App, root: &std::path::Path) {
    if std::env::var_os("GEARBOX_DRAW_BENCH").is_none() {
        return;
    }
    let file = std::fs::File::create(root.join("frames.jsonl")).expect("bench frame log");
    app.insert_resource(Bench { out: std::io::BufWriter::new(file), frame: 0 })
        .add_systems(Update, msaa)
        .add_systems(PostUpdate, record.after(super::drive));
}

/// `GEARBOX_MSAA` (1, 2, 4 or 8) holds the main view's samples for a profile,
/// whatever the viewer sets.
fn msaa(mut commands: Commands, mut cameras: Query<(Entity, Option<&mut Msaa>), With<ChaseCamera>>) {
    let Some(samples) = std::env::var("GEARBOX_MSAA").ok().and_then(|v| v.parse::<u32>().ok()) else {
        return;
    };
    for (camera, current) in &mut cameras {
        match current {
            Some(mut current) if current.samples() != samples => *current = Msaa::from_samples(samples),
            Some(_) => {}
            None => {
                commands.entity(camera).insert(Msaa::from_samples(samples));
            }
        }
    }
}

/// Resident set size from `/proc/self/statm`, when the platform has it.
fn rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

fn record(
    mut bench: ResMut<Bench>,
    probe: Res<ProbeState>,
    time: Res<Time<Real>>,
    store: Res<gearbox_api::DrawStoreRes>,
    octrees: Query<&super::super::octree::Node>,
    camera: Query<&Msaa, With<ChaseCamera>>,
    _driver: Res<Driver>,
) {
    let report = *probe.0.report.lock().unwrap_or_else(|p| p.into_inner());
    let backend = store.backend();
    let unix_s = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    let visible_cells: u64 = octrees.iter().filter(|n| n.visible).map(|n| n.page.cells.records.values.len() as u64).sum();
    let line = serde_json::json!({
        "frame": bench.frame,
        "unix_s": unix_s,
        "cpu_ms": time.delta_secs_f64() * 1e3,
        "gpu_ms": null,
        "gpu_ms_reason": "GPU timestamp queries are not enabled",
        "lod": report.lod[0],
        "uploads": report.uploads,
        "allocations": report.allocations,
        "owned_cpu_bytes": backend.budget().usage().committed(),
        "owned_gpu_bytes": backend.gpu().usage().bytes,
        "gpu_uploaded_bytes": backend.gpu().usage().uploaded,
        "octree_visible_cells": visible_cells,
        "rss_bytes": rss_bytes(),
        "msaa": camera.iter().next().map(|m| m.samples()),
    });
    let frame = bench.frame;
    let _ = writeln!(bench.out, "{line}");
    if frame % 30 == 0 {
        let _ = bench.out.flush();
    }
    bench.frame += 1;
}
