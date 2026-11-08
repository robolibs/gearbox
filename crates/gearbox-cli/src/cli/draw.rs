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
use gearbox_api::wire::draw_widgets::{self as w, Detection3d, DetectionStyle, Glyph, Place, Pose, Zone};
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
    /// Colour: #rgb, #rrggbb, #rrggbbaa or a name (rviz names on Okabe-Ito, or a role: plan, goal, sensor, ...)
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
    /// Look: auto, plain, ribbon_flow, tube_pulse, hairline, light_wall, beacon, holo, shield, alert, orb, decal, arrow_glow, point_glow, label, heat, splat, sweep, ghost, ruler
    #[arg(long, default_value = "auto")]
    style: String,
    /// Brightness of the style's light (1 usual); the effects below take the style's default when left out and switch off below 0
    #[arg(long, allow_negative_numbers = true)]
    glow: Option<f32>,
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
        #[arg(long, default_value_t = 14.0)]
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
    /// Numbered waypoints: current a beacon, the rest holo pillars, joined by a flowing ribbon
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
        #[arg(long, num_args = 3..=4, value_names = ["X", "Y", "Z", "YAW"], allow_negative_numbers = true)]
        pose: Option<Vec<f64>>,
        #[arg(long)]
        color: Option<String>,
        #[arg(long, allow_negative_numbers = true)]
        width: Option<f32>,
        #[arg(long)]
        style: Option<String>,
        #[arg(long, allow_negative_numbers = true)]
        glow: Option<f32>,
        #[arg(long)]
        lifetime: Option<f64>,
        #[arg(long)]
        urgency: Option<f32>,
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
        Some(t) => parse_color(t).ok_or_else(|| CliError::usage(format!("bad colour `{t}`"))),
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
fn restyle(e: DrawEntity, c: &Common) -> Result<DrawEntity> {
    let mut e = e;
    if c.color.is_some() && !matches!(e.op, gearbox_api::draw_op::DELETE) {
        e = e.with_color(color(&c.color)?);
    }
    if let Some(wd) = c.width {
        e = e.with_width(wd);
    }
    if c.style != "auto" {
        e = e.with_style(draw_style::parse(&c.style).ok_or_else(|| CliError::usage(format!("unknown style `{}`", c.style)))?);
    }
    for (v, slot) in [(c.glow, &mut e.glow), (c.pulse, &mut e.pulse_hz), (c.beam, &mut e.beam_height), (c.fade, &mut e.fade), (c.urgency, &mut e.urgency)] {
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
    Ok(e)
}

fn entity(kind: u32, c: &Common) -> Result<DrawEntity> {
    let mut e = DrawEntity::new(kind, &c.ns, &c.id)
        .in_frame(&c.frame)
        .with_color(color(&c.color)?)
        .with_width(c.width.unwrap_or(0.0))
        .with_lifetime(c.lifetime);
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
        .with_urgency(c.urgency.unwrap_or(0.0));
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
    Ok(e)
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
    let client = ctx.client()?;
    let mut parts = Vec::new();
    for e in entities {
        let e = restyle(e, c)?;
        parts.push(format!("{}/{}", e.ns(), e.id()));
        check(client.draw(&e)?, &format!("draw {}/{}", e.ns(), e.id()))?;
    }
    let root = format!("{}/{}", c.ns, c.id);
    ctx.done(&root, &format!("drew {what} {root} ({} entities)", parts.len()), || json!({ "widget": what, "root": root, "parts": parts }));
    Ok(())
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
            let e = entity(draw_kind::SHAPES, &common)?
                .with_sub(sub)
                .with_fill(fill)
                .with_instances(&instances)
                .with_color2(color(&outline)?);
            send(ctx, e)
        }
        Cmd::Text { common, points, text, size } => {
            let pts = points_of(&points)?;
            let mut e = entity(draw_kind::TEXT, &common)?.with_points(&pts).with_prop("text", &text).with_width(-size.abs());
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
            let mut c = common.clone();
            c.color = None;
            send_all(ctx, &c, "frame", w::frame(&place(&common), &common.ns, &common.id, entity_pose(&common), parent, length, label.as_deref()))
        }
        Cmd::PoseArray { common, poses, glyph, length } => {
            let glyph = Glyph::parse(&glyph).ok_or_else(|| CliError::usage(format!("unknown glyph `{glyph}`")))?;
            let poses = tokens(&poses).map(pose).collect::<Result<Vec<_>>>()?;
            let e = w::pose_array(&place(&common), &common.ns, &common.id, &poses, glyph, length);
            send(ctx, restyle(e, &common)?)
        }
        Cmd::Waypoints { common, points, current } => {
            let pts = points_of(&points)?;
            send_all(ctx, &Common { color: None, ..common.clone() }, "waypoints", w::waypoints(&place(&common), &common.ns, &common.id, &pts, current))
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
        Cmd::Update { ns, id, pose, color: c, width, style, glow, lifetime, urgency } => {
            let mut e = DrawEntity::update(&ns, &id);
            let mut groups = Vec::new();
            if let Some(p) = pose {
                e = e.at_yaw([p[0], p[1], p[2]], p.get(3).copied().unwrap_or(0.0));
                groups.push("pose");
            }
            if c.is_some() {
                e = e.with_color(color(&c)?);
                groups.push("color");
            }
            if let Some(wd) = width {
                e = e.with_width(wd);
                groups.push("width");
            }
            if let Some(s) = style {
                e = e.with_style(draw_style::parse(&s).ok_or_else(|| CliError::usage(format!("unknown style `{s}`")))?);
                groups.push("style");
            }
            if glow.is_some() || urgency.is_some() {
                e = e.with_glow(glow.unwrap_or(0.0)).with_urgency(urgency.unwrap_or(0.0));
                groups.push("effects");
            }
            if let Some(l) = lifetime {
                e = e.with_lifetime(l);
                groups.push("lifetime");
            }
            check(ctx.client()?.draw_raw(&e.updating(&groups))?, &format!("update draw {ns}/{id}"))?;
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
            let e = entity(draw_kind::SHAPES, &common)?
                .with_sub(shape_kind::SECTOR)
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
            let e = w::particles(&place(&common), &common.ns, &common.id, &poses, &weights, min, max, glyph);
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
            send_all(ctx, &Common { color: None, ..common.clone() }, "swaths", w::swaths(&place(&common), &common.ns, &common.id, &lines, swath_width, done))
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
            send(ctx, e)
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
        Cmd::Compass { common, at, size } => {
            send_all(ctx, &Common { color: None, ..common.clone() }, "compass", w::compass(&place(&common), &common.ns, &common.id, point(&at)?, size))
        }
        Cmd::ScaleBar { common, at, length } => {
            send(ctx, restyle(w::scale_bar(&place(&common), &common.ns, &common.id, point(&at)?, length), &common)?)
        }
        Cmd::Callout { common, anchor, text, offset } => {
            let parts = w::callout(&place(&common), &common.ns, &common.id, point(&anchor)?, point(&offset)?, &text, color(&common.color)?);
            send_all(ctx, &Common { color: None, ..common.clone() }, "callout", parts)
        }
        Cmd::Angle { common, centre, from, to, radius } => {
            send_all(ctx, &Common { color: None, ..common.clone() }, "angle", w::angle(&place(&common), &common.ns, &common.id, point(&centre)?, from, to, radius))
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
