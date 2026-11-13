use std::time::{Duration, Instant};

use gearbox_api::{Client, DataPipelineConfig, IdentitySource, registry};
use serde_json::json;

#[path = "../tests/support/draw_data_upload.rs"]
mod upload;

fn main() -> upload::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let target = args.first().map(String::as_str).unwrap_or("fake");
    let points = args
        .get(1)
        .map(|n| n.parse::<usize>())
        .transpose()?
        .unwrap_or(100_000);
    let repeats = args
        .get(2)
        .map(|n| n.parse::<usize>())
        .transpose()?
        .unwrap_or(3);
    if !(1..=20).contains(&repeats) {
        return Err("repetitions must be 1..20".into());
    }
    let fake = (target == "fake")
        .then(|| gearbox_api::fake::FakeHost::start("capacity-host", &[]))
        .transpose()?;
    let client = if let Some(fake) = &fake {
        Client::connect_id(fake.endpoint_id, IdentitySource::Random, "capacity-client")?
    } else {
        let entry = registry::find(target).ok_or("instance is not running")?;
        Client::connect(&entry.did, IdentitySource::Random, "capacity-client")?
    };
    let namespace = format!("draw-data-capacity/{}", std::process::id());
    let fixture = upload::UploadFixture::cloud(&namespace, "cloud", points)?;
    let mut direct = client.draw_data()?;
    let pipeline = client.draw_data_pipeline(
        DataPipelineConfig::default(),
        Instant::now() + Duration::from_secs(10),
    )?;
    let mut results = Vec::new();
    for window in [0, 1, 4, 8] {
        let mut connection = if window == 0 {
            upload::Connection::Direct(&mut direct)
        } else {
            upload::Connection::Pipeline {
                client: &pipeline,
                window,
            }
        };
        let lease = connection.open()?;
        let result = (|| -> upload::Result<_> {
            let mut base = None;
            let mut timings = Vec::new();
            for transaction in 1..=(repeats + 1) {
                let start = Instant::now();
                let revision = connection.upload(&fixture, lease, transaction as u64, base)?;
                let elapsed = start.elapsed().as_secs_f64();
                connection.verify(&fixture, revision)?;
                base = Some(revision);
                if transaction > 1 {
                    timings.push(elapsed);
                }
            }
            Ok(timings)
        })();
        let clear = connection.clear(&namespace, lease);
        let close = connection.close_writer(lease);
        let mut times = result?;
        clear?;
        close?;
        times.sort_by(f64::total_cmp);
        let median = times[times.len() / 2];
        results.push(json!({"window":window,"workers":if window==0 {1} else {4},"seconds":times,"median_seconds":median,"payload_mib_per_s":fixture.bytes as f64 / (1024.0*1024.0*median),"retained_points_per_s":points as f64 / median,"pipeline":pipeline.stats()}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"target":target,"service":if fake.is_some() {"FakeHost 2ms poll"} else {"native drawing service"},"points":points,"pages":fixture.pages.len(),"input_payload_bytes":fixture.bytes,"repetitions":repeats,"warmup_uploads_per_mode":1,"rendered":false,"scope":"local SHM retained uploads; not renderer, cross-machine, RSS, or sensor-rate acceptance","results":results})
        )?
    );
    Ok(())
}
