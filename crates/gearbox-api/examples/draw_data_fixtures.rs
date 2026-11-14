#[path = "../tests/support/draw_data_fixtures.rs"]
mod fixtures;

use std::path::PathBuf;

use fixtures::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(
        args.next()
            .ok_or("usage: draw_data_fixtures OUTPUT [COUNT]")?,
    );
    let count: usize = args.next().map(|s| s.parse()).transpose()?.unwrap_or(1024);
    if args.next().is_some() {
        return Err("unexpected fixture argument".into());
    }
    if count > MAX_FIXTURE_POINTS {
        return Err(format!("fixture point limit is {MAX_FIXTURE_POINTS}").into());
    }
    std::fs::create_dir_all(&out)?;
    for shape in CLOUD_SHAPES {
        let samples = cloud(shape, count, FIXTURE_SEED)?;
        let manifest = cloud_manifest(shape, FIXTURE_SEED, &samples);
        let name = serde_json::to_value(shape)?
            .as_str()
            .ok_or("invalid fixture name")?
            .to_owned();
        std::fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
    }
    std::fs::write(
        out.join("occupancy.json"),
        serde_json::to_vec_pretty(&occupancy())?,
    )?;
    println!("wrote 6 fixture manifests to {}", out.display());
    Ok(())
}
