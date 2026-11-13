//! `gearbox draw data`: typed spatial datasets — listing, status, layer and
//! slice controls and field queries.

use std::time::{Duration, Instant};

use clap::Subcommand;
use gearbox_api::wire::draw_data::*;
use gearbox_api::{DrawDataClient, colormap};
use serde_json::json;

use crate::ctx::Ctx;
use crate::error::{CliError, Result};
use crate::out::pretty_json;

#[derive(Subcommand, Debug)]
pub enum DataCmd {
    /// Datasets under NS (all when omitted)
    List { ns: Option<String> },
    /// One dataset's retained revision, transfer and visibility
    Status { ns: String, id: String },
    /// Read a volume at volume-frame points; unknown support answers no value
    Field {
        ns: String,
        id: String,
        /// A point as X Y Z (repeatable)
        #[arg(long = "at", num_args = 3, value_names = ["X", "Y", "Z"], required = true, allow_hyphen_values = true)]
        at: Vec<f64>,
        /// Also read the central-difference gradient one voxel apart
        #[arg(long)]
        gradient: bool,
        /// Read the containing voxel instead of trilinear interpolation
        #[arg(long)]
        nearest: bool,
        /// Read TSDF integration weights instead of values
        #[arg(long)]
        weight: bool,
    },
    /// Colour an elevation map by one layer, replacing its presentation; geometry is kept
    Layer {
        ns: String,
        id: String,
        /// `elevation` or a named layer
        name: String,
        #[arg(long, default_value = "turbo")]
        colormap: String,
        /// Colormap range LO HI
        #[arg(long, num_args = 2, value_names = ["LO", "HI"], allow_hyphen_values = true, required = true)]
        range: Vec<f64>,
        /// Colour of samples the layer leaves missing, RRGGBBAA
        #[arg(long, default_value = "5a5a5aff")]
        missing: String,
    },
    /// Show a slice through a volume, replacing its presentation
    Slice {
        ns: String,
        id: String,
        /// Plane normal axis: x, y or z
        #[arg(long, default_value = "z")]
        axis: String,
        /// Slice centre X Y Z in the volume frame
        #[arg(long, num_args = 3, value_names = ["X", "Y", "Z"], allow_hyphen_values = true)]
        center: Vec<f64>,
        /// Extent along the plane's two axes, metres
        #[arg(long, num_args = 2, value_names = ["W", "H"], default_values = ["4", "4"])]
        size: Vec<f64>,
        /// Edge of one sampled cell, metres
        #[arg(long, default_value_t = 0.05)]
        cell: f64,
        /// Colour by `value` or `weight`
        #[arg(long, default_value = "value")]
        by: String,
        #[arg(long, default_value = "turbo")]
        colormap: String,
        /// Colormap range LO HI
        #[arg(long, num_args = 2, value_names = ["LO", "HI"], allow_hyphen_values = true, required = true)]
        range: Vec<f64>,
        /// Leave out cells whose value lies outside LO HI
        #[arg(long, num_args = 2, value_names = ["LO", "HI"], allow_hyphen_values = true)]
        band: Option<Vec<f64>>,
        /// Mark a level where neighbouring cells straddle it (repeatable)
        #[arg(long = "contour", allow_hyphen_values = true)]
        contours: Vec<f64>,
        /// Read the containing voxel instead of trilinear interpolation
        #[arg(long)]
        nearest: bool,
        /// Unknown-support colour RRGGBBAA
        #[arg(long, default_value = "5a1e64ff")]
        unknown: String,
    },
    /// Delete one dataset
    Delete { ns: String, id: String },
    /// Delete every dataset under NS
    Clear { ns: String },
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

fn failed(what: &str) -> impl Fn(&dyn std::fmt::Display) -> CliError + '_ {
    move |error| CliError::error(format!("{what}: {error}"))
}

