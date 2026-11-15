use std::time::{Duration, Instant};

use gearbox_api::wire::{draw::rgba, draw_data::*, draw_widgets::Place};
use gearbox_api::{
    Client, DataPipelineConfig, DataUpload, DataUploadPage, DrawDataClient, DrawDataWriter,
    DrawEntity as E, IdentitySource, Instance, draw_flag, draw_style, registry, shape_kind,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NS: &str = "vfx-gallery/cloud-scale";
const OBSERVER: &str = "draw-scale-observer";
#[path = "draw_cloud_scale/fixture.rs"]
mod fixture;
#[path = "draw_cloud_scale/proof.rs"]
mod proof;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(120)
}

fn require(value: bool, reason: &str) -> Result<()> {
    if value { Ok(()) } else { Err(reason.into()) }
}

fn call(data: &mut DrawDataClient, mut request: DataRequest) -> Result<DataReply> {
    request.host_epoch = data.capabilities().host_epoch;
    let reply = data.request_with_deadline(&request, deadline())?;
    require(
        reply.code == data_code::OK,
        &format!(
            "API {}: {}",
            reply.code,
            String::from_utf8_lossy(&reply.message)
        ),
    )?;
    Ok(reply)
}

fn clear(data: &mut DrawDataClient, lease: u64, namespace: &str) -> Result<()> {
    call(
        data,
        DataRequest {
            lease,
            ..DataRequest::new(data_op::CLEAR)
        }
        .with_metadata(&NamespaceFilter {
            namespace: namespace.into(),
        })?,
    )?;
    Ok(())
}

fn status(data: &mut DrawDataClient, key: &DataKey) -> Result<DatasetStatus> {
    let reply = call(
        data,
        DataRequest::new(data_op::STATUS)
            .with_key(key)?
            .with_metadata(&DataStatusQuery {
                generation: None,
                view: None,
            })?,
    )?;
    Ok(DatasetStatus::from_json(&reply.metadata)?)
}

fn upload(writer: &mut DrawDataWriter, fixture: &fixture::Fixture) -> Result<DatasetRevision> {
    let pages: Vec<_> = fixture
        .pages
        .iter()
        .map(|(descriptor, bytes)| DataUploadPage { descriptor, bytes })
        .collect();
    let reply = writer.upload(
        &DataUpload::replace(&fixture.descriptor, &fixture.scans, &pages),
        deadline(),
        None,
    )?;
    Ok(DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    })
}

fn visible(
    data: &mut DrawDataClient,
    key: &DataKey,
    revision: DatasetRevision,
) -> Result<GenerationVisibility> {
    let value = data
        .wait_visible_with_deadline(key, revision, None, deadline())?
        .visibility();
    require(
        matches!(
            value,
            GenerationVisibility::Visible {
                source_samples: 1_000_000,
                selected_items: 1_000_000,
                disposition: FrontierDisposition::Drawn,
                ..
            }
        ),
        "million points did not complete a drawn frame",
    )?;
    Ok(value)
}

fn demo_visible(
    data: &mut DrawDataClient,
    key: &DataKey,
    revision: DatasetRevision,
    points: u32,
) -> Result<GenerationVisibility> {
    let value = data
        .wait_visible_with_deadline(key, revision, None, deadline())?
        .visibility();
    require(
        matches!(value, GenerationVisibility::Visible {
            source_samples,
            selected_items,
            disposition: FrontierDisposition::Drawn,
            ..
        } if source_samples == u64::from(points)
            && selected_items > 0
            && selected_items <= u64::from(data.capabilities().limits.visible_points)),
        "survey did not complete a bounded drawn selection",
    )?;
    Ok(value)
}

