//! `gearbox draw`: retained debug draws in the viewer, kept per `NS` + `ID`
//! until deleted, cleared or out of lifetime. Coordinates are Z-up in the
//! frame: `world` (home: x east, y north, z up), `MACHINE/odom` (where the
//! machine's reported pose is measured from) or `MACHINE/LINK`. Points are
//! `X,Y[,Z]`, several to an argument when split by `;`; points after `--`
//! may start with a minus sign. Widgets made of several entities live
//! under `NS/ID`, so `gearbox draw clear NS/ID` removes them whole.

use std::time::{Duration, Instant};

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::wire::draw::parse_color;
use gearbox_api::wire::draw_widgets::{self as w, Detection3d, DetectionStyle, Glyph, Place, Pose, TerrainGlyph, Zone};
use gearbox_api::{
    DrawEntity, GridCells, Instance, SceneEvent, colormap, draw_flag, draw_kind, draw_style, event_kind, interact_kind,
    line_mode, next_sample, panel_kind, point_style, shape_kind,
};
use serde_json::json;

use super::check;
use crate::ctx::Ctx;
use crate::error::{CliError, Result};
use crate::out::Table;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(ClapArgs, Debug, Clone)]
struct Common {
    /// Path the entity lives under, e.g. `lab/robot`
    ns: String,
    /// The entity's id under NS
    id: String,
    /// Frame: world, MACHINE/odom, MACHINE/LINK or MACHINE (its base link)
    #[arg(long, default_value = "world")]
    frame: String,
    /// Colour: #rgb, #rrggbb, #rrggbbaa or a name. Axis RGB, class/status palettes and scalar maps retain their meanings.
    #[arg(long)]
    color: Option<String>,
    /// Line, outline or point width: > 0 metres, < 0 pixels
    #[arg(long, allow_negative_numbers = true)]
    width: Option<f32>,
    /// Sim seconds it lives; 0 keeps it
    #[arg(long, default_value_t = 0.0)]
    lifetime: f64,
    /// Pose of the whole entity in its frame: X Y Z [YAW]
    #[arg(long, num_args = 3..=4, value_names = ["X", "Y", "Z", "YAW"], allow_negative_numbers = true)]
    pose: Option<Vec<f64>>,
    /// Draw over everything instead of depth tested
    #[arg(long)]
    on_top: bool,
    /// Keep the frame's pose at arrival instead of following it
    #[arg(long)]
    snapshot: bool,
    /// One label per item (strip, point, arrow, shape, polygon), repeated
    #[arg(long = "label")]
    labels: Vec<String>,
    /// Caller-supplied annotation category
    #[arg(long)]
    label_category: Option<String>,
    /// Caller-supplied annotation detail or metric
    #[arg(long)]
    label_detail: Option<String>,
    /// Annotation icon; no operational state is inferred
    #[arg(long, value_parser = ["robot", "sensor", "destination", "measurement", "obstacle", "frame"])]
    label_icon: Option<String>,
    /// Select annotation styling; --label-selected=false clears it
    #[arg(long, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
    label_selected: Option<bool>,
    /// Panel backing and foreground contrast
    #[arg(long, value_parser = ["dark", "light"])]
    panel_theme: Option<String>,
    /// Glass finish: caller color, white + color, or black + color
    #[arg(long, value_parser = ["color", "white", "black"])]
    panel_finish: Option<String>,
    /// Look: auto, plain, ribbon_flow, tube_pulse, hairline, light_wall, beacon, holo, shield, alert, orb, decal, arrow_glow, point_glow, label, heat, splat, sweep, ghost, ruler, field
    #[arg(long, default_value = "auto")]
    style: String,
    /// Brightness of the style's light (1 usual); the effects below take the style's default when left out and switch off below 0
    #[arg(long, allow_negative_numbers = true)]
    glow: Option<f32>,
    /// Frost strength from 0 (clear) to 1 (full), independent of glow
    #[arg(long)]
    blur: Option<f32>,
    /// Rate of pulses, ripples and breathing, Hz
    #[arg(long, allow_negative_numbers = true)]
    pulse: Option<f32>,
    /// Speed patterns travel along lines, m/s
    #[arg(long, allow_negative_numbers = true)]
    flow: Option<f32>,
    /// Spacing of chevrons, dashes, stripes, ticks, rings and contours, metres
    #[arg(long, allow_negative_numbers = true)]
    spacing: Option<f32>,
    /// Height of beams and light walls, metres
    #[arg(long, allow_negative_numbers = true)]
    beam: Option<f32>,
    /// Opacity lines fade to at their faded end, 0..1
    #[arg(long, allow_negative_numbers = true)]
    fade: Option<f32>,
    /// How pressing it is, 0..1: shields ripple faster and turn vermillion
    #[arg(long)]
    urgency: Option<f32>,
    /// Opacity of the whole drawing, 0..1 (default 0.5, 1 fully opaque)
    #[arg(long)]
    opacity: Option<f32>,
    /// Lie flat in the frame's XY plane instead of facing the camera
    #[arg(long)]
    flat: bool,
    /// Add as light instead of blending over what is behind
    #[arg(long)]
    additive: bool,
    /// Take z from the frame itself, not as height above the terrain (world and odom frames)
    #[arg(long)]
    no_drape: bool,
    /// Values per item, coloured through --colormap (comma separated)
    #[arg(long, allow_negative_numbers = true)]
    values: Option<String>,
    /// Colormap for values and cells: turbo, viridis, rdylgn, costmap, map, raw, okabe, rainbow, grey
    #[arg(long)]
    colormap: Option<String>,
    /// Value at the colormap's start and end (equal: the data's own range; reversed runs it backwards)
    #[arg(long, num_args = 2, value_names = ["VMIN", "VMAX"], allow_negative_numbers = true)]
    range: Option<Vec<f32>>,
}

#[derive(ClapArgs, Debug)]
struct UpdateOptions {
    /// Frost strength from 0 (clear) to 1 (full)
    #[arg(long)]
    blur: Option<f32>,
    #[arg(long, num_args = 3..=4, value_names = ["X", "Y", "Z", "YAW"], allow_negative_numbers = true)]
    pose: Option<Vec<f64>>,
    #[arg(long, num_args = 3, value_names = ["X", "Y", "Z"], allow_negative_numbers = true)]
    scale: Option<Vec<f32>>,
    #[arg(long)]
    color: Option<String>,
    #[arg(long, alias = "outline")]
    color2: Option<String>,
    #[arg(long, allow_negative_numbers = true)]
    width: Option<f32>,
    #[arg(long)]
    style: Option<String>,
    #[arg(long, allow_negative_numbers = true)]
    glow: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    pulse: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    flow: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    spacing: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    beam: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    fade: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    urgency: Option<f32>,
    #[arg(long)]
    opacity: Option<f32>,
    /// Replace the complete flags bitmask
    #[arg(long)]
    flags: Option<u32>,
    #[arg(long)]
    colormap: Option<String>,
    #[arg(long, allow_negative_numbers = true)]
    vmin: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    vmax: Option<f32>,
    #[arg(long)]
    lifetime: Option<f64>,
    #[arg(long, value_parser = ["color", "white", "black"])]
    panel_finish: Option<String>,
    #[arg(long, value_parser = ["dark", "light"])]
    panel_theme: Option<String>,
    /// Merge metadata; repeat for each KEY=VALUE (ns, id and set are reserved)
    #[arg(long = "prop", value_name = "KEY=VALUE")]
    props: Vec<String>,
}