fn rgba(text: &str) -> Result<[u8; 4]> {
    let value = u32::from_str_radix(text.trim_start_matches('#'), 16)
        .ok()
        .filter(|_| text.trim_start_matches('#').len() == 8)
        .ok_or_else(|| CliError::usage(format!("colour `{text}` is not RRGGBBAA")))?;
    Ok(value.to_be_bytes())
}

fn request(data: &DrawDataClient, op: u32, key: Option<&DataKey>) -> Result<DataRequest> {
    let mut request = DataRequest { host_epoch: data.capabilities().host_epoch, ..DataRequest::new(op) };
    if let Some(key) = key {
        request = request.with_key(key).map_err(|e| failed("key")(&e))?;
    }
    Ok(request)
}

fn checked(data: &mut DrawDataClient, request: &DataRequest, what: &str) -> Result<DataReply> {
    let reply = data.request_with_deadline(request, deadline()).map_err(|e| failed(what)(&e))?;
    if reply.code != data_code::OK {
        return Err(CliError::refused(format!("{what}: {}", String::from_utf8_lossy(&reply.message))));
    }
    Ok(reply)
}

/// Runs `body` under a fresh writer lease, closing it whatever happens.
fn with_lease<T>(data: &mut DrawDataClient, body: impl FnOnce(&mut DrawDataClient, u64) -> Result<T>) -> Result<T> {
    let open = checked(data, &request(data, data_op::OPEN_WRITER, None)?, "writer")?;
    let result = body(data, open.lease);
    if let Ok(close) = request(data, data_op::CLOSE_WRITER, None) {
        let _ = data.request_with_deadline(&DataRequest { lease: open.lease, ..close }, deadline());
    }
    result
}

fn retained(data: &mut DrawDataClient, key: &DataKey) -> Result<DatasetRevision> {
    let status = request(data, data_op::STATUS, Some(key))?
        .with_metadata(&DataStatusQuery::default())
        .map_err(|e| failed("status")(&e))?;
    let reply = checked(data, &status, "status")?;
    if reply.incarnation == 0 {
        return Err(CliError::refused(format!("{}/{} has no retained generation", key.namespace, key.id)));
    }
    Ok(DatasetRevision { incarnation: reply.incarnation, generation: reply.stored_generation })
}

fn print(ctx: &Ctx, value: &serde_json::Value) {
    if ctx.json {
        println!("{}", serde_json::to_string(value).unwrap_or_default());
    } else {
        println!("{}", pretty_json(value));
    }
}

/// Replaces the presentation of the retained generation; no data is re-sent.
fn restyle(ctx: &Ctx, key: DataKey, presentation: &Presentation, what: &str) -> Result<()> {
    presentation.validate_intrinsic().map_err(CliError::usage)?;
    let mut data = ctx.client()?.draw_data().map_err(|e| failed("draw data")(&e))?;
    let base = retained(&mut data, &key)?;
    let reply = with_lease(&mut data, |data, lease| {
        let patch = DataRequest {
            lease,
            incarnation: base.incarnation,
            base_generation: base.generation,
            ..request(data, data_op::PATCH_PRESENTATION, Some(&key))?
        }
        .with_metadata(presentation)
        .map_err(|e| failed(what)(&e))?;
        checked(data, &patch, what)
    })?;
    print(ctx, &json!({"ns": key.namespace, "id": key.id, "generation": reply.stored_generation}));
    Ok(())
}

