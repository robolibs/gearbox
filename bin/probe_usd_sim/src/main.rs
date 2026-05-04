//! `probe-usd-sim <path/to/file.usd>` — minimal end-to-end test of
//! the `gearbox-usd` bridge.
//!
//! 1. Build a default gearbox `Sim` (gravity, no ground for now).
//! 2. Call `gearbox_usd::load_usd_into_sim` to spawn every USD rigid
//!    body into `Sim.bodies`.
//! 3. Step the sim 100 ticks at 60 Hz.
//! 4. Print the start vs end position of each body. Anything dynamic
//!    should fall (Y decreasing under default gravity).
//!
//! No colliders, no joints yet — this proves the bridge plumbing,
//! not full physics behaviour.

use std::path::PathBuf;

use anyhow::Result;
use gearbox_physics::Sim;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: probe-usd-sim <path/to/file.usd>");

    let mut sim = Sim::new();
    let descriptor = gearbox_usd::load_usd_into_sim(&path, &mut sim)?;

    println!("loaded {} rigid body prim(s) from {}", descriptor.bodies.len(), path.display());
    println!("gravity: {:?}", sim.gravity);

    // Snapshot start positions.
    let mut entries: Vec<(String, [f64; 3])> = descriptor
        .bodies
        .iter()
        .map(|(p, h)| {
            let t = sim.bodies[*h].translation();
            (p.clone(), [t.x, t.y, t.z])
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let dt = 1.0 / 60.0;
    let ticks = 100;
    for _ in 0..ticks {
        sim.step(dt);
    }

    println!("\nafter {} ticks @ {} Hz ({}s simulated):", ticks, (1.0 / dt) as i32, dt * ticks as f64);
    println!(
        "{:<60} {:>9} {:>9} {:>9}   {:>9} {:>9} {:>9}",
        "prim_path", "start_x", "start_y", "start_z", "end_x", "end_y", "end_z"
    );
    for (path, start) in &entries {
        let h = descriptor.bodies[path];
        let t = sim.bodies[h].translation();
        println!(
            "{:<60} {:>9.3} {:>9.3} {:>9.3}   {:>9.3} {:>9.3} {:>9.3}",
            path, start[0], start[1], start[2], t.x, t.y, t.z
        );
    }

    Ok(())
}