impl UpdateOptions {
    fn entity(&self, ns: &str, id: &str) -> Result<DrawEntity> {
        let mut e = DrawEntity::update(ns, id);
        if let Some(blur) = self.blur { e = e.with_blur(blur); }
        let mut fields = Vec::new();
        if let Some(p) = &self.pose {
            e = e.at_yaw([p[0], p[1], p[2]], p.get(3).copied().unwrap_or(0.0));
            fields.push("pose");
        }
        if let Some(s) = &self.scale {
            (e.sx, e.sy, e.sz) = (s[0], s[1], s[2]);
            fields.push("scale");
        }
        for (name, field, value) in [("tint", &mut e.color, &self.color), ("color2", &mut e.color2, &self.color2)] {
            if value.is_some() {
                *field = color(value)?;
                fields.push(name);
            }
        }
        if let Some(s) = &self.style {
            e.style = draw_style::parse(s).ok_or_else(|| CliError::usage(format!("unknown style `{s}`")))?;
            fields.push("style");
        }
        for (name, field, value) in [
            ("width", &mut e.width, self.width),
            ("glow", &mut e.glow, self.glow),
            ("pulse_hz", &mut e.pulse_hz, self.pulse),
            ("flow_speed", &mut e.flow_speed, self.flow),
            ("dash", &mut e.dash, self.spacing),
            ("beam_height", &mut e.beam_height, self.beam),
            ("fade", &mut e.fade, self.fade),
            ("urgency", &mut e.urgency, self.urgency),
            ("opacity", &mut e.opacity, self.opacity),
            ("vmin", &mut e.vmin, self.vmin),
            ("vmax", &mut e.vmax, self.vmax),
        ] {
            if let Some(value) = value {
                *field = value;
                fields.push(name);
            }
        }
        if let Some(flags) = self.flags {
            e.flags = flags;
            fields.push("flags");
        }
        if let Some(map) = &self.colormap {
            e.colormap = colormap::parse(map).ok_or_else(|| CliError::usage(format!("unknown colormap `{map}`")))?;
            fields.push("palette");
        }
        if let Some(lifetime) = self.lifetime {
            e.lifetime_s = lifetime;
            fields.push("lifetime");
        }
        for prop in &self.props {
            let (key, value) = prop.split_once('=').ok_or_else(|| CliError::usage("--prop expects KEY=VALUE"))?;
            if key.trim().is_empty() || matches!(key, "ns" | "id" | "set") {
                return Err(CliError::usage(format!("reserved or empty update property `{key}`")));
            }
            e = e.with_prop(key, value);
        }
        for (key, value) in [("panel_finish", &self.panel_finish), ("panel_theme", &self.panel_theme)] {
            if let Some(value) = value { e = e.with_prop(key, value); }
        }
        if fields.is_empty() {
            if self.props.is_empty() && self.panel_finish.is_none() && self.panel_theme.is_none() && self.blur.is_none() {
                return Err(CliError::usage("update requires at least one field or --prop KEY=VALUE"));
            }
            fields.push("props");
        }
        Ok(e.updating(&fields))
    }
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Polylines through X,Y[,Z] points; `/` starts another strip
    Lines {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1.., value_name = "X,Y[,Z]")]
        points: Vec<String>,
        /// Close each strip
        #[arg(long = "loop")]
        closed: bool,
        /// Pair the points into separate segments
        #[arg(long)]
        segments: bool,
        /// Dashed, with this period in metres
        #[arg(long)]
        dash: Option<f32>,
        /// Shade as tubes
        #[arg(long)]
        shaded: bool,
        /// End each strip in an arrowhead
        #[arg(long)]
        arrow_tip: bool,
    },
    /// Spheres (or flat discs, square splats) at X,Y[,Z] points
    Points {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1.., value_name = "X,Y[,Z]")]
        points: Vec<String>,
        /// Radius: > 0 metres, < 0 pixels
        #[arg(long, allow_negative_numbers = true)]
        radius: Option<f32>,
        /// Flat discs facing the camera instead of spheres
        #[arg(long)]
        disc: bool,
        /// Square splats
        #[arg(long)]
        square: bool,
    },
    /// Arrows from TAIL to TIP, X,Y[,Z] each, in pairs
    Arrows {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 2.., value_name = "X,Y[,Z]")]
        points: Vec<String>,
    },
    /// Shapes centred at X,Y[,Z[,YAW]] points
    Shapes {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1.., value_name = "X,Y[,Z[,YAW]]")]
        centers: Vec<String>,
        /// box, ellipsoid, cylinder, cone, capsule, ellipse, rect, frustum (HFOV,VFOV,RANGE) or sector (RADIUS,ANGLE,INNER)
        #[arg(long, default_value = "box")]
        shape: String,
        /// Full extents SX,SY[,SZ] in metres (angles in radians for frustums and sectors)
        #[arg(long, default_value = "1,1,1")]
        size: String,
        /// solid, wire or both
        #[arg(long, default_value = "solid")]
        fill: String,
        /// Outline colour for `--fill both`
        #[arg(long)]
        outline: Option<String>,
    },
    /// Text centred at X,Y[,Z] points
    Text {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1.., value_name = "X,Y[,Z]")]
        points: Vec<String>,
        /// What it says
        #[arg(long)]
        text: String,
        /// Pixels tall
        #[arg(long, default_value_t = 16.0)]
        size: f32,
    },
    /// An RGB axis triad at the entity pose
    Axes {
        #[command(flatten)]
        common: Common,
        /// Axis length in metres
        #[arg(long, default_value_t = 1.0)]
        length: f32,
    },
    /// A polygon with holes: its outline, then `/` and each hole; `//` starts another polygon
    Polygon {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 3.., value_name = "X,Y[,Z]")]
        points: Vec<String>,
        /// outline, fill or both
        #[arg(long, default_value = "both")]
        mode: String,
        /// Lift above the frame's (or ground's) z, metres
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        z_offset: f32,
        /// Colour each polygon by its index (polygon_ros Unique)
        #[arg(long)]
        unique: bool,
    },
    /// A frame: glowing triad, its name and a dashed line to its parent
    Frame {
        #[command(flatten)]
        common: Common,
        /// Parent position X,Y,Z
        #[arg(long, allow_hyphen_values = true)]
        parent: Option<String>,
        /// Axis length, metres
        #[arg(long, default_value_t = 0.5)]
        length: f32,
    },
    /// Poses X,Y[,Z[,YAW]] as chevrons, arrows or axes
    PoseArray {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1.., value_name = "X,Y[,Z[,YAW]]")]
        poses: Vec<String>,
        /// chevron, arrow or axes
        #[arg(long, default_value = "chevron")]
        glyph: String,
        #[arg(long, default_value_t = 0.5)]
        length: f32,
    },
    /// Numbered route stations and selected corner brackets joined by a ribbon
    Waypoints {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1.., value_name = "X,Y[,Z]")]
        points: Vec<String>,
        /// Index of the current waypoint
        #[arg(long, default_value_t = 0)]
        current: usize,
    },
    /// A covariance: 4 values (2×2) as draped rings, 9 (3×3) as shield ellipsoids, 36 (6×6) adds a yaw wedge
    Covariance {
        #[command(flatten)]
        common: Common,
        /// Mean X,Y[,Z[,YAW]]
        #[arg(long, allow_hyphen_values = true, default_value = "0,0,0")]
        mean: String,
        /// Row-major covariance, comma separated
        #[arg(long, allow_hyphen_values = true)]
        cov: String,
        /// Sigma rings, comma separated
        #[arg(long, default_value = "1,2,3")]
        sigmas: String,
    },
    /// A ruler from A to B with ticks, end caps and its length
    Ruler {
        #[command(flatten)]
        common: Common,
        #[arg(value_name = "X,Y[,Z]", allow_hyphen_values = true)]
        a: String,
        #[arg(value_name = "X,Y[,Z]", allow_hyphen_values = true)]
        b: String,
    },
    /// Change a live entity's header only (pose, colour, width, style, effects, colormap, lifetime)
    Update {
        ns: String,
        id: String,
        #[command(flatten)]
        changes: UpdateOptions,
    },
    /// A grid of cells as a heat map: COLS×ROWS cells from the pose (lower-left corner)
    Grid {
        #[command(flatten)]
        common: Common,
        #[arg(long)]
        cols: usize,
        #[arg(long)]
        rows: usize,
        /// Cell size, metres
        #[arg(long, default_value_t = 0.5)]
        cell: f32,
        /// Cell values, comma separated, row by row (or --demo)
        #[arg(long, allow_hyphen_values = true)]
        cells: Option<String>,
        /// u8, u16 or f32
        #[arg(long, default_value = "u8")]
        encoding: String,
        /// The value drawn as nothing
        #[arg(long, allow_negative_numbers = true)]
        unknown: Option<f32>,
        /// Fill with a smooth demo field instead of --cells
        #[arg(long)]
        demo: bool,
    },
    /// Replace a W×H rectangle of a live grid at column X, row Y
    GridPatch {
        ns: String,
        id: String,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        /// W×H cell values, comma separated
        #[arg(allow_hyphen_values = true)]
        cells: String,
        #[arg(long, default_value = "u8")]
        encoding: String,
    },
    /// 3-D detections: X,Y,Z,YAW,SX,SY,SZ,CLASS,SCORE[,TRACK[,VX,VY]] each
    Detection3d {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1..)]
        detections: Vec<String>,
        /// Dim boxes scoring under this
        #[arg(long, default_value_t = 0.0)]
        threshold: f32,
        /// Edges only
        #[arg(long)]
        only_edge: bool,
    },
    /// 2-D detections CX,CY,W,H,CLASS,SCORE (pixels) on a camera's image plane
    Detection2d {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1..)]
        boxes: Vec<String>,
        #[arg(long, default_value_t = 1.2)]
        hfov: f32,
        #[arg(long, default_value_t = 0.9)]
        vfov: f32,
        /// Image width and height, pixels
        #[arg(long, num_args = 2, default_values_t = [640.0, 480.0])]
        image: Vec<f32>,
        /// Distance of the image plane, metres
        #[arg(long, default_value_t = 2.0)]
        depth: f32,
    },
    /// A point cloud as splats; colour by --values or --axis
    Cloud {
        #[command(flatten)]
        common: Common,
        #[arg(num_args = 0.., value_name = "X,Y[,Z]")]
        points: Vec<String>,
        /// Colour by x, y or z
        #[arg(long)]
        axis: Option<String>,
        /// Splat size: > 0 metres, < 0 pixels
        #[arg(long, default_value_t = -1.5, allow_negative_numbers = true)]
        size: f32,
        /// Send positions as i16 steps of this many metres
        #[arg(long, default_value_t = 0.0)]
        quant: f32,
        /// A demo cloud of this many points instead of POINTS
        #[arg(long)]
        demo: Option<usize>,
    },
    /// A planar scan from ranges (metres; `inf` no return)
    Scan {
        #[command(flatten)]
        common: Common,
        #[arg(long, allow_hyphen_values = true)]
        ranges: String,
        #[arg(long, allow_negative_numbers = true)]
        angle_min: f32,
        #[arg(long, allow_negative_numbers = true)]
        increment: f32,
        #[arg(long, default_value_t = 0.0)]
        range_max: f32,
    },
    /// A sensor's field of view on the ground: a wedge along +x of the pose
    Sweep {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value_t = 5.0)]
        radius: f32,
        /// Opening angle, radians
        #[arg(long, default_value_t = 1.0)]
        angle: f32,
        #[arg(long, default_value_t = 0.0)]
        inner: f32,
    },
    /// A range sensor's cone with its reading
    RangeCone {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value_t = 0.5)]
        fov: f32,
        #[arg(long, default_value_t = 0.05)]
        min: f32,
        #[arg(long, default_value_t = 4.0)]
        max: f32,
        #[arg(long)]
        reading: Option<f32>,
    },
    /// A camera frustum along +x of the pose
    Frustum {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value_t = 1.2)]
        hfov: f32,
        #[arg(long, default_value_t = 0.9)]
        vfov: f32,
        #[arg(long, default_value_t = 0.3)]
        near: f32,
        #[arg(long, default_value_t = 4.0)]
        far: f32,
    },
    /// A particle cloud X,Y[,Z[,YAW[,WEIGHT]]] as flat chevrons
    Particles {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1..)]
        particles: Vec<String>,
        #[arg(long, default_value_t = 0.1)]
        min: f32,
        #[arg(long, default_value_t = 0.5)]
        max: f32,
        #[arg(long, default_value = "chevron")]
        glyph: String,
    },
    /// A field boundary (`/` before each hole) as a decal fenced by a wall of light
    Field {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 3..)]
        points: Vec<String>,
    },
    /// A headland: OUTER ring `/` INNER ring
    Headland {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 6..)]
        points: Vec<String>,
    },
    /// Swaths (`/` between them) with chevrons in the driving direction
    Swaths {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 2..)]
        points: Vec<String>,
        #[arg(long, default_value_t = 3.0)]
        swath_width: f32,
        /// How many are done
        #[arg(long, default_value_t = 0)]
        done: usize,
    },
    /// Headland turns (`/` between them) as narrow dashed ribbons
    Turns {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 2..)]
        points: Vec<String>,
        #[arg(long, default_value_t = 3.0)]
        swath_width: f32,
    },
    /// An AB line through A and B
    AbLine {
        #[command(flatten)]
        common: Common,
        #[arg(allow_hyphen_values = true)]
        a: String,
        #[arg(allow_hyphen_values = true)]
        b: String,
        #[arg(long, default_value_t = 50.0)]
        extend: f32,
    },
    /// Coverage: a pass count per cell
    Coverage {
        #[command(flatten)]
        common: Common,
        #[arg(long)]
        cols: usize,
        #[arg(long)]
        rows: usize,
        #[arg(long, default_value_t = 0.5)]
        cell: f32,
        #[arg(long, allow_hyphen_values = true)]
        counts: String,
    },
    /// An implement's sections (1 on, 0 off) behind its hitch
    Implement {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value_t = 6.0)]
        implement_width: f32,
        #[arg(long, default_value_t = 0.6)]
        depth: f32,
        /// Section states, e.g. 1,1,0,1
        #[arg(long)]
        sections: String,
    },
    /// A zone on the ground: keepout, slow or stop
    Zone {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 3..)]
        points: Vec<String>,
        #[arg(long, default_value = "keepout")]
        kind: String,
    },
    /// A wrench at the pose: FORCE X,Y,Z and TORQUE X,Y,Z
    Wrench {
        #[command(flatten)]
        common: Common,
        #[arg(long, allow_hyphen_values = true)]
        force: String,
        #[arg(long, allow_hyphen_values = true, default_value = "0,0,0")]
        torque: String,
        /// Metres per newton
        #[arg(long, default_value_t = 0.1)]
        scale: f32,
    },
    /// A contact at POINT with its NORMAL and force
    Contact {
        #[command(flatten)]
        common: Common,
        #[arg(allow_hyphen_values = true)]
        point: String,
        #[arg(allow_hyphen_values = true)]
        normal: String,
        #[arg(long, default_value_t = 10.0)]
        force: f32,
    },
    /// See-through copies of a machine at poses X,Y[,Z[,YAW]]
    Ghost {
        #[command(flatten)]
        common: Common,
        #[arg(long)]
        machine: String,
        #[arg(required = true, num_args = 1..)]
        poses: Vec<String>,
    },
    /// An in-world chart at the pose: sparkline, bars, gauge, chip, table or legend
    Panel {
        #[command(flatten)]
        common: Common,
        /// sparkline, bars, gauge, chip, table, legend
        #[arg(long, default_value = "table")]
        kind: String,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long, default_value = "")]
        unit: String,
        /// Chip text
        #[arg(long)]
        text: Option<String>,
        /// Table rows KEY=VALUE, repeated
        #[arg(long = "row")]
        rows: Vec<String>,
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        min: f32,
        #[arg(long, default_value_t = 100.0, allow_negative_numbers = true)]
        max: f32,
        /// Add the values to a sparkline instead of replacing it, keeping this many
        #[arg(long)]
        extend: Option<usize>,
    },
    /// A status pill over a machine
    Badge {
        ns: String,
        id: String,
        machine: String,
        text: String,
        #[arg(long, default_value = "plan")]
        color: String,
        #[arg(long, default_value_t = 2.0)]
        height: f32,
    },
    /// A graph: NODES X,Y[,Z] and --edge A,B (indices)
    Graph {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1..)]
        nodes: Vec<String>,
        #[arg(long = "edge")]
        edges: Vec<String>,
    },
    /// Wait for the Next button in the viewer
    Step {
        #[arg(default_value = "Next")]
        caption: String,
        #[arg(long)]
        full: bool,
    },
    /// Wait for a point (or with --pose a pose) picked on the ground
    Pick {
        #[arg(long)]
        pose: bool,
        #[arg(long, default_value = "world")]
        frame: String,
        #[arg(long, default_value = "")]
        prompt: String,
    },
    /// A 6-DoF handle at the pose; prints its feedback
    Handle {
        #[command(flatten)]
        common: Common,
        /// move, rotate, move_plane, move_axis, rotate_axis (comma separated)
        #[arg(long, default_value = "move,rotate")]
        controls: String,
        /// Menu entries, comma separated
        #[arg(long)]
        menu: Option<String>,
        /// Seconds to print feedback for
        #[arg(long, default_value_t = 30.0)]
        watch: f64,
    },
    /// A mesh from an OBJ or STL file at the pose (metres)
    Mesh {
        #[command(flatten)]
        common: Common,
        asset: String,
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
    },
    /// Voxels: boxes SIZE across at X,Y,Z centres
    Voxels {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1..)]
        centres: Vec<String>,
        #[arg(long, default_value_t = 0.2)]
        size: f32,
        #[arg(long)]
        holo: bool,
    },
    /// Batched world-space sample glyphs; the caller supplies their meaning
    TerrainMarks {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1.., value_name = "X,Y[,Z]")]
        points: Vec<String>,
        /// square, cross, triangle or diamond
        #[arg(long, default_value = "square")]
        glyph: String,
        /// Full glyph size in metres
        #[arg(long, default_value_t = 0.4)]
        size: f32,
    },
    /// Equal 3D arc-length marks along a supplied world-space route
    RouteStations {
        #[command(flatten)]
        common: Common,
        #[arg(required = true, num_args = 1.., value_name = "X,Y[,Z]")]
        points: Vec<String>,
        /// Distance between stations, metres; separate from effect --spacing
        #[arg(long, default_value_t = 1.0)]
        interval: f32,
        /// Station glyph size, metres
        #[arg(long, default_value_t = 0.3)]
        size: f32,
    },
    /// Broken ground rings and an upright diamond at --pose
    Destination {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value_t = 1.0)]
        radius: f32,
        #[arg(long, default_value = "")]
        text: String,
    },
    /// A flat ring round a machine at --pose: broken for hover and selected,
    /// closed with inward chevrons for following
    MachineRing {
        #[command(flatten)]
        common: Common,
        /// Outer radius, metres
        #[arg(long, default_value_t = 2.0)]
        radius: f32,
        /// hover, selected or following
        #[arg(long, default_value = "selected")]
        mark: String,
        /// Height over the ground (or the pose when not draped), metres
        #[arg(long, default_value_t = 0.1)]
        height: f32,
    },
    /// World-space corner brackets for supplied extents at --pose
    ClearanceBox {
        #[command(flatten)]
        common: Common,
        /// Full extents X,Y,Z, metres; not a collision/safety classification
        #[arg(long, default_value = "1,1,1")]
        size: String,
        #[arg(long, default_value = "")]
        text: String,
    },
    /// Transparent segmented field volume at --pose, with no screen HUD
    FieldVolume {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value = "1,1,1")]
        size: String,
        /// Rectangular grid spacing, metres
        #[arg(long, default_value_t = 0.5)]
        grid_spacing: f32,
    },
    /// A compass rose at X,Y[,Z]
    Compass {
        #[command(flatten)]
        common: Common,
        #[arg(allow_hyphen_values = true, default_value = "0,0,0")]
        at: String,
        #[arg(long, default_value_t = 4.0)]
        size: f32,
    },
    /// A scale bar LENGTH metres from X,Y[,Z]
    ScaleBar {
        #[command(flatten)]
        common: Common,
        #[arg(allow_hyphen_values = true)]
        at: String,
        length: f32,
    },
    /// A callout: TEXT on a leader line from ANCHOR
    Callout {
        #[command(flatten)]
        common: Common,
        #[arg(allow_hyphen_values = true)]
        anchor: String,
        text: String,
        #[arg(long, allow_hyphen_values = true, default_value = "0,0,1.5")]
        offset: String,
    },
    /// An angle at CENTRE from heading FROM to TO (radians)
    Angle {
        #[command(flatten)]
        common: Common,
        #[arg(allow_hyphen_values = true)]
        centre: String,
        #[arg(allow_hyphen_values = true)]
        from: f32,
        #[arg(allow_hyphen_values = true)]
        to: f32,
        #[arg(long, default_value_t = 1.0)]
        radius: f32,
    },
    /// Switch the TF tree overlay on (or off) for a machine
    Tf {
        machine: String,
        #[arg(long)]
        off: bool,
    },
    /// Remove one entity
    Delete { ns: String, id: String },
    /// Remove PREFIX and every path below it; all draws without one
    Clear { prefix: Option<String> },
    /// The live entities under PREFIX
    List { prefix: Option<String> },
    /// Typed spatial datasets: list, status, layers, slices and field queries
    Data {
        #[command(subcommand)]
        cmd: super::draw_data::DataCmd,
    },
}