pub fn run(ctx: &Ctx, cmd: DataCmd) -> Result<()> {
    let key = |ns: &str, id: &str| DataKey::new(ns, id).map_err(|e| CliError::usage(format!("dataset key: {e}")));
    match cmd {
        DataCmd::Layer { ns, id, name, colormap: map, range, missing } => {
            let colormap = colormap::parse(&map).ok_or_else(|| CliError::usage(format!("unknown colormap `{map}`")))?;
            let presentation = Presentation {
                color: ColorMode::Scalar { name, colormap, range: [range[0], range[1]], missing: rgba(&missing)? },
                ..Presentation::default()
            };
            return restyle(ctx, key(&ns, &id)?, &presentation, "layer");
        }
        DataCmd::List { ns } => {
            let mut data = ctx.client()?.draw_data().map_err(|e| failed("draw data")(&e))?;
            let query = DataListQuery { namespace: ns.unwrap_or_default().trim_matches('/').into(), ..DataListQuery::default() };
            let list = request(&data, data_op::LIST, None)?.with_metadata(&query).map_err(|e| failed("list")(&e))?;
            let reply = checked(&mut data, &list, "list")?;
            print(ctx, &serde_json::from_slice(&reply.metadata)?);
        }
        DataCmd::Status { ns, id } => {
            let mut data = ctx.client()?.draw_data().map_err(|e| failed("draw data")(&e))?;
            let status = request(&data, data_op::STATUS, Some(&key(&ns, &id)?))?
                .with_metadata(&DataStatusQuery::default())
                .map_err(|e| failed("status")(&e))?;
            let reply = checked(&mut data, &status, "status")?;
            print(ctx, &serde_json::from_slice(&reply.metadata)?);
        }
        DataCmd::Field { ns, id, at, gradient, nearest, weight } => {
            let mut data = ctx.client()?.draw_data().map_err(|e| failed("draw data")(&e))?;
            let query = FieldQuery {
                points: at.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect(),
                channel: if weight { Channel::Weight } else { Channel::Value },
                interpolation: if nearest { Interpolation::Nearest } else { Interpolation::Trilinear },
                gradient,
                generation: None,
            };
            query.validate().map_err(CliError::usage)?;
            let field = request(&data, data_op::FIELD_QUERY, Some(&key(&ns, &id)?))?
                .with_metadata(&query)
                .map_err(|e| failed("field")(&e))?;
            let reply = checked(&mut data, &field, "field")?;
            print(ctx, &serde_json::from_slice(&reply.metadata)?);
        }
        DataCmd::Slice { ns, id, axis, center, size, cell, by, colormap: map, range, band, contours, nearest, unknown } => {
            let axis = match axis.as_str() {
                "x" => 0,
                "y" => 1,
                "z" => 2,
                other => return Err(CliError::usage(format!("axis `{other}` is not x, y or z"))),
            };
            let center: [f64; 3] = center.try_into().map_err(|_| CliError::usage("--center takes X Y Z"))?;
            let colormap = colormap::parse(&map).ok_or_else(|| CliError::usage(format!("unknown colormap `{map}`")))?;
            let presentation = Presentation {
                color: ColorMode::Scalar { name: by, colormap, range: [range[0], range[1]], missing: [0, 0, 0, 0] },
                volume: Some(VolumeView {
                    slices: vec![VolumeSlice::across(axis, center, [size[0], size[1]], cell)],
                    interpolation: if nearest { Interpolation::Nearest } else { Interpolation::Trilinear },
                    band: band.map(|b| [b[0], b[1]]),
                    contours,
                    contour_rgba: [255, 255, 255, 255],
                    unknown_rgba: rgba(&unknown)?,
                    frame_rgba: Some([235, 240, 255, 255]),
                }),
                ..Presentation::default()
            };
            restyle(ctx, key(&ns, &id)?, &presentation, "slice")?;
        }
        DataCmd::Delete { ns, id } => {
            let key = key(&ns, &id)?;
            let mut data = ctx.client()?.draw_data().map_err(|e| failed("draw data")(&e))?;
            with_lease(&mut data, |data, lease| {
                checked(data, &DataRequest { lease, ..request(data, data_op::DELETE, Some(&key))? }, "delete")
            })?;
        }
        DataCmd::Clear { ns } => {
            let client = ctx.client()?;
            let mut data = client.draw_data().map_err(|e| failed("draw data")(&e))?;
            with_lease(&mut data, |data, lease| {
                let clear = DataRequest { lease, ..request(data, data_op::CLEAR, None)? }
                    .with_metadata(&NamespaceFilter { namespace: ns.trim_matches('/').into() })
                    .map_err(|e| failed("clear")(&e))?;
                checked(data, &clear, "clear")
            })?;
        }
    }
    Ok(())
}
