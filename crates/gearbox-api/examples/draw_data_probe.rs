use gearbox_api::{
    Client, DataPipelineConfig, DataUpload, DataUploadPage, IdentitySource, registry,
};
use std::time::{Duration, Instant};

#[path = "../tests/support/draw_data_native.rs"]
mod native;

#[allow(dead_code)]
#[path = "../tests/support/draw_data_upload.rs"]
mod upload;

fn writer_probe(client: &Client) -> upload::Result<serde_json::Value> {
    let deadline = || Instant::now() + Duration::from_secs(15);
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let namespace = format!("draw-writer-probe/{}", std::process::id());
    let fixture = upload::UploadFixture::cloud(&namespace, "cloud", 100_000)?;
    let pages: Vec<_> = fixture
        .pages
        .iter()
        .map(|(descriptor, bytes)| DataUploadPage { descriptor, bytes })
        .collect();
    let scans = [fixture.scan.clone()];
    let mut source = DataUpload::replace(&fixture.descriptor, &scans, &pages);
    let mut writer = pipeline.writer(8, deadline())?;
    let mut verify = upload::Connection::Pipeline {
        client: &pipeline,
        window: 8,
    };
    let result = (|| -> upload::Result<_> {
        let first = writer.upload(&source, deadline(), None)?;
        let revision = gearbox_api::wire::draw_data::DatasetRevision {
            incarnation: first.incarnation,
            generation: first.stored_generation,
        };
        verify.verify(&fixture, revision)?;
        source.base = Some(revision);
        source.mode = gearbox_api::wire::draw_data::CommitMode::Delta;
        source.scans = &[];
        source.pages = &pages[..1];
        let second = writer.upload(&source, deadline(), None)?;
        let next = gearbox_api::wire::draw_data::DatasetRevision {
            incarnation: second.incarnation,
            generation: second.stored_generation,
        };
        verify.verify(&fixture, next)?;
        if next.incarnation != revision.incarnation || next.generation != revision.generation + 1 {
            return Err("writer delta revision mismatch".into());
        }
        Ok(next)
    })();
    let clear = verify.clear(&namespace, writer.lease());
    let close = writer.close(deadline());
    let revision = result?;
    clear?;
    close?;
    drop(writer);
    Ok(
        serde_json::json!({"points":100000,"revision":revision,"rendered":false,"pipeline":pipeline.stats(),"result":"public writer replace/delta and cleanup passed"}),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let instance = std::env::args().nth(1).ok_or("provide an instance name")?;
    let entry = registry::find(&instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "draw-data-probe")?;
    let reset = std::env::args().nth(2).as_deref() == Some("--reset-owned-scene");
    let report = native::verify(&client, reset)?;
    let writer = writer_probe(&client)?;
    println!(
        "{}",
        serde_json::json!({"instance":instance,"pid":entry.pid,"verification":report,"writer":writer})
    );
    Ok(())
}