fn numbers(text: &str) -> Result<Vec<f32>> {
    text.split(',')
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().parse::<f32>().map_err(|_| CliError::usage(format!("bad number in `{text}`"))))
        .collect()
}

fn point(text: &str) -> Result<[f32; 3]> {
    match numbers(text)?.as_slice() {
        [x, y] => Ok([*x, *y, 0.0]),
        [x, y, z] => Ok([*x, *y, *z]),
        _ => Err(CliError::usage(format!("`{text}` is not X,Y or X,Y,Z"))),
    }
}

fn pose(text: &str) -> Result<Pose> {
    let v = numbers(text)?;
    if v.len() < 2 {
        return Err(CliError::usage(format!("`{text}` is not X,Y[,Z[,YAW]]")));
    }
    let g = |i: usize| v.get(i).copied().unwrap_or(0.0) as f64;
    Ok(Pose::yawed(g(0), g(1), g(2), g(3)))
}

/// Every point token of the arguments, split on `;` and whitespace.
fn tokens(args: &[String]) -> impl Iterator<Item = &str> {
    args.iter().flat_map(|a| a.split(|c: char| c == ';' || c.is_whitespace())).filter(|t| !t.is_empty())
}

fn points_of(args: &[String]) -> Result<Vec<[f32; 3]>> {
    tokens(args).map(point).collect()
}

/// Groups of point lists: `/` ends a list, `//` a group of lists.
fn groups(args: &[String]) -> Result<Vec<Vec<Vec<[f32; 3]>>>> {
    let mut out: Vec<Vec<Vec<[f32; 3]>>> = vec![vec![Vec::new()]];
    for t in tokens(args) {
        match t {
            "//" => out.push(vec![Vec::new()]),
            "/" => out.last_mut().unwrap().push(Vec::new()),
            p => out.last_mut().unwrap().last_mut().unwrap().push(point(p)?),
        }
    }
    for g in &mut out {
        g.retain(|r| !r.is_empty());
    }
    out.retain(|g| !g.is_empty());
    Ok(out)
}

fn color(text: &Option<String>) -> Result<u32> {
    match text {
        None => Ok(0),
        Some(t) => parse_color(t).map(|c| c.max(1)).ok_or_else(|| CliError::usage(format!("bad colour `{t}`"))),
    }
}

fn place(c: &Common) -> Place {
    Place { frame: c.frame.clone(), lifetime: c.lifetime, drape: !c.no_drape }
}

fn entity_pose(c: &Common) -> Pose {
    match &c.pose {
        Some(p) => Pose::yawed(p[0], p[1], p[2], p.get(3).copied().unwrap_or(0.0)),
        None => Pose::default(),
    }
}

/// Common's look laid onto a widget's entity: colour, width, style, effects.
fn annotation(mut e: DrawEntity, c: &Common) -> DrawEntity {
    for (key, value) in [("label_category", &c.label_category), ("label_detail", &c.label_detail), ("label_icon", &c.label_icon)] {
        if let Some(value) = value { e = e.with_prop(key, value); }
    }
    if let Some(selected) = c.label_selected {
        e = e.with_prop("label_selected", if selected { "true" } else { "false" });
    }
    if let Some(theme) = &c.panel_theme { e = e.with_prop("panel_theme", theme); }
    if let Some(finish) = &c.panel_finish { e = e.with_prop("panel_finish", finish); }
    e
}

fn restyle(e: DrawEntity, c: &Common) -> Result<DrawEntity> {
    let mut e = e;
    if let Some(blur) = c.blur { e = e.with_blur(blur); }
    if c.color.is_some() && !matches!(e.op, gearbox_api::draw_op::DELETE) {
        e = e.with_color(color(&c.color)?);
    }
    if let Some(wd) = c.width {
        e = e.with_width(wd);
    }
    if c.style != "auto" {
        e = e.with_style(draw_style::parse(&c.style).ok_or_else(|| CliError::usage(format!("unknown style `{}`", c.style)))?);
    }
    for (v, slot) in [(c.glow, &mut e.glow), (c.pulse, &mut e.pulse_hz), (c.flow, &mut e.flow_speed), (c.beam, &mut e.beam_height), (c.fade, &mut e.fade), (c.urgency, &mut e.urgency), (c.opacity, &mut e.opacity)] {
        if let Some(v) = v {
            *slot = v;
        }
    }
    if let Some(s) = c.spacing {
        e.dash = s;
    }
    if c.on_top {
        e = e.with_flags(draw_flag::ON_TOP);
    }
    if e.op != gearbox_api::draw_op::DELETE {
        for (enabled, flag) in [(c.snapshot, draw_flag::SNAPSHOT), (c.flat, draw_flag::FLAT), (c.additive, draw_flag::ADDITIVE)] {
            if enabled { e = e.with_flags(flag); }
        }
    }
    Ok(annotation(e, c))
}

