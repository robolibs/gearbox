//! The bay every typed-data lab shares with the cloud and octree labs: a
//! numbered title with its category, a detail line, a HOLO wire bound and an
//! energized light-wall rim that follows the ground. Bays stand on the
//! terrain; datasets inside use `on_ground` frames at the bay centre.

use gearbox_api::wire::draw::rgba;
use gearbox_api::wire::draw_widgets::Place;
use gearbox_api::{Client, DrawEntity as E, Instance, draw_flag, draw_style, shape_kind};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct Bay<'a> {
    pub id: &'a str,
    /// Bay centre, east and north metres.
    pub at: [f64; 2],
    /// Bound width (east), depth (north) and height.
    pub size: [f32; 3],
    pub title: String,
    pub detail: &'a str,
    pub category: &'a str,
    pub tint: u32,
}

/// Clears and redraws the bays' annotations in `ns`.
pub fn bays(client: &Client, ns: &str, bays: &[Bay<'_>]) -> Result<()> {
    let _ = client.draw_clear(ns);
    let place = Place::default();
    for bay in bays {
        let [x, y] = bay.at.map(|v| v as f32);
        let [w, d, h] = bay.size;
        let id = |part: &str| format!("{}-{part}", bay.id);
        let rim = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5], [-0.5, -0.5]].map(|[a, b]| [x + a * w, y + b * d, 0.3]);
        let objects = [
            place
                .put(E::text(ns, &id("title"), &[[x, y + d * 0.5 + 0.2, h + 0.5]], &bay.title))
                .with_style(draw_style::LABEL)
                .with_width(-17.0)
                .with_color(rgba(224, 235, 242, 255))
                .with_prop("accent", &bay.tint.to_string())
                .with_prop("label_category", bay.category),
            place
                .put(E::text(ns, &id("detail"), &[[x, y - d * 0.5 - 0.35, 0.55]], bay.detail))
                .with_style(draw_style::LABEL)
                .with_width(-11.0)
                .with_color(rgba(214, 226, 236, 255))
                .with_prop("accent", &bay.tint.to_string()),
            place
                .put(E::shapes(ns, &id("bounds"), shape_kind::BOX, &[Instance::at([x, y, h * 0.5], [w, d, h])]))
                .with_style(draw_style::HOLO)
                .with_fill(draw_flag::FILL_WIRE)
                .with_width(-0.8)
                .with_color(bay.tint)
                .with_glow(0.18)
                .with_blur(0.2),
            place
                .put(E::lines(ns, &id("rim"), &rim))
                .with_style(draw_style::LIGHT_WALL)
                .with_width(0.02)
                .with_beam(0.16)
                .with_color(bay.tint)
                .with_glow(0.3)
                .with_blur(0.5)
                .with_flow(0.4, 0.7),
        ];
        for object in objects {
            let status = client.draw(&object)?;
            if !status.is_ok() {
                return Err(status.message().into());
            }
        }
    }
    Ok(())
}

/// A free-standing label in the bay family, e.g. a lab title.
pub fn label(client: &Client, ns: &str, id: &str, at: [f64; 3], text: &str, size: f32, tint: u32, category: Option<&str>) -> Result<()> {
    let mut entity = Place::default()
        .put(E::text(ns, id, &[at.map(|v| v as f32)], text))
        .with_style(draw_style::LABEL)
        .with_width(size)
        .with_color(rgba(224, 235, 242, 255))
        .with_prop("accent", &tint.to_string());
    if let Some(category) = category {
        entity = entity.with_prop("label_category", category);
    }
    let status = client.draw(&entity)?;
    if !status.is_ok() {
        return Err(status.message().into());
    }
    Ok(())
}