fn annotations(client: &Client, points: u32, pages: usize) -> Result<()> {
    let place = Place::default().no_drape();
    let tint = rgba(97, 211, 182, 255);
    let title = format!("{:.1}M / SURVEY SAMPLES", f64::from(points) / 1_000_000.0);
    let detail = format!("{pages} PAGES / SOURCE RGB / CAMERA-ADAPTIVE DETAIL");
    for object in [
        place
            .put(E::text(NS, "title", &[[0.0, 16.0, 9.0]], &title))
            .with_style(draw_style::LABEL)
            .with_width(-21.0)
            .with_color(rgba(225, 235, 242, 255))
            .with_prop("accent", &tint.to_string())
            .with_prop("label_category", "TYPED CLOUD / SCALE"),
        place
            .put(E::text(NS, "detail", &[[0.0, -16.0, 0.7]], &detail))
            .with_style(draw_style::LABEL)
            .with_width(-14.0)
            .with_color(rgba(225, 235, 242, 255))
            .with_prop("accent", &tint.to_string()),
        place
            .put(E::shapes(
                NS,
                "bounds",
                shape_kind::BOX,
                &[Instance::at([0.0, 0.0, 4.0], [32.0, 32.0, 7.0])],
            ))
            .with_style(draw_style::HOLO)
            .with_fill(draw_flag::FILL_WIRE)
            .with_width(-0.8)
            .with_color(tint)
            .with_glow(0.18)
            .with_blur(0.2),
        place
            .put(E::lines(
                NS,
                "rim",
                &[
                    [-16.0, -16.0, 0.5],
                    [16.0, -16.0, 0.5],
                    [16.0, 16.0, 0.5],
                    [-16.0, 16.0, 0.5],
                    [-16.0, -16.0, 0.5],
                ],
            ))
            .with_style(draw_style::LIGHT_WALL)
            .with_width(0.025)
            .with_beam(0.18)
            .with_color(tint)
            .with_glow(0.28)
            .with_blur(0.5)
            .with_flow(0.4, 0.7),
    ] {
        let result = client.draw(&object)?;
        require(result.is_ok(), &result.message())?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let points = match std::env::var("GEARBOX_CLOUD_POINTS") {
        Ok(value) => value.parse::<u32>()?,
        Err(std::env::VarError::NotPresent) => fixture::POINTS,
        Err(error) => return Err(error.into()),
    };
    require(
        matches!(points, 100_000 | 1_000_000 | 5_000_000),
        "choose 100000, 1000000 or 5000000 source points",
    )?;
    require(
        std::env::var_os("GEARBOX_CLOUD_SCALE_PROBE").is_none() || points == fixture::POINTS,
        "the scale resource proof requires its exact million-point fixture",
    )?;
    let instance = std::env::args()
        .nth(1)
        .ok_or("provide owned INSTANCE or manifest")?;
    if instance == "manifest" {
        println!("{}", fixture::manifest());
        return Ok(());
    }
    require(
        instance.starts_with("draw-world-preview-") || instance.starts_with("draw-vfx-bench-"),
        "scale demo requires an owned drawing preview",
    )?;
    let entry = registry::find(&instance).ok_or("instance is not running")?;
    let client = Client::connect(&entry.did, IdentitySource::Random, "typed-cloud-scale")?;
    let mut data = client.draw_data()?;
    let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline())?;
    let mut writer = pipeline.writer(4, deadline())?;
    let run = (|| -> Result<()> {
        for namespace in [NS, OBSERVER, "vfx-gallery/cloud-basics"] {
            clear(&mut data, writer.lease(), namespace)?;
            let result = client.draw_clear(namespace)?;
            require(result.is_ok(), &result.message())?;
        }
        let mut fixture = fixture::make_sized(DataKey::new(NS, "survey")?, points)?;
        if let Ok(strength) = std::env::var("GEARBOX_CLOUD_EYE_DOME") {
            fixture.descriptor.presentation.eye_dome = strength.parse()?;
        }
        let revision = if let Ok(root) = std::env::var("GEARBOX_CLOUD_SCALE_PROBE") {
            proof::run(
                &mut data,
                &mut writer,
                &fixture,
                std::path::Path::new(&root),
            )?
        } else {
            upload(&mut writer, &fixture)?
        };
        let provider = gearbox_api::draw_data::survey::SURVEY_PROVIDER;
        let paged = std::env::var_os("GEARBOX_CLOUD_SCALE_PROBE").is_none()
            && data
                .capabilities()
                .page_providers
                .iter()
                .any(|name| name == provider);
        if paged {
            writer.attach_provider(&fixture.descriptor.key, revision, provider, deadline())?;
        }
        clear(&mut data, writer.lease(), OBSERVER)?;
        writer.close(deadline())?;
        let receipt = if std::env::var_os("GEARBOX_CLOUD_SCALE_PROBE").is_some() {
            visible(&mut data, &fixture.descriptor.key, revision)?
        } else {
            demo_visible(&mut data, &fixture.descriptor.key, revision, points)?
        };
        annotations(&client, points, fixture.pages.len())?;
        println!(
            "{}",
            serde_json::json!({"scene":"cloud-scale","transport":"public typed writer","source_points":points,"page_provider":paged.then_some(provider),"visible_point_limit":data.capabilities().limits.visible_points,"datasets":[{"key":fixture.descriptor.key,"incarnation":revision.incarnation,"stored_generation":revision.generation}],"visible_generation_ack":[{"key":fixture.descriptor.key,"visibility":receipt}]})
        );
        Ok(())
    })();
    if run.is_err()
        && let Ok(reply) = call(&mut data, DataRequest::new(data_op::OPEN_WRITER))
    {
        let _ = clear(&mut data, reply.lease, NS);
        let _ = clear(&mut data, reply.lease, OBSERVER);
        let _ = call(
            &mut data,
            DataRequest {
                lease: reply.lease,
                ..DataRequest::new(data_op::CLOSE_WRITER)
            },
        );
    }
    let closed = writer.close(deadline());
    run?;
    closed?;
    Ok(())
}