fn entity(kind: u32, c: &Common) -> Result<DrawEntity> {
    let mut e = DrawEntity::new(kind, &c.ns, &c.id)
        .in_frame(&c.frame)
        .with_color(color(&c.color)?)
        .with_width(c.width.unwrap_or(0.0))
        .with_lifetime(c.lifetime);
    if let Some(blur) = c.blur { e = e.with_blur(blur); }
    if let Some(pose) = &c.pose {
        let half = 0.5 * pose.get(3).copied().unwrap_or(0.0);
        e = e.with_pose([pose[0], pose[1], pose[2]], [half.cos(), 0.0, 0.0, half.sin()]);
    }
    if c.on_top {
        e = e.with_flags(draw_flag::ON_TOP);
    }
    if c.snapshot {
        e = e.with_flags(draw_flag::SNAPSHOT);
    }
    if !c.labels.is_empty() {
        e = e.with_flags(draw_flag::LABELS).with_labels(&c.labels);
    }
    let style = draw_style::parse(&c.style).ok_or_else(|| CliError::usage(format!("unknown style `{}`", c.style)))?;
    e = e
        .with_style(style)
        .with_glow(c.glow.unwrap_or(0.0))
        .with_pulse(c.pulse.unwrap_or(0.0))
        .with_flow(c.flow.unwrap_or(0.0), c.spacing.unwrap_or(0.0))
        .with_beam(c.beam.unwrap_or(0.0))
        .with_fade(c.fade.unwrap_or(0.0))
        .with_urgency(c.urgency.unwrap_or(0.0))
        .with_opacity(c.opacity.unwrap_or(0.0));
    if c.flat {
        e = e.with_flags(draw_flag::FLAT);
    }
    if c.additive {
        e = e.with_flags(draw_flag::ADDITIVE);
    }
    if !c.no_drape && gearbox_api::DrawFrame::drapes(&c.frame) {
        e = e.with_flags(draw_flag::DRAPE);
    }
    if let Some(v) = &c.values {
        e = e.with_values(&numbers(v)?).with_colormap(colormap::TURBO, 0.0, 0.0);
    }
    if let Some(map) = &c.colormap {
        e.colormap = colormap::parse(map).ok_or_else(|| CliError::usage(format!("unknown colormap `{map}`")))?;
    }
    if let Some(r) = &c.range {
        (e.vmin, e.vmax) = (r[0], r[1]);
    }
    Ok(annotation(e, c))
}

fn shape_entity(c: &Common, shape: u32) -> Result<DrawEntity> {
    Ok(entity(draw_kind::SHAPES, c)?.with_sub(shape)
        .with_scale([1.0, 1.0, if shape == shape_kind::SECTOR { 0.0 } else { 1.0 }]))
}

fn send(ctx: &Ctx, e: DrawEntity) -> Result<()> {
    let (ns, id, kind, items) = (e.ns(), e.id(), draw_kind::name(e.kind), e.item_count());
    check(ctx.client()?.draw(&e)?, &format!("draw {ns}/{id}"))?;
    ctx.done(&id, &format!("drew {kind} {ns}/{id} ({items} items)"), || {
        json!({ "ns": ns, "id": id, "kind": kind, "items": items })
    });
    Ok(())
}

/// A widget's entities, styled by `c`, sent in order.
fn send_all(ctx: &Ctx, c: &Common, what: &str, entities: Vec<DrawEntity>) -> Result<()> {
    let entities = entities.into_iter().map(|e| restyle(e, c)).collect::<Result<Vec<_>>>()?;
    send_prepared(ctx, c, what, entities)
}

fn send_prepared(ctx: &Ctx, c: &Common, what: &str, entities: Vec<DrawEntity>) -> Result<()> {
    let client = ctx.client()?;
    let mut parts = Vec::new();
    for e in entities {
        parts.push(format!("{}/{}", e.ns(), e.id()));
        check(client.draw(&e)?, &format!("draw {}/{}", e.ns(), e.id()))?;
    }
    let root = format!("{}/{}", c.ns, c.id);
    ctx.done(&root, &format!("drew {what} {root} ({} entities)", parts.len()), || json!({ "widget": what, "root": root, "parts": parts }));
    Ok(())
}

fn world_color(c: &Common) -> Result<u32> {
    if c.color.is_some() { color(&c.color) } else { Ok(gearbox_api::draw_color::PLAN) }
}

fn waypoint_entities(c: &Common, points: &[[f32; 3]], current: usize) -> Result<Vec<DrawEntity>> {
    Ok(w::waypoints_colored(&place(c), &c.ns, &c.id, points, current, world_color(c)?))
}

fn machine_ring_entities(c: &Common, radius: f32, mark: &str, height: f32) -> Result<Vec<DrawEntity>> {
    let mark = w::RingMark::parse(mark)
        .ok_or_else(|| CliError::usage(format!("unknown ring mark `{mark}`; choose hover, selected or following")))?;
    let tint = if c.color.is_some() { color(&c.color)? } else { 0 };
    Ok(w::machine_ring(&place(c), &c.ns, &c.id, [0.0, 0.0, height], radius, mark, tint))
}

fn world_size(text: &str) -> Result<[f32; 3]> {
    match numbers(text)?.as_slice() {
        [x, y, z] => Ok([*x, *y, *z]),
        _ => Err(CliError::usage(format!("`{text}` is not three full extents X,Y,Z"))),
    }
}

fn terrain_glyph(name: &str) -> Result<TerrainGlyph> {
    TerrainGlyph::parse(name).ok_or_else(|| CliError::usage(format!("unknown terrain glyph `{name}`")))
}

fn world_entities(c: &Common, entities: Vec<DrawEntity>) -> Result<Vec<DrawEntity>> {
    let style = Common { color: None, ..c.clone() };
    entities.into_iter().map(|mut e| {
        if e.op != gearbox_api::draw_op::DELETE {
            if c.pose.is_some() {
                let pose = entity_pose(c);
                e = e.with_pose(pose.position, pose.rotation);
            }
        }
        restyle(e, &style)
    }).collect()
}

fn send_world(ctx: &Ctx, c: &Common, what: &str, entities: Vec<DrawEntity>) -> Result<()> {
    send_prepared(ctx, c, what, world_entities(c, entities)?)
}

/// The next draw event for `ns`/`id` saying one of `whats`.
fn await_event(ctx: &Ctx, ns: &str, id: &str, whats: &[&str], timeout: Option<Duration>) -> Result<Option<SceneEvent>> {
    let client = ctx.client()?;
    let mut sub = client.events()?;
    let started = Instant::now();
    loop {
        if timeout.is_some_and(|t| started.elapsed() > t) {
            return Ok(None);
        }
        let Some(ev) = next_sample::<SceneEvent>(&mut sub, Duration::from_millis(500))? else {
            continue;
        };
        let p = ev.props();
        if ev.kind == event_kind::DRAW
            && p.get("ns").as_deref() == Some(ns)
            && p.get("id").as_deref() == Some(id)
            && p.get("what").is_some_and(|w| whats.contains(&w.as_str()))
        {
            return Ok(Some(ev));
        }
    }
}

fn grid_cells(text: &str, encoding: &str) -> Result<GridCells> {
    let v = numbers(text)?;
    Ok(match encoding {
        "u16" => GridCells::U16(v.iter().map(|c| *c as u16).collect()),
        "f32" => GridCells::F32(v),
        _ => GridCells::U8(v.iter().map(|c| *c as i32 as u8).collect()),
    })
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Lines { common, points, closed, segments, dash, shaded, arrow_tip } => {
            let mut strips: Vec<Vec<[f32; 3]>> = vec![Vec::new()];
            for p in tokens(&points) {
                if p == "/" {
                    strips.push(Vec::new());
                } else {
                    strips.last_mut().unwrap().push(point(p)?);
                }
            }
            strips.retain(|s| !s.is_empty());
            let mode = if segments { line_mode::LIST } else if closed { line_mode::LOOP } else { line_mode::STRIP };
            let mut e = entity(draw_kind::LINES, &common)?.with_sub(mode);
            for strip in &strips {
                e.offsets.push(e.point_count() as u32);
                e.points.extend(strip.iter().flatten());
            }
            if e.offsets.len() <= 1 {
                e.offsets.clear();
            }
            if let Some(period) = dash {
                e = e.with_dash(period);
            }
            if shaded {
                e = e.with_flags(draw_flag::SHADED);
            }
            if arrow_tip {
                e = e.with_flags(draw_flag::ARROW_TIP);
            }
            send(ctx, e)
        }
        Cmd::Points { common, points, radius, disc, square } => {
            let style = if square { w::POINT_SQUARE } else if disc { point_style::DISC } else { point_style::SPHERE };
            let pts = points_of(&points)?;
            let mut e = entity(draw_kind::POINTS, &common)?.with_points(&pts).with_sub(style);
            if let Some(r) = radius {
                e = e.with_width(2.0 * r);
            }
            send(ctx, e)
        }
        Cmd::Arrows { common, points } => {
            let pts = points_of(&points)?;
            if pts.len() % 2 != 0 {
                return Err(CliError::usage("arrows take TAIL TIP pairs"));
            }
            send(ctx, entity(draw_kind::ARROWS, &common)?.with_points(&pts))
        }
        Cmd::Shapes { common, centers, shape, size, fill, outline } => {
            let sub = shape_kind::parse(&shape).ok_or_else(|| CliError::usage(format!("unknown shape `{shape}`")))?;
            let fill = draw_flag::fill_parse(&fill).ok_or_else(|| CliError::usage(format!("unknown fill `{fill}`")))?;
            let s = numbers(&size)?;
            let size = [s.first().copied().unwrap_or(1.0), s.get(1).copied().unwrap_or(1.0), s.get(2).copied().unwrap_or(if sub == shape_kind::SECTOR { 0.0 } else { 1.0 })];
            let instances = tokens(&centers)
                .map(|c| {
                    let v = numbers(c)?;
                    if v.len() < 2 {
                        return Err(CliError::usage(format!("`{c}` is not X,Y[,Z[,YAW]]")));
                    }
                    let at = Instance::at([v[0], v[1], v.get(2).copied().unwrap_or(0.0)], size);
                    Ok(at.yawed(v.get(3).copied().unwrap_or(0.0)))
                })
                .collect::<Result<Vec<_>>>()?;
            let e = shape_entity(&common, sub)?
                .with_fill(fill)
                .with_instances(&instances)
                .with_color2(color(&outline)?);
            send(ctx, e)
        }
        Cmd::Text { common, points, text, size } => {
            let pts = points_of(&points)?;
            let mut e = entity(draw_kind::TEXT, &common)?.with_points(&pts).with_prop("text", &text).with_width(common.width.unwrap_or(-size.abs()));
            e.flags &= !draw_flag::LABELS;
            send(ctx, e)
        }
        Cmd::Axes { common, length } => {
            let e = entity(draw_kind::AXES, &common)?.with_scale([length; 3]);
            send(ctx, e)
        }
        Cmd::Polygon { common, points, mode, z_offset, unique } => {
            let polys = groups(&points)?;
            let fill = draw_flag::fill_parse(&mode).ok_or_else(|| CliError::usage(format!("unknown mode `{mode}`")))?;
            let mut e = entity(draw_kind::POLYGON, &common)?;
            let shaped = DrawEntity::polygons(&common.ns, &common.id, &polys);
            (e.points, e.offsets, e.rings) = (shaped.points, shaped.offsets, shaped.rings);
            e = e.with_fill(fill);
            if z_offset != 0.0 {
                e = e.with_prop("z_offset", &z_offset.to_string());
            }
            if unique {
                e = e.with_values(&(0..polys.len()).map(|k| k as f32).collect::<Vec<_>>()).with_colormap(colormap::OKABE, 0.0, 0.0);
            }
            send(ctx, e)
        }
        Cmd::Frame { common, parent, length } => {
            let parent = parent.as_deref().map(point).transpose()?;
            let label = common.labels.first().cloned();
            send_all(ctx, &common, "frame", w::frame(&place(&common), &common.ns, &common.id, entity_pose(&common), parent, length, label.as_deref()))
        }
        Cmd::PoseArray { common, poses, glyph, length } => {
            let glyph = Glyph::parse(&glyph).ok_or_else(|| CliError::usage(format!("unknown glyph `{glyph}`")))?;
            let poses = tokens(&poses).map(pose).collect::<Result<Vec<_>>>()?;
            let e = w::pose_array(&place(&common), &common.ns, &common.id, &poses, glyph, length);
            send(ctx, restyle(e, &common)?)
        }
        Cmd::Waypoints { common, points, current } => {
            let pts = points_of(&points)?;
            send_all(ctx, &Common { color: None, ..common.clone() }, "waypoints", waypoint_entities(&common, &pts, current)?)
        }
        Cmd::Covariance { common, mean, cov, sigmas } => {
            let cov: Vec<f64> = numbers(&cov)?.into_iter().map(|v| v as f64).collect();
            let sigmas = numbers(&sigmas)?;
            let parts = w::covariance(&place(&common), &common.ns, &common.id, pose(&mean)?, &cov, &sigmas, color(&common.color)?)
                .map_err(CliError::usage)?;
            send_all(ctx, &Common { color: None, ..common.clone() }, "covariance", parts)
        }
        Cmd::Ruler { common, a, b } => {
            let e = w::ruler(&place(&common), &common.ns, &common.id, point(&a)?, point(&b)?, common.spacing.unwrap_or(0.0));
            send(ctx, restyle(e, &common)?)
        }
        Cmd::Update { ns, id, changes } => {
            let e = changes.entity(&ns, &id)?;
            let groups = e.props().get("set").unwrap_or_default().split(',').map(str::to_owned).collect::<Vec<_>>();
            check(ctx.client()?.draw_raw(&e)?, &format!("update draw {ns}/{id}"))?;
            ctx.done(&id, &format!("updated {ns}/{id} ({})", groups.join(", ")), || json!({ "ns": ns, "id": id, "set": groups }));
            Ok(())
        }
        Cmd::Grid { common, cols, rows, cell, cells, encoding, unknown, demo } => {
            let cells = if demo {
                let n = cols * rows;
                GridCells::F32(
                    (0..n)
                        .map(|i| {
                            let (x, y) = ((i % cols) as f32 / cols as f32, (i / cols) as f32 / rows as f32);
                            (x * 6.0).sin() * (y * 4.0).cos() * 0.5 + 0.5
                        })
                        .collect(),
                )
            } else {
                grid_cells(cells.as_deref().unwrap_or(""), &encoding)?
            };
            let mut e = entity(draw_kind::GRID, &common)?;
            let shaped = DrawEntity::grid(&common.ns, &common.id, cols, rows, cell, cells);
            (e.sub, e.cells) = (shaped.sub, shaped.cells);
            e = e
                .with_prop("cols", &cols.to_string())
                .with_prop("rows", &rows.to_string())
                .with_prop("cell", &cell.to_string());
            if e.colormap == colormap::NONE {
                e.colormap = colormap::VIRIDIS;
            }
            if let Some(u) = unknown {
                e = e.with_prop("unknown", &u.to_string());
            }
            send(ctx, e)
        }
        Cmd::GridPatch { ns, id, x, y, w: width, h, cells, encoding } => {
            let e = DrawEntity::grid_patch(&ns, &id, x, y, width, h, grid_cells(&cells, &encoding)?);
            check(ctx.client()?.draw_raw(&e)?, &format!("patch grid {ns}/{id}"))?;
            ctx.done(&id, &format!("patched {ns}/{id} {width}×{h} at {x},{y}"), || json!({ "ns": ns, "id": id }));
            Ok(())
        }
        Cmd::Detection3d { common, detections, threshold, only_edge } => {
            let dets = detections
                .iter()
                .map(|d| {
                    let f: Vec<&str> = d.split(',').collect();
                    if f.len() < 9 {
                        return Err(CliError::usage(format!("`{d}` is not X,Y,Z,YAW,SX,SY,SZ,CLASS,SCORE[,TRACK[,VX,VY]]")));
                    }
                    let n = |i: usize| f.get(i).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
                    Ok(Detection3d {
                        pose: Pose::yawed(n(0) as f64, n(1) as f64, n(2) as f64, n(3) as f64),
                        size: [n(4), n(5), n(6)],
                        class: f[7].trim().to_string(),
                        score: n(8),
                        track: f.get(9).and_then(|v| v.trim().parse().ok()),
                        velocity: (f.len() >= 12).then(|| [n(10), n(11), 0.0]),
                        trail: Vec::new(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let style = DetectionStyle { threshold, only_edge, ..Default::default() };
            send_all(ctx, &Common { color: None, ..common.clone() }, "detections", w::detections_3d(&place(&common), &common.ns, &common.id, &dets, &style))
        }
        Cmd::Detection2d { common, boxes, hfov, vfov, image, depth } => {
            let mut rects = Vec::new();
            let mut classes = Vec::new();
            let mut scores = Vec::new();
            for b in &boxes {
                let f: Vec<&str> = b.split(',').collect();
                let n = |i: usize| f.get(i).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
                rects.push([n(0), n(1), n(2), n(3)]);
                classes.push(f.get(4).map(|s| s.trim().to_string()).unwrap_or_default());
                scores.push(n(5));
            }
            let parts = w::detections_2d(&place(&common), &common.ns, &common.id, entity_pose(&common), hfov, vfov, [image[0], image[1]], &rects, &classes, &scores, depth);
            send_all(ctx, &Common { color: None, ..common.clone() }, "detections", parts)
        }
        Cmd::Cloud { common, points, axis, size, quant, demo } => {
            let pts = match demo {
                Some(n) => (0..n)
                    .map(|i| {
                        let t = i as f32 * 0.618_034;
                        let (a, r) = ((t * 7.0) % std::f32::consts::TAU, 1.0 + (t * 3.1) % 6.0);
                        [r * a.cos(), r * a.sin(), 0.2 + 1.5 * ((r * 1.3).sin() * 0.5 + 0.5) * ((i % 17) as f32 / 17.0)]
                    })
                    .collect(),
                None => points_of(&points)?,
            };
            let values: Vec<f32> = match (&axis, &common.values) {
                (Some(a), _) => {
                    let k = "xyz".find(a.as_str()).ok_or_else(|| CliError::usage("axis is x, y or z"))?;
                    pts.iter().map(|p| p[k]).collect()
                }
                (None, Some(v)) => numbers(v)?,
                _ => Vec::new(),
            };
            let map = common.colormap.as_deref().and_then(colormap::parse).unwrap_or(colormap::TURBO);
            let e = w::cloud(&place(&common), &common.ns, &common.id, &pts, &values, map, size, quant);
            let mut c = common.clone();
            c.width = None;
            send(ctx, restyle(e.with_pose(entity_pose(&common).position, entity_pose(&common).rotation), &c)?)
        }
        Cmd::Scan { common, ranges, angle_min, increment, range_max } => {
            let ranges: Vec<f32> = ranges
                .split(',')
                .map(|r| if r.trim() == "inf" { f32::INFINITY } else { r.trim().parse().unwrap_or(f32::INFINITY) })
                .collect();
            send_all(ctx, &common, "scan", w::scan(&place(&common), &common.ns, &common.id, entity_pose(&common), &ranges, angle_min, increment, range_max))
        }
        Cmd::Sweep { common, radius, angle, inner } => {
            let e = shape_entity(&common, shape_kind::SECTOR)?
                .with_instances(&[Instance::at([0.0; 3], [radius, angle, inner])])
                .with_style(if common.style == "auto" { draw_style::SWEEP } else { draw_style::parse(&common.style).unwrap_or(draw_style::SWEEP) });
            send(ctx, e)
        }
        Cmd::RangeCone { common, fov, min, max, reading } => {
            let parts = w::range_cone(&place(&common), &common.ns, &common.id, entity_pose(&common), fov, min, max, reading.unwrap_or(f32::NAN));
            send_all(ctx, &common, "range cone", parts)
        }
        Cmd::Frustum { common, hfov, vfov, near, far } => {
            let e = w::camera_frustum(&place(&common), &common.ns, &common.id, entity_pose(&common), hfov, vfov, near, far);
            send(ctx, restyle(e, &common)?)
        }
        Cmd::Particles { common, particles, min, max, glyph } => {
            let mut poses = Vec::new();
            let mut weights = Vec::new();
            for p in tokens(&particles) {
                let v = numbers(p)?;
                let g = |i: usize| v.get(i).copied().unwrap_or(0.0);
                poses.push(Pose::yawed(g(0) as f64, g(1) as f64, g(2) as f64, g(3) as f64));
                weights.push(v.get(4).copied().unwrap_or(1.0));
            }
            let glyph = Glyph::parse(&glyph).ok_or_else(|| CliError::usage(format!("unknown glyph `{glyph}`")))?;
            let tint = if common.color.is_some() { color(&common.color)? } else { gearbox_api::draw_color::CYAN };
            let e = w::particles_colored(&place(&common), &common.ns, &common.id, &poses, &weights, min, max, glyph, tint);
            send(ctx, restyle(e, &Common { color: None, ..common.clone() })?)
        }
        Cmd::Field { common, points } => {
            let g = groups(&points)?;
            let rings = g.into_iter().next().unwrap_or_default();
            let (boundary, holes) = rings.split_first().ok_or_else(|| CliError::usage("a field needs a boundary"))?;
            let parts = w::field(&place(&common), &common.ns, &common.id, boundary, holes, color(&common.color)?);
            send_all(ctx, &Common { color: None, ..common.clone() }, "field", parts)
        }
        Cmd::Headland { common, points } => {
            let rings = groups(&points)?.into_iter().next().unwrap_or_default();
            if rings.len() != 2 {
                return Err(CliError::usage("a headland is OUTER / INNER"));
            }
            send(ctx, restyle(w::headland(&place(&common), &common.ns, &common.id, &rings[0], &rings[1]), &common)?)
        }
        Cmd::Swaths { common, points, swath_width, done } => {
            let lines = groups(&points)?.into_iter().next().unwrap_or_default();
            send_all(ctx, &Common { color: None, ..common.clone() }, "swaths", w::swaths_colored(&place(&common), &common.ns, &common.id, &lines, swath_width, done, world_color(&common)?))
        }
        Cmd::Turns { common, points, swath_width } => {
            let paths = groups(&points)?.into_iter().next().unwrap_or_default();
            send(ctx, restyle(w::turns(&place(&common), &common.ns, &common.id, &paths, swath_width), &common)?)
        }
        Cmd::AbLine { common, a, b, extend } => {
            send_all(ctx, &common, "ab line", w::ab_line(&place(&common), &common.ns, &common.id, point(&a)?, point(&b)?, extend))
        }
        Cmd::Coverage { common, cols, rows, cell, counts } => {
            let counts: Vec<u8> = numbers(&counts)?.into_iter().map(|c| c as u8).collect();
            let e = w::coverage(&place(&common), &common.ns, &common.id, cols, rows, cell, entity_pose(&common), &counts);
            send(ctx, restyle(e, &common)?)
        }
        Cmd::Implement { common, implement_width, depth, sections } => {
            let on: Vec<bool> = sections.split(',').map(|s| s.trim() == "1" || s.trim() == "on").collect();
            send(ctx, restyle(w::implement(&place(&common), &common.ns, &common.id, implement_width, depth, &on), &Common { color: None, ..common.clone() })?)
        }
        Cmd::Zone { common, points, kind } => {
            let rings = groups(&points)?.into_iter().next().unwrap_or_default();
            let kind = Zone::parse(&kind).ok_or_else(|| CliError::usage(format!("unknown zone `{kind}`")))?;
            send(ctx, restyle(w::zone(&place(&common), &common.ns, &common.id, kind, &rings), &common)?)
        }
        Cmd::Wrench { common, force, torque, scale } => {
            let parts = w::wrench(&place(&common), &common.ns, &common.id, entity_pose(&common), point(&force)?, point(&torque)?, scale);
            send_all(ctx, &Common { color: None, ..common.clone() }, "wrench", parts)
        }
        Cmd::Contact { common, point: p, normal, force } => {
            let parts = w::contact(&place(&common), &common.ns, &common.id, point(&p)?, point(&normal)?, force, 0.05);
            send_all(ctx, &common, "contact", parts)
        }
        Cmd::Ghost { common, machine, poses } => {
            let poses = tokens(&poses).map(pose).collect::<Result<Vec<_>>>()?;
            let e = w::ghost_trail(&place(&common), &common.ns, &common.id, &machine, &poses, &[], &[], 1, color(&common.color)?);
            send(ctx, restyle(e, &common)?)
        }
        Cmd::Panel { common, kind, title, unit, text, rows, min, max, extend } => {
            let kind_id = panel_kind::parse(&kind).ok_or_else(|| CliError::usage(format!("unknown panel `{kind}`")))?;
            let values = common.values.as_deref().map(numbers).transpose()?.unwrap_or_default();
            let mut e = DrawEntity::panel(&common.ns, &common.id, kind_id, &values)
                .in_frame(&common.frame)
                .with_lifetime(common.lifetime)
                .with_color(color(&common.color)?)
                .with_prop("title", &title)
                .with_prop("unit", &unit)
                .with_prop("min", &min.to_string())
                .with_prop("max", &max.to_string());
            let p = entity_pose(&common);
            e = e.with_pose(p.position, p.rotation);
            if !common.labels.is_empty() {
                e = e.with_labels(&common.labels);
            }
            if let Some(t) = text {
                e = e.with_prop("text", &t);
            }
            if !rows.is_empty() {
                e = e.with_prop("rows", &rows.join("\n"));
            }
            if kind_id == panel_kind::LEGEND {
                let map = common.colormap.as_deref().and_then(colormap::parse).unwrap_or(colormap::TURBO);
                let r = common.range.clone().unwrap_or(vec![min, max]);
                e = e.with_colormap(map, r[0], r[1]);
            }
            if let Some(keep) = extend {
                e.op = gearbox_api::draw_op::EXTEND;
                e = e.with_prop("max_points", &keep.to_string());
            }
            send(ctx, restyle(e, &common)?)
        }
        Cmd::Badge { ns, id, machine, text, color: c, height } => {
            send(ctx, w::badge(&ns, &id, &machine, &text, color(&Some(c))?, height))
        }
        Cmd::Graph { common, nodes, edges } => {
            let nodes = points_of(&nodes)?;
            let edges = edges
                .iter()
                .map(|e| {
                    let v = numbers(e)?;
                    if v.len() != 2 {
                        return Err(CliError::usage(format!("`{e}` is not A,B")));
                    }
                    Ok((v[0] as usize, v[1] as usize))
                })
                .collect::<Result<Vec<_>>>()?;
            send_all(ctx, &common, "graph", w::graph(&place(&common), &common.ns, &common.id, &nodes, &edges, &common.labels))
        }
        Cmd::Step { caption, full } => {
            let id = format!("step{}", std::process::id());
            let e = DrawEntity::interact("gearbox/interact", &id, interact_kind::STEP).with_prop("text", &caption).with_prop("full", if full { "1" } else { "0" });
            check(ctx.client()?.draw(&e)?, "step")?;
            let ev = await_event(ctx, "gearbox/interact", &id, &["step"], None)?;
            ctx.done(&id, "stepped", || json!({ "stepped": ev.is_some() }));
            Ok(())
        }
        Cmd::Pick { pose: want_pose, frame, prompt } => {
            let (kind, what, id) = if want_pose { (interact_kind::PICK_POSE, "pose", "pose") } else { (interact_kind::PICK_POINT, "point", "point") };
            let prompt = if prompt.is_empty() { if want_pose { "Press and drag for the heading".to_string() } else { "Click a point".to_string() } } else { prompt };
            let e = DrawEntity::interact("gearbox/interact", id, kind).in_frame(&frame).with_prop("text", &prompt);
            check(ctx.client()?.draw(&e)?, "pick")?;
            let Some(ev) = await_event(ctx, "gearbox/interact", id, &[what], None)? else {
                return Err(CliError::usage("nothing picked"));
            };
            let yaw = ev.props().get_f64("yaw").unwrap_or(0.0);
            ctx.done(&format!("{:.3},{:.3},{:.3}", ev.x, ev.y, ev.z), &format!("picked {:.3}, {:.3}, {:.3}{}", ev.x, ev.y, ev.z, if want_pose { format!(" yaw {yaw:.3}") } else { String::new() }), || {
                json!({ "x": ev.x, "y": ev.y, "z": ev.z, "yaw": yaw })
            });
            Ok(())
        }
        Cmd::Handle { common, controls, menu, watch } => {
            let mut e = entity(draw_kind::INTERACT, &common)?.with_sub(interact_kind::HANDLE).with_prop("controls", &controls);
            e = e.without_flags(draw_flag::DRAPE);
            if let Some(m) = &menu {
                e = e.with_prop("menu", m);
            }
            check(ctx.client()?.draw(&e)?, "handle")?;
            let client = ctx.client()?;
            let mut sub = client.events()?;
            let started = Instant::now();
            while started.elapsed().as_secs_f64() < watch {
                let Some(ev) = next_sample::<SceneEvent>(&mut sub, Duration::from_millis(500))? else {
                    continue;
                };
                let p = ev.props();
                if ev.kind == event_kind::DRAW && p.get("ns").as_deref() == Some(common.ns.as_str()) && p.get("id").as_deref() == Some(common.id.as_str()) {
                    println!("{:12} ({:.3}, {:.3}, {:.3}) {}", p.get("what").unwrap_or_default(), ev.x, ev.y, ev.z, p.get("menu").unwrap_or_default());
                }
            }
            Ok(())
        }
        Cmd::Mesh { common, asset, scale } => {
            let e = entity(draw_kind::MESH, &common)?.with_prop("asset", &asset).with_scale([scale; 3]);
            send(ctx, e)
        }
        Cmd::Voxels { common, centres, size, holo } => {
            let centres = points_of(&centres)?;
            let values = common.values.as_deref().map(numbers).transpose()?.unwrap_or_default();
            let map = common.colormap.as_deref().and_then(colormap::parse).unwrap_or(colormap::TURBO);
            send(ctx, restyle(w::voxels(&place(&common), &common.ns, &common.id, &centres, size, &values, map, holo), &common)?)
        }
        Cmd::TerrainMarks { common, points, glyph, size } => {
            let glyph = terrain_glyph(&glyph)?;
            let parts = w::terrain_marks(&place(&common), &common.ns, &common.id, &points_of(&points)?, glyph, size, world_color(&common)?);
            send_world(ctx, &common, "terrain-marks", parts)
        }
        Cmd::RouteStations { common, points, interval, size } => {
            let parts = w::route_stations(&place(&common), &common.ns, &common.id, &points_of(&points)?, interval, size, world_color(&common)?);
            send_world(ctx, &common, "route-stations", parts)
        }
        Cmd::Destination { common, radius, text } => {
            let parts = w::destination(&place(&common), &common.ns, &common.id, entity_pose(&common), radius, &text, world_color(&common)?);
            send_world(ctx, &common, "destination", parts)
        }
        Cmd::MachineRing { common, radius, mark, height } => {
            let parts = machine_ring_entities(&common, radius, &mark, height)?;
            send_world(ctx, &common, "machine-ring", parts)
        }
        Cmd::ClearanceBox { common, size, text } => {
            let parts = w::clearance_box(&place(&common), &common.ns, &common.id, entity_pose(&common), world_size(&size)?, world_color(&common)?, &text);
            send_world(ctx, &common, "clearance-box", parts)
        }
        Cmd::FieldVolume { common, size, grid_spacing } => {
            let parts = w::field_volume(&place(&common), &common.ns, &common.id, entity_pose(&common), world_size(&size)?, grid_spacing, world_color(&common)?);
            send_world(ctx, &common, "field-volume", parts)
        }
        Cmd::Compass { common, at, size } => {
            let tint = if common.color.is_some() { color(&common.color)? } else { gearbox_api::draw_color::WHITE };
            send_all(ctx, &Common { color: None, ..common.clone() }, "compass", w::compass_colored(&place(&common), &common.ns, &common.id, point(&at)?, size, tint))
        }
        Cmd::ScaleBar { common, at, length } => {
            send(ctx, restyle(w::scale_bar(&place(&common), &common.ns, &common.id, point(&at)?, length), &common)?)
        }
        Cmd::Callout { common, anchor, text, offset } => {
            let parts = w::callout(&place(&common), &common.ns, &common.id, point(&anchor)?, point(&offset)?, &text, color(&common.color)?);
            send_all(ctx, &Common { color: None, ..common.clone() }, "callout", parts)
        }
        Cmd::Angle { common, centre, from, to, radius } => {
            let tint = if common.color.is_some() { color(&common.color)? } else { gearbox_api::draw_color::YELLOW };
            send_all(ctx, &Common { color: None, ..common.clone() }, "angle", w::angle_colored(&place(&common), &common.ns, &common.id, point(&centre)?, from, to, radius, tint))
        }
        Cmd::Tf { machine, off } => {
            let client = ctx.client()?;
            let status = if off { client.draw_delete("gearbox/tf", &machine)? } else { client.draw(&w::tf_tree("gearbox/tf", &machine, &machine))? };
            check(status, "tf")?;
            ctx.done(&machine, &format!("tf tree {} for {machine}", if off { "off" } else { "on" }), || json!({ "machine": machine, "on": !off }));
            Ok(())
        }
        Cmd::Delete { ns, id } => {
            let status = check(ctx.client()?.draw_delete(&ns, &id)?, &format!("delete draw {ns}/{id}"))?;
            let touched = status.props().get("touched").unwrap_or_default();
            ctx.done(&id, &format!("deleted {ns}/{id} ({touched})"), || json!({ "ns": ns, "id": id, "touched": touched }));
            Ok(())
        }
        Cmd::Clear { prefix } => {
            let prefix = prefix.unwrap_or_default();
            let status = check(ctx.client()?.draw_clear(&prefix)?, "clear draws")?;
            let touched = status.props().get("touched").unwrap_or_default();
            let what = if prefix.is_empty() { "all draws".to_string() } else { prefix.clone() };
            ctx.done(&touched, &format!("cleared {what}: {touched} entities"), || json!({ "prefix": prefix, "removed": touched }));
            Ok(())
        }
        Cmd::Data { cmd } => return super::draw_data::run(ctx, cmd),
        Cmd::List { prefix } => {
            let listed = ctx.client()?.draw_list(&prefix.unwrap_or_default())?;
            if ctx.json {
                let rows: Vec<_> = listed
                    .iter()
                    .map(|e| {
                        let p = e.props();
                        json!({
                            "ns": e.ns(), "id": e.id(), "kind": draw_kind::name(e.kind),
                            "frame": p.get("frame").unwrap_or_default(),
                            "points": p.get("points").unwrap_or_default(),
                            "instances": p.get("instances").unwrap_or_default(),
                            "style": draw_style::name(e.style),
                            "blur": e.prop_f32("blur"),
                            "lifetime_s": e.lifetime_s,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_default());
            } else if ctx.quiet {
                for e in &listed {
                    println!("{}/{}", e.ns(), e.id());
                }
            } else {
                let mut t = Table::new(&["NS", "ID", "KIND", "STYLE", "FRAME", "POINTS", "INSTANCES", "LIFETIME"]);
                for e in &listed {
                    let p = e.props();
                    t.row(vec![
                        e.ns(),
                        e.id(),
                        draw_kind::name(e.kind).to_string(),
                        draw_style::name(e.style).to_string(),
                        p.get("frame").filter(|f| !f.is_empty()).unwrap_or_else(|| "world".into()),
                        p.get("points").unwrap_or_default(),
                        p.get("instances").unwrap_or_default(),
                        if e.lifetime_s > 0.0 { format!("{:.1}s", e.lifetime_s) } else { "-".into() },
                    ]);
                }
                if t.is_empty() {
                    println!("no draws");
                } else {
                    t.print();
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Parser, Debug)]
    struct DrawCli {
        #[command(flatten)]
        draw: Args,
    }

    const WORLD_CASES: [&[&str]; 5] = [
        &["terrain-marks", "contract", "terrain", "1,2,3"],
        &["route-stations", "contract", "route", "0,0,0;2,0,0"],
        &["destination", "contract", "destination"],
        &["clearance-box", "contract", "clearance"],
        &["field-volume", "contract", "field"],
    ];

    fn parse(args: &[&str]) -> std::result::Result<Cmd, clap::Error> {
        DrawCli::try_parse_from(std::iter::once("draw-test").chain(args.iter().copied())).map(|cli| cli.draw.cmd)
    }

    #[test]
    fn sector_builders_default_to_zero_inner_extent_only() {
        let Cmd::Sweep { common, .. } = parse(&["sweep", "n", "s"]).unwrap() else { panic!() };
        assert_eq!(shape_entity(&common, shape_kind::SECTOR).unwrap().sz, 0.0);
        assert_eq!(shape_entity(&common, shape_kind::BOX).unwrap().sz, 1.0);
    }

    #[test]
    fn blur_creation_and_metadata_update_keep_explicit_zero() {
        let Cmd::Sweep { common, .. } = parse(&["sweep", "n", "s", "--blur", "0.5"]).unwrap() else { panic!() };
        assert_eq!(entity(draw_kind::SHAPES, &common).unwrap().prop_f32("blur"), Some(0.5));
        let Cmd::Update { ns, id, changes } = parse(&["update", "n", "s", "--blur", "0"]).unwrap() else { panic!() };
        let update = changes.entity(&ns, &id).unwrap();
        assert_eq!(update.prop_f32("blur"), Some(0.0));
        assert_eq!(update.props().get("set").as_deref(), Some("props"));
    }

    fn build(cmd: Cmd) -> Result<(Common, Vec<DrawEntity>)> {
        let (common, entities) = match cmd {
            Cmd::TerrainMarks { common, points, glyph, size } => {
                let glyph = terrain_glyph(&glyph)?;
                let entities = w::terrain_marks(&place(&common), &common.ns, &common.id, &points_of(&points)?, glyph, size, world_color(&common)?);
                (common, entities)
            }
            Cmd::RouteStations { common, points, interval, size } => {
                let entities = w::route_stations(&place(&common), &common.ns, &common.id, &points_of(&points)?, interval, size, world_color(&common)?);
                (common, entities)
            }
            Cmd::Destination { common, radius, text } => {
                let entities = w::destination(&place(&common), &common.ns, &common.id, entity_pose(&common), radius, &text, world_color(&common)?);
                (common, entities)
            }
            Cmd::ClearanceBox { common, size, text } => {
                let entities = w::clearance_box(&place(&common), &common.ns, &common.id, entity_pose(&common), world_size(&size)?, world_color(&common)?, &text);
                (common, entities)
            }
            Cmd::FieldVolume { common, size, grid_spacing } => {
                let entities = w::field_volume(&place(&common), &common.ns, &common.id, entity_pose(&common), world_size(&size)?, grid_spacing, world_color(&common)?);
                (common, entities)
            }
            _ => panic!("expected a world drawing command"),
        };
        let entities = world_entities(&common, entities)?;
        Ok((common, entities))
    }

    #[test]
    fn machine_ring_takes_its_mark_tint_and_pose() {
        let ring = |args: &[&str]| {
            let Cmd::MachineRing { common, radius, mark, height } = parse(args).unwrap() else { panic!("machine-ring command") };
            machine_ring_entities(&common, radius, &mark, height).and_then(|parts| world_entities(&common, parts))
        };
        let parts = ring(&["machine-ring", "n", "tractor", "--mark", "following", "--radius", "3", "--pose", "1", "2", "0"]).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!((parts[0].ns(), parts[0].id()), ("n/tractor".to_string(), "ring".to_string()));
        assert_eq!(parts[0].color, w::RingMark::Following.tint());
        assert_eq!((parts[0].px, parts[0].py, parts[0].width), (1.0, 2.0, 6.0));
        assert_eq!(parts[0].props().get("mark").as_deref(), Some("following"));
        let selected = ring(&["machine-ring", "n", "tractor", "--color", "red"]).unwrap();
        assert_eq!(selected[0].color, gearbox_api::draw_color::RED);
        assert!(ring(&["machine-ring", "n", "tractor", "--mark", "spinning"]).is_err());
    }

    fn prepare(args: &[&str]) -> Vec<DrawEntity> {
        build(parse(args).expect("valid CLI syntax")).expect("valid world geometry").1
    }

    fn update(args: &[&str]) -> Result<DrawEntity> {
        let args: Vec<_> = ["update", "n", "id"].into_iter().chain(args.iter().copied()).collect();
        let Cmd::Update { ns, id, changes } = parse(&args).unwrap() else { unreachable!() };
        changes.entity(&ns, &id)
    }

    #[test]
    fn updates_select_only_the_supplied_field() {
        for (option, mask) in [
            ("glow", "glow"), ("pulse", "pulse_hz"), ("flow", "flow_speed"),
            ("spacing", "dash"), ("beam", "beam_height"), ("fade", "fade"),
            ("urgency", "urgency"), ("opacity", "opacity"), ("vmin", "vmin"), ("vmax", "vmax"),
            ("flags", "flags"), ("width", "width"), ("lifetime", "lifetime"),
        ] {
            let e = update(&[&format!("--{option}"), "0"]).unwrap();
            assert_eq!(e.props().get("set").as_deref(), Some(mask), "option {option}");
        }
        let e = update(&["--glow", "-1", "--flow", "2.5", "--pulse", "3"]).unwrap();
        assert_eq!(e.props().get("set").as_deref(), Some("glow,pulse_hz,flow_speed"));
        assert_eq!((e.glow, e.flow_speed, e.pulse_hz), (-1.0, 2.5, 3.0));
        let e = update(&["--colormap", "viridis"]).unwrap();
        assert_eq!(e.props().get("set").as_deref(), Some("palette"));
        assert_eq!(e.colormap, colormap::VIRIDIS);
        let e = update(&["--color", "#00000000", "--color2", "#ff000080"]).unwrap();
        assert_eq!(e.props().get("set").as_deref(), Some("tint,color2"));
        assert_eq!((e.color, e.color2), (1, 0x800000ff));
    }

    #[test]
    fn update_metadata_does_not_reset_headers_and_rejects_empty_or_reserved_input() {
        let e = update(&["--panel-finish", "white", "--panel-theme", "light", "--prop", "label_detail=42=answer"]).unwrap();
        assert_eq!(e.props().get("set").as_deref(), Some("props"));
        assert_eq!(e.props().get("panel_finish").as_deref(), Some("white"));
        assert_eq!(e.props().get("panel_theme").as_deref(), Some("light"));
        assert_eq!(e.props().get("label_detail").as_deref(), Some("42=answer"));
        for args in [&[][..], &["--prop", "set="], &["--prop", "ns=bad"], &["--prop", "id=bad"], &["--prop", "=bad"], &["--prop", "missing-value"]] {
            assert!(update(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn clap_definition_and_five_world_defaults_are_valid() {
        DrawCli::command().debug_assert();
        let child_ids: [&[&str]; 5] = [
            &["marks"], &["route", "stations"], &["rings", "marker", "leader", "label"],
            &["brackets", "label"], &["volume"],
        ];
        for (args, ids) in WORLD_CASES.into_iter().zip(child_ids) {
            let (common, entities) = build(parse(args).unwrap()).unwrap();
            assert_eq!(common.frame, "world");
            assert_eq!(common.lifetime, 0.0);
            assert_eq!(common.style, "auto");
            assert!(common.glow.is_none());
            assert_eq!(entities.iter().map(|e| e.id()).collect::<Vec<_>>(), ids);
            for e in entities.iter().filter(|e| e.op != gearbox_api::draw_op::DELETE) {
                assert_eq!(e.ns(), format!("{}/{}", common.ns, common.id));
                assert_eq!(e.color, gearbox_api::draw_color::PLAN);
                assert_ne!(e.flags & draw_flag::DRAPE, 0);
                assert_eq!(e.flags & (draw_flag::ON_TOP | draw_flag::ADDITIVE), 0);
                if e.kind == draw_kind::LINES {
                    assert_eq!(e.style, draw_style::PLAIN);
                    assert_eq!(e.glow, 0.0);
                }
            }
        }
    }

    #[test]
    fn world_common_flags_and_negative_controls_reach_entities() {
        let options = [
            "--frame", "robot/link", "--lifetime", "7", "--pose", "-10", "-20", "-30", "-0.5",
            "--width", "-2.5", "--glow", "-1", "--pulse", "-2", "--flow", "-3", "--spacing", "-4",
            "--beam", "-5", "--fade", "0.25", "--urgency", "0.4", "--opacity", "0.8", "--snapshot", "--flat", "--additive",
            "--on-top", "--no-drape", "--style", "plain", "--color", "#00000000",
        ];
        for case in WORLD_CASES {
            let mut args = case.to_vec();
            args.extend(options);
            let entities = prepare(&args);
            let flags = draw_flag::SNAPSHOT | draw_flag::FLAT | draw_flag::ADDITIVE | draw_flag::ON_TOP;
            let live: Vec<_> = entities.iter().filter(|e| e.op != gearbox_api::draw_op::DELETE).collect();
            assert!(!live.is_empty());
            for e in live {
                assert_eq!(e.props().get("frame").as_deref(), Some("robot/link"));
                assert_eq!(e.lifetime_s, 7.0);
                assert_eq!([e.px, e.py, e.pz], [-10.0, -20.0, -30.0]);
                assert!((e.qw - (-0.25_f64).cos()).abs() < 1e-12);
                assert!((e.qz - (-0.25_f64).sin()).abs() < 1e-12);
                assert_eq!([e.qx, e.qy], [0.0, 0.0]);
                assert_eq!(e.width, -2.5);
                assert_eq!([e.glow, e.pulse_hz, e.flow_speed, e.dash, e.beam_height], [-1.0, -2.0, -3.0, -4.0, -5.0]);
                assert_eq!([e.fade, e.urgency, e.opacity], [0.25, 0.4, 0.8]);
                assert_eq!(e.flags & flags, flags);
                assert_eq!(e.flags & draw_flag::DRAPE, 0);
                assert_eq!(e.style, draw_style::PLAIN);
                assert_eq!(e.color, 1);
            }
        }
    }

    #[test]
    fn existing_composites_keep_explicit_effects_and_flags() {
        let Cmd::Waypoints { common, .. } = parse(&["waypoints", "n", "route", "0,0,0;2,0,0", "--glow", "-1", "--pulse", "-2",
            "--flow", "-3", "--width", "-2.5", "--snapshot", "--flat", "--additive", "--on-top"]).unwrap() else {
            panic!("waypoints command");
        };
        let entities = w::waypoints(&place(&common), "n", "route", &[[0.0; 3], [2.0, 0.0, 0.0]], 0);
        for e in entities {
            let e = restyle(e, &common).unwrap();
            assert_eq!([e.glow, e.pulse_hz, e.flow_speed, e.width], [-1.0, -2.0, -3.0, -2.5]);
            let mask = draw_flag::SNAPSHOT | draw_flag::FLAT | draw_flag::ADDITIVE | draw_flag::ON_TOP;
            assert_eq!(e.flags & mask, mask);
        }
    }

    #[test]
    fn waypoint_cli_color_reaches_generated_items_without_replacing_state_colors() {
        for (argument, expected) in [("magenta", gearbox_api::draw_color::MAGENTA), ("#00000000", 1), ("#00ff0080", 0x8000_ff00)] {
            let Cmd::Waypoints { common, points, current } = parse(&["waypoints", "n", "route", "0,0,0;2,0,0;4,0,0",
                "--current", "1", "--color", argument]).unwrap() else { panic!("waypoints command"); };
            let entities = waypoint_entities(&common, &points_of(&points).unwrap(), current).unwrap();
            assert_eq!(entities[0].colors[2], expected);
            assert_eq!(entities[3].color & 0x00ff_ffff, gearbox_api::draw_color::GOAL & 0x00ff_ffff);
            assert_eq!(entities[3].color >> 24, expected >> 24);
        }
        let Cmd::Waypoints { common, points, current } = parse(&["waypoints", "n", "route", "0,0,0;2,0,0"]).unwrap() else {
            panic!("waypoints command");
        };
        assert_eq!(waypoint_entities(&common, &points_of(&points).unwrap(), current).unwrap()[0].colors,
            [gearbox_api::draw_color::PLAN; 2]);
    }

    #[test]
    fn common_tint_preserves_explicit_item_colors_and_scalar_maps() {
        for argument in ["magenta", "#00000000", "#00ff0080"] {
            let Cmd::Destination { common, .. } = parse(&["destination", "n", "fixture", "--color", argument]).unwrap() else {
                panic!("destination command");
            };
            let expected = color(&common.color).unwrap();
            for kind in [draw_kind::LINES, draw_kind::POINTS, draw_kind::ARROWS, draw_kind::SHAPES, draw_kind::TEXT,
                draw_kind::GHOST, draw_kind::PANEL, draw_kind::GRID, draw_kind::AXES] {
                let e = DrawEntity::new(kind, "n", "fixture").with_colors(&[gearbox_api::draw_color::GREEN])
                    .with_values(&[0.25]).with_colormap(colormap::VIRIDIS, 0.0, 1.0);
                let e = restyle(e, &common).unwrap();
                assert_eq!(e.color, expected);
                assert_eq!(e.colors, [gearbox_api::draw_color::GREEN]);
                assert_eq!((e.colormap, e.vmin, e.vmax), (colormap::VIRIDIS, 0.0, 1.0));
                assert_eq!(e.values, [0.25]);
            }
        }
        assert_eq!(color(&None).unwrap(), 0);
    }

    #[test]
    fn panel_theme_and_glow_reach_the_entity_without_changing_values() {
        for (theme, finish) in ["dark", "light"].into_iter().flat_map(|theme| ["color", "white", "black"].map(|finish| (theme, finish))) {
            let Cmd::Panel { common, .. } = parse(&["panel", "n", "chart", "--panel-theme", theme, "--panel-finish", finish, "--glow=-1"]).unwrap() else {
                panic!("panel command");
            };
            let e = restyle(DrawEntity::panel("n", "chart", panel_kind::BARS, &[0.25, 0.75])
                .with_colors(&[gearbox_api::draw_color::GREEN, gearbox_api::draw_color::MAGENTA]), &common).unwrap();
            assert_eq!(e.props().get("panel_theme").as_deref(), Some(theme));
            assert_eq!(e.props().get("panel_finish").as_deref(), Some(finish));
            assert_eq!(e.glow, -1.0);
            assert_eq!(e.values, [0.25, 0.75]);
            assert_eq!(e.colors, [gearbox_api::draw_color::GREEN, gearbox_api::draw_color::MAGENTA]);
        }
        assert!(parse(&["panel", "n", "chart", "--panel-theme", "unknown"]).is_err());
        assert!(parse(&["panel", "n", "chart", "--panel-finish", "unknown"]).is_err());
    }

    #[test]
    fn annotation_metadata_reaches_primitives_and_composites() {
        let Cmd::Destination { common, .. } = parse(&["destination", "n", "dock", "--label-category", "INSPECTION",
            "--label-detail", "0.4 m", "--label-icon", "measurement", "--label-selected"]).unwrap() else {
            panic!("destination command");
        };
        let primitive = entity(draw_kind::TEXT, &common).unwrap();
        let composite = restyle(DrawEntity::text("n", "label", &[[0.0; 3]], "Supplied"), &common).unwrap();
        for e in [primitive, composite] {
            for (key, value) in [("label_category", "INSPECTION"), ("label_detail", "0.4 m"),
                ("label_icon", "measurement"), ("label_selected", "true")] {
                assert_eq!(e.props().get(key).as_deref(), Some(value));
            }
        }
        let Cmd::Destination { common, .. } = parse(&["destination", "n", "dock", "--label-selected=false"]).unwrap() else {
            panic!("destination command");
        };
        assert_eq!(common.label_selected, Some(false));
        assert!(parse(&["destination", "n", "dock", "--label-icon", "inferred-safe"]).is_err());
    }

    #[test]
    fn world_frames_and_three_component_pose_keep_drape_contract() {
        for (frame, draped) in [("world", true), ("robot/odom", true), ("robot/link", false)] {
            for case in WORLD_CASES {
                let mut args = case.to_vec();
                args.extend(["--frame", frame, "--pose", "-1", "-2", "-3"]);
                for e in prepare(&args).iter().filter(|e| e.op != gearbox_api::draw_op::DELETE) {
                    assert_eq!(e.flags & draw_flag::DRAPE != 0, draped);
                    assert_eq!([e.px, e.py, e.pz], [-1.0, -2.0, -3.0]);
                    assert_eq!([e.qw, e.qx, e.qy, e.qz], [1.0, 0.0, 0.0, 0.0]);
                }
                args.push("--no-drape");
                assert!(prepare(&args).iter().all(|e| e.flags & draw_flag::DRAPE == 0));
            }
        }
    }

    #[test]
    fn route_interval_and_field_grid_spacing_are_not_effect_spacing() {
        let route = prepare(&["route-stations", "n", "r", "0,0,0;0,0,3;4,0,3", "--interval", "2", "--size", "0.4", "--spacing", "0.75"]);
        assert_eq!(route[0].point_count(), 4);
        assert_eq!(route[1].point_count(), 24);
        let centres: Vec<_> = route[1].point_list().chunks_exact(6).map(|p| std::array::from_fn::<_, 3, _>(|i| (p[0][i] + p[1][i]) * 0.5)).collect();
        assert_eq!(centres, [[0.0, 0.0, 0.0], [0.0, 0.0, 2.0], [1.0, 0.0, 3.0], [3.0, 0.0, 3.0]]);
        assert!(route.iter().all(|e| e.dash == 0.75));
        let args = ["field-volume", "n", "f", "--size", "2,4,6", "--grid-spacing", "0.5"];
        let field = prepare(&args);
        assert_eq!(field[0].style, draw_style::FIELD);
        assert_eq!(field[0].dash, 0.5);
        assert_ne!(field[0].flags & draw_flag::DASHED, 0);
        assert_eq!(field[0].instance_list()[0].size, [2.0, 4.0, 6.0]);
        let mut overridden = args.to_vec();
        overridden.extend(["--spacing", "0.75"]);
        assert_eq!(prepare(&overridden)[0].dash, 0.75);
    }

    #[test]
    fn terrain_glyphs_and_negative_points_parse_without_option_collisions() {
        for (glyph, points) in [("square", 16), ("cross", 8), ("triangle", 12), ("diamond", 16)] {
            let entities = prepare(&["terrain-marks", "n", "t", "--glyph", glyph, "--size", "2", "--", "-1,-2,-3;2,3,4"]);
            assert_eq!(entities[0].point_count(), points);
            assert!(entities[0].point_list().iter().any(|p| p[2] == -3.0));
        }
    }

    #[test]
    fn world_labels_and_explicit_zero_glow_are_forwarded() {
        for name in ["destination", "clearance-box"] {
            let entities = prepare(&[name, "n", "a", "--text", "Dock", "--glow", "0"]);
            let label = entities.iter().find(|e| e.id() == "label").unwrap();
            assert_ne!(label.op, gearbox_api::draw_op::DELETE);
            assert_eq!(label.props().get("text").as_deref(), Some("Dock"));
            assert!(entities.iter().all(|e| e.glow == 0.0));
        }
        for case in WORLD_CASES {
            let mut args = case.to_vec();
            args.extend(["--style", "field", "--glow", "0"]);
            assert!(prepare(&args).iter().all(|e| e.style == draw_style::FIELD && e.glow == 0.0));
        }
    }

    #[test]
    fn invalid_world_values_fail_pure_dispatch_validation() {
        for glyph in ["Square", "safe", ""] {
            let cmd = parse(&["terrain-marks", "n", "a", "0,0,0", "--glyph", glyph]).unwrap();
            let error = build(cmd).err().expect("unknown glyph rejected before sending");
            assert_eq!(error.code, crate::error::EXIT_USAGE);
            assert!(error.message.contains("unknown terrain glyph"));
        }
        for args in [
            &["clearance-box", "n", "a", "--size", "1,2"][..],
            &["field-volume", "n", "a", "--size", "bad"][..],
            &["destination", "n", "a", "--style", "invalid"][..],
            &["destination", "n", "a", "--color", "invalid"][..],
        ] {
            assert_eq!(build(parse(args).unwrap()).err().unwrap().code, crate::error::EXIT_USAGE);
        }
    }

    #[test]
    fn missing_duplicate_and_malformed_arguments_are_clap_errors() {
        for args in [
            &["terrain-marks", "n", "a"][..],
            &["route-stations", "n", "a", "0,0,0", "--interval", "1", "--interval", "2"][..],
            &["field-volume", "n", "a", "--grid-spacing", "bad"][..],
            &["destination", "n", "a", "--pose", "1", "2"][..],
            &["clearance-box", "n", "a", "--glow", "bad"][..],
        ] {
            assert!(parse(args).is_err(), "accepted invalid arguments: {args:?}");
        }
    }
}
