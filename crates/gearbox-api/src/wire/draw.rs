//! Retained debug draws: one entity per `ns` path and `id`, kept by the host
//! and drawn in the viewer until deleted, cleared or out of lifetime.
//!
//! Coordinates are right-handed Z-up in the entity's frame: `world` is the
//! home site with x east, y north, z up (the axes machines report their pose
//! in), `<machine>/odom` is that machine's own datum, the origin its reported
//! pose is measured from, and `<machine>/<link>` is the link's own frame.

use std::ops::Range;

use super::common::Props;

/// What a message does to the entity it names.
pub mod draw_op {
    /// Replace the entity with this one.
    pub const UPSERT: u32 = 0;
    /// One chunk of an entity sent in parts; chunks share `seq` and the
    /// assembled entity replaces the old one when the `last` chunk lands.
    pub const APPEND: u32 = 1;
    /// Remove the entity `ns` + `id`.
    pub const DELETE: u32 = 2;
    /// Remove every entity whose `ns` is the given path or below it; an
    /// empty path removes all.
    pub const CLEAR: u32 = 3;
    /// Add the message's items to the live entity (its last strip for
    /// lines), creating it when missing; props `max_points` caps its length.
    pub const EXTEND: u32 = 4;
    /// Change the live entity's header (pose, scale, colours, width, style,
    /// effects, flags, colormap, lifetime) and merge its props, keeping its
    /// sections; props `set` lists the header groups to take (`pose`,
    /// `scale`, `color`, `width`, `style`, `effects`, `flags`, `colormap`,
    /// `lifetime`), all of them when absent or empty. Scalar selectors are
    /// `tint` (primary colour), `color2`, `glow`, `pulse_hz`, `flow_speed`,
    /// `dash`, `beam_height`, `fade`, `urgency`, `palette` (colormap ID),
    /// `vmin` and `vmax`. `props` alone merges metadata without changing headers.
    pub const UPDATE: u32 = 5;
    /// Replace items `chunk_offset..chunk_offset + n` of the live entity
    /// with the message's `n`, growing it at the end; a grid takes the
    /// rectangles `x, y, w, h` in `offsets` and their cells, row by row, in
    /// `cells` (the `map_msgs/OccupancyGridUpdate` model).
    pub const PATCH: u32 = 6;

    pub fn name(value: u32) -> &'static str {
        match value {
            UPSERT => "upsert",
            APPEND => "append",
            DELETE => "delete",
            CLEAR => "clear",
            EXTEND => "extend",
            UPDATE => "update",
            PATCH => "patch",
            _ => "unknown",
        }
    }

    pub fn parse(name: &str) -> Option<u32> {
        match name {
            "upsert" | "set" | "" => Some(UPSERT),
            "append" => Some(APPEND),
            "delete" => Some(DELETE),
            "clear" => Some(CLEAR),
            "extend" => Some(EXTEND),
            "update" | "modify" => Some(UPDATE),
            "patch" => Some(PATCH),
            _ => None,
        }
    }
}

/// What an entity draws.
pub mod draw_kind {
    /// Polylines: `points`, strips split by `offsets`, mode in `sub`.
    pub const LINES: u32 = 1;
    /// Spheres or flat discs at `points`, style in `sub`.
    pub const POINTS: u32 = 2;
    /// Arrows: tail and tip pairs in `points`, or along +X of each instance.
    pub const ARROWS: u32 = 3;
    /// Instanced solids, shape in `sub`, fill in `flags`.
    pub const SHAPES: u32 = 4;
    /// A triangle list in `points`, colours per vertex.
    pub const TRIANGLES: u32 = 5;
    /// Text at `points`: `labels` per point, or props `text` for all.
    pub const TEXT: u32 = 6;
    /// An RGB axis triad at the pose or at each instance.
    pub const AXES: u32 = 7;
    /// Polygons with holes: `points` hold every ring, `offsets` where each
    /// ring starts and `rings` how many rings each polygon has (its outline
    /// first, then its holes); one polygon when `rings` is empty. Fill in
    /// `flags`: solid fills only, wire outlines only, both does both.
    /// `colors` and `values` per polygon, props `z_offset` lifts it.
    pub const POLYGON: u32 = 8;
    /// A grid of cells, row by row from the pose (its lower-left corner):
    /// props `cols`, `rows`, `cell` (metres, `cell_y` when not square),
    /// `cells` in the [`grid_encoding`] of `sub` coloured through
    /// `colormap` over `vmin..vmax`, props `unknown` the value drawn as
    /// nothing; `values` an optional height per cell.
    pub const GRID: u32 = 9;
    /// See-through copies of a machine: props `machine`, an instance per
    /// copy, props `joints` (comma separated names) with that many `values`
    /// per copy.
    pub const GHOST: u32 = 10;
    /// An in-world chart at the pose, [`panel_kind`] in `sub`: `values` the
    /// data, `labels` their names, props `title`, `unit`, `min`, `max`.
    pub const PANEL: u32 = 11;
    /// A mesh loaded from props `asset` (an OBJ or STL file the viewer reads),
    /// placed at the pose or at each instance.
    pub const MESH: u32 = 12;
    /// A request for input in the viewer, [`interact_kind`] in `sub`; the
    /// answers come as `draw` scene events carrying the entity's ns and id.
    pub const INTERACT: u32 = 13;

    pub const LAST: u32 = INTERACT;

    const NAMES: [&str; 14] = [
        "unknown", "lines", "points", "arrows", "shapes", "triangles", "text", "axes", "polygon", "grid", "ghost", "panel",
        "mesh", "interact",
    ];

    pub fn name(value: u32) -> &'static str {
        NAMES.get(value as usize).copied().unwrap_or("unknown")
    }

    pub fn parse(name: &str) -> Option<u32> {
        match name {
            "lines" | "line" => Some(LINES),
            "points" | "point" => Some(POINTS),
            "arrows" | "arrow" => Some(ARROWS),
            "shapes" | "shape" => Some(SHAPES),
            "triangles" | "triangle" => Some(TRIANGLES),
            "text" | "label" => Some(TEXT),
            "axes" | "axis" => Some(AXES),
            "polygon" | "polygons" => Some(POLYGON),
            "grid" | "map" => Some(GRID),
            "ghost" | "ghosts" => Some(GHOST),
            "panel" | "chart" => Some(PANEL),
            "mesh" => Some(MESH),
            "interact" => Some(INTERACT),
            _ => None,
        }
    }
}

/// `sub` of [`draw_kind::LINES`].
pub mod line_mode {
    pub const STRIP: u32 = 0;
    /// Each strip closes back to its first point.
    pub const LOOP: u32 = 1;
    /// Independent segments, two points each.
    pub const LIST: u32 = 2;

    pub fn parse(name: &str) -> Option<u32> {
        match name {
            "strip" | "" => Some(STRIP),
            "loop" => Some(LOOP),
            "list" | "segments" => Some(LIST),
            _ => None,
        }
    }
}

/// `sub` of [`draw_kind::POINTS`].
pub mod point_style {
    /// Shaded spheres.
    pub const SPHERE: u32 = 0;
    /// Flat discs facing the camera.
    pub const DISC: u32 = 1;

    pub fn parse(name: &str) -> Option<u32> {
        match name {
            "sphere" | "spheres" | "" => Some(SPHERE),
            "disc" | "discs" | "flat" => Some(DISC),
            _ => None,
        }
    }
}

/// `sub` of [`draw_kind::SHAPES`]. Sizes are full extents along the
/// instance's x, y and z: box edges, ellipsoid diameters, cylinder and cone
/// base diameters and height (cone apex at +z), capsule diameter and total
/// length along z, ellipse diameters in its XY plane.
pub mod shape_kind {
    pub const BOX: u32 = 0;
    pub const ELLIPSOID: u32 = 1;
    pub const CYLINDER: u32 = 2;
    pub const CONE: u32 = 3;
    pub const CAPSULE: u32 = 4;
    /// A flat ellipse or disc in the instance's XY plane.
    pub const ELLIPSE: u32 = 5;
    /// A flat rectangle in the instance's XY plane, x by y.
    pub const RECT: u32 = 6;
    /// A view frustum from the instance origin along +x: size is the
    /// horizontal and vertical field of view (radians) and the range.
    pub const FRUSTUM: u32 = 7;
    /// A flat wedge in the XY plane centred on +x: size is the radius, the
    /// opening angle (radians) and the inner radius.
    pub const SECTOR: u32 = 8;

    const NAMES: [&str; 9] = ["box", "ellipsoid", "cylinder", "cone", "capsule", "ellipse", "rect", "frustum", "sector"];

    pub fn name(value: u32) -> &'static str {
        NAMES.get(value as usize).copied().unwrap_or("unknown")
    }

    pub fn parse(name: &str) -> Option<u32> {
        match name {
            "box" | "cube" | "cuboid" => Some(BOX),
            "ellipsoid" | "sphere" => Some(ELLIPSOID),
            "cylinder" => Some(CYLINDER),
            "cone" => Some(CONE),
            "capsule" => Some(CAPSULE),
            "ellipse" | "disc" | "disk" | "circle" => Some(ELLIPSE),
            "rect" | "rectangle" | "plane" | "quad" => Some(RECT),
            "frustum" | "camera" => Some(FRUSTUM),
            "sector" | "wedge" | "fan" => Some(SECTOR),
            _ => None,
        }
    }
}

/// `sub` of [`draw_kind::GRID`]: how `cells` encode each cell.
pub mod grid_encoding {
    pub const U8: u32 = 0;
    /// Little-endian u16.
    pub const U16: u32 = 1;
    /// Little-endian f32.
    pub const F32: u32 = 2;

    /// Bytes per cell.
    pub fn size(encoding: u32) -> usize {
        match encoding {
            U16 => 2,
            F32 => 4,
            _ => 1,
        }
    }

    pub fn parse(name: &str) -> Option<u32> {
        match name {
            "u8" | "uint8" | "int8" | "i8" | "" => Some(U8),
            "u16" | "uint16" => Some(U16),
            "f32" | "float" | "float32" => Some(F32),
            _ => None,
        }
    }
}

/// `sub` of [`draw_kind::PANEL`].
pub mod panel_kind {
    /// A line over the recent `values`, the newest one written beside it.
    pub const SPARKLINE: u32 = 0;
    /// A bar per value, named by `labels`.
    pub const BARS: u32 = 1;
    /// An arc filled to the first value between props `min` and `max`.
    pub const GAUGE: u32 = 2;
    /// A pill naming a state: props `text`, the colour its tone.
    pub const CHIP: u32 = 3;
    /// Rows of `labels` with their `values`, or props `rows` as
    /// `key=value` lines.
    pub const TABLE: u32 = 4;
    /// A colormap bar from `vmin` to `vmax` in screen space, props `title`.
    pub const LEGEND: u32 = 5;

    const NAMES: [&str; 6] = ["sparkline", "bars", "gauge", "chip", "table", "legend"];

    pub fn name(value: u32) -> &'static str {
        NAMES.get(value as usize).copied().unwrap_or("unknown")
    }

    pub fn parse(name: &str) -> Option<u32> {
        NAMES.iter().position(|n| *n == name).map(|i| i as u32).or(match name {
            "spark" | "line" => Some(SPARKLINE),
            "bar" => Some(BARS),
            "state" => Some(CHIP),
            "kv" => Some(TABLE),
            _ => None,
        })
    }
}

/// `sub` of [`draw_kind::INTERACT`].
pub mod interact_kind {
    /// A button reading props `text` (and `full` for a full step): pressing
    /// it sends `step` and removes the entity.
    pub const STEP: u32 = 0;
    /// The next click on the ground sends `point` with its x, y, z in the
    /// entity's frame and removes the entity.
    pub const PICK_POINT: u32 = 1;
    /// A press on the ground and a drag for the heading sends `pose` with
    /// x, y, z and `yaw`, and removes the entity.
    pub const PICK_POSE: u32 = 2;
    /// A 6-DoF handle at the pose: drags send `pose_update` with x, y, z
    /// and `qw`, `qx`, `qy`, `qz`, a press `mouse_down`, the release
    /// `mouse_up`; props `menu` (comma separated) adds entries sending
    /// `menu_select`, props `button` makes it a button sending
    /// `button_click`. Props `controls` limits it: `move`, `rotate`,
    /// `move_plane`, `move_axis`, `rotate_axis` (default all).
    pub const HANDLE: u32 = 3;
    /// The next click on drawn spatial data sends `data` with the nearest
    /// visible sample or octree cell under it (key, epoch, revision, scan and
    /// source ID or leaf key, LOD state, attributes, x, y, z in the entity's
    /// frame), `no_data` when it covers none, or `stale`; then removes the
    /// entity. Props `prefix` limits it to namespaces under that prefix.
    pub const PICK_DATA: u32 = 4;

    const NAMES: [&str; 5] = ["step", "pick_point", "pick_pose", "handle", "pick_data"];

    pub fn name(value: u32) -> &'static str {
        NAMES.get(value as usize).copied().unwrap_or("unknown")
    }

    pub fn parse(name: &str) -> Option<u32> {
        NAMES.iter().position(|n| *n == name).map(|i| i as u32).or(match name {
            "point" | "pick" => Some(PICK_POINT),
            "pose" => Some(PICK_POSE),
            "data" => Some(PICK_DATA),
            "gizmo" | "marker" => Some(HANDLE),
            _ => None,
        })
    }
}

/// Bits of `flags`.
pub mod draw_flag {
    /// Fill of shapes, bits 0..1: solid, wire, or both.
    pub const FILL_MASK: u32 = 0b11;
    pub const FILL_SOLID: u32 = 0;
    pub const FILL_WIRE: u32 = 1;
    /// Solid fill (alpha honoured) with an outline in `color2`, else in the
    /// fill colour made opaque.
    pub const FILL_BOTH: u32 = 2;
    /// Drawn over everything instead of depth tested.
    pub const ON_TOP: u32 = 1 << 2;
    /// Show `labels` at the items.
    pub const LABELS: u32 = 1 << 3;
    /// Square line ends instead of round ones.
    pub const FLAT_CAPS: u32 = 1 << 4;
    /// Dashed lines, `dash` metres a period, moving at `flow_speed`.
    pub const DASHED: u32 = 1 << 5;
    /// Keep the frame's pose at arrival instead of following it.
    /// Updates, extensions and patches retain that frame; a replacement,
    /// frame change or enabling this flag captures it again.
    pub const SNAPSHOT: u32 = 1 << 6;
    /// Shade lines as tubes and solids with a headlight.
    pub const SHADED: u32 = 1 << 7;
    /// Lines and points lie flat in the frame's XY plane (a ribbon on the
    /// ground) instead of facing the camera.
    pub const FLAT: u32 = 1 << 8;
    /// Added onto what is behind (light) instead of blended over it.
    pub const ADDITIVE: u32 = 1 << 9;
    /// In world and odom frames z is height above the terrain under each
    /// point, and lines bend over the ground between their points.
    pub const DRAPE: u32 = 1 << 10;
    /// Lines end in an arrowhead at the last point of each strip.
    pub const ARROW_TIP: u32 = 1 << 11;

    pub fn fill_parse(name: &str) -> Option<u32> {
        match name {
            "solid" | "fill" | "filler" | "" => Some(FILL_SOLID),
            "wire" | "wireframe" | "outline" | "edge" => Some(FILL_WIRE),
            "both" | "solid+wire" | "translucent" => Some(FILL_BOTH),
            _ => None,
        }
    }
}

/// `style` of an entity: how it looks beyond colour and width. Effect
/// parameters at 0 take the style's default, below 0 switch the effect off.
pub mod draw_style {
    /// Picked by kind and by role words in the path: `path`/`plan`,
    /// `horizon`/`predict`, `rollout`, `trace`/`history`, `goal`,
    /// `waypoint`, `obstacle`, `lookahead`, `footprint`, `alert`/`collision`.
    pub const AUTO: u32 = 0;
    /// Flat colour, smooth edges.
    pub const PLAIN: u32 = 1;
    /// A flat ribbon with bright rims and chevrons flowing along it at
    /// `flow_speed`, `dash` metres apart. Paths and plans.
    pub const RIBBON_FLOW: u32 = 2;
    /// A tube fading to its tail (`fade` is the tail's opacity) with one
    /// pulse running head to tail at `pulse_hz`. Predicted horizons.
    pub const TUBE_PULSE: u32 = 3;
    /// Hairlines added as faint light, so overlaps build a density map.
    /// Sampled rollouts.
    pub const HAIRLINE: u32 = 4;
    /// A low wall of light standing on the line, `beam_height` tall, fading
    /// towards the line's start. Driven traces.
    pub const LIGHT_WALL: u32 = 5;
    /// A ring with ticks and ripples on the ground and a beam of light
    /// `beam_height` tall. Goals.
    pub const BEACON: u32 = 6;
    /// Glowing edges, faint faces and a scanline, with a short beam.
    /// Waypoint boxes.
    pub const HOLO: u32 = 7;
    /// A luminous dome or ellipsoid with a fine energy weave and ripples at
    /// `pulse_hz`, in the supplied colour. Obstacle predictions.
    pub const SHIELD: u32 = 8;
    /// Scrolling hazard stripes, pulsing, drawn on top. Collision warnings.
    pub const ALERT: u32 = 9;
    /// A luminous orb with a pulsing core and bounded halo. Lookahead points.
    pub const ORB: u32 = 10;
    /// An outline on the ground with a fill fading inward. Footprints.
    pub const DECAL: u32 = 11;
    /// Arrows brightening from root to tip, with a halo.
    pub const ARROW_GLOW: u32 = 12;
    /// Spheres with a soft halo.
    pub const POINT_GLOW: u32 = 13;
    /// World text with optional annotation hierarchy, frame and accent.
    pub const LABEL: u32 = 14;
    /// A colormap laid on the ground with optional scalar contours every
    /// `dash`. Costmaps and heatmaps.
    pub const HEAT: u32 = 15;
    /// Flat round splats with no halo, in pixels or metres. Clouds, scans.
    pub const SPLAT: u32 = 16;
    /// A sensor's reach: a faint fill falling off with range, a bright rim,
    /// range rings every `dash` and a radar line sweeping at `pulse_hz`;
    /// frustums add glowing edges and a faint image plane.
    pub const SWEEP: u32 = 17;
    /// A fresnel rim on the shape with a faint fill, fading along a trail
    /// by `fade`. Ghost robots and trajectories.
    pub const GHOST: u32 = 18;
    /// A thin tube with ticks every `dash`, end caps and its length written
    /// at the middle. Measuring.
    pub const RULER: u32 = 19;
    /// A transparent rectangular field grid on boxes and rectangles, with
    /// major panels every `dash` metres and a slow local scan at `pulse_hz`.
    /// Negative glow disables emission; negative pulse disables the scan.
    /// Other geometry uses plain style.
    pub const FIELD: u32 = 20;

    const NAMES: [&str; 21] = [
        "auto", "plain", "ribbon_flow", "tube_pulse", "hairline", "light_wall", "beacon", "holo", "shield", "alert", "orb",
        "decal", "arrow_glow", "point_glow", "label", "heat", "splat", "sweep", "ghost", "ruler", "field",
    ];

    pub fn name(value: u32) -> &'static str {
        NAMES.get(value as usize).copied().unwrap_or("auto")
    }

    pub fn parse(name: &str) -> Option<u32> {
        let name = name.trim().to_ascii_lowercase().replace('-', "_");
        let alias = match name.as_str() {
            "" | "default" => "auto",
            "none" | "flat" => "plain",
            "ribbon" | "flow" => "ribbon_flow",
            "tube" | "pulse" => "tube_pulse",
            "hair" => "hairline",
            "wall" | "tron" => "light_wall",
            "goal" | "beam" => "beacon",
            "hologram" => "holo",
            "warning" | "hazard" => "alert",
            "glow" => "point_glow",
            "arrow" => "arrow_glow",
            "text" => "label",
            "heatmap" | "costmap" => "heat",
            "cloud" | "splats" => "splat",
            "fov" | "radar" => "sweep",
            "measure" => "ruler",
            other => other,
        };
        NAMES.iter().position(|n| *n == alias).map(|i| i as u32)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn field_appends_without_renumbering_existing_styles() {
            let ids = [AUTO, PLAIN, RIBBON_FLOW, TUBE_PULSE, HAIRLINE, LIGHT_WALL, BEACON, HOLO, SHIELD, ALERT, ORB,
                DECAL, ARROW_GLOW, POINT_GLOW, LABEL, HEAT, SPLAT, SWEEP, GHOST, RULER, FIELD];
            for (index, id) in ids.into_iter().enumerate() {
                assert_eq!(id, index as u32);
                assert_eq!(parse(name(id)), Some(id));
            }
        }

        #[test]
        fn field_name_parses_without_changing_existing_aliases() {
            assert_eq!(name(FIELD), "field");
            assert_eq!(parse(" FIELD "), Some(20));
            assert_eq!(parse("tron"), Some(LIGHT_WALL));
            assert_eq!(parse("hologram"), Some(HOLO));
            assert_eq!(parse("radar"), Some(SWEEP));
            assert_eq!(parse("unknown-field"), None);
        }
    }
}

/// `colormap` of an entity: how `values` and `cells` become colours.
/// Ramps (turbo, viridis, rdylgn, rainbow, grey) take the value over
/// `vmin..vmax`; palettes (costmap, map, raw, okabe) take it as it is.
pub mod colormap {
    pub const NONE: u32 = 0;
    pub const TURBO: u32 = 1;
    pub const VIRIDIS: u32 = 2;
    /// Vermillion, yellow, bluish green: rviz `getColorScale`.
    pub const RDYLGN: u32 = 3;
    /// The RViz costmap palette: 0 clear, 1..98 blue to red, 99 cyan
    /// (inscribed), 100 purple (lethal), 255 (-1) grey-green.
    pub const COSTMAP: u32 = 4;
    /// The RViz map palette: 0..100 free white to occupied black, 255 (-1)
    /// grey-green.
    pub const MAP: u32 = 5;
    /// The byte as grey.
    pub const RAW: u32 = 6;
    /// Okabe-Ito categories by index.
    pub const OKABE: u32 = 7;
    pub const RAINBOW: u32 = 8;
    pub const GREY: u32 = 9;

    const NAMES: [&str; 10] = ["none", "turbo", "viridis", "rdylgn", "costmap", "map", "raw", "okabe", "rainbow", "grey"];

    pub fn name(value: u32) -> &'static str {
        NAMES.get(value as usize).copied().unwrap_or("none")
    }

    pub fn parse(name: &str) -> Option<u32> {
        let name = name.trim().to_ascii_lowercase();
        NAMES.iter().position(|n| *n == name).map(|i| i as u32).or(match name.as_str() {
            "" => Some(NONE),
            "jet" | "intensity" => Some(TURBO),
            "cost" | "rd_yl_gn" | "scale" => Some(RDYLGN),
            "occupancy" => Some(MAP),
            "category" | "categorical" | "class" => Some(OKABE),
            "gray" => Some(GREY),
            _ => None,
        })
    }

    /// Whether the map takes raw values rather than `vmin..vmax`.
    pub fn is_palette(map: u32) -> bool {
        matches!(map, COSTMAP | MAP | RAW | OKABE)
    }
}

/// Named colours: rviz_visual_tools names on the Okabe-Ito palette, and the
/// roles the auto styles draw in.
pub mod color {
    use super::rgba;

    pub const BLACK: u32 = 0xff00_0000;
    pub const BROWN: u32 = 0xff1d_76a6;
    pub const BLUE: u32 = 0xffb2_7200;
    pub const CYAN: u32 = 0xffe9_b456;
    pub const GREY: u32 = 0xff99_9999;
    pub const DARK_GREY: u32 = 0xff55_5555;
    pub const GREEN: u32 = 0xff73_9e00;
    pub const LIME_GREEN: u32 = GREEN;
    pub const MAGENTA: u32 = 0xffa7_79cc;
    pub const PINK: u32 = MAGENTA;
    pub const PURPLE: u32 = MAGENTA;
    pub const ORANGE: u32 = 0xff00_9fe6;
    pub const RED: u32 = 0xff00_5ed5;
    pub const WHITE: u32 = 0xffff_ffff;
    pub const YELLOW: u32 = 0xff42_e4f0;
    /// White at 0.25: grey vanishes under additive styles.
    pub const TRANSLUCENT: u32 = 0x40ff_ffff;
    pub const TRANSLUCENT_LIGHT: u32 = 0x1aff_ffff;
    pub const TRANSLUCENT_DARK: u32 = 0x80ff_ffff;
    /// Fully transparent.
    pub const CLEAR: u32 = 0x00ff_ffff;
    /// The colour the entity's style picks.
    pub const DEFAULT: u32 = 0;

    pub const PLAN: u32 = CYAN;
    pub const PREDICTION: u32 = GREEN;
    pub const LOOKAHEAD: u32 = ORANGE;
    pub const GOAL: u32 = YELLOW;
    pub const OBSTACLE: u32 = MAGENTA;
    pub const DANGER: u32 = RED;
    pub const HISTORY: u32 = 0x59ff_ffff;
    pub const SENSOR: u32 = BLUE;

    /// Axis colours of frames: x vermillion, y bluish green, z blue.
    pub const AXIS_X: u32 = RED;
    pub const AXIS_Y: u32 = GREEN;
    pub const AXIS_Z: u32 = BLUE;

    /// The Okabe-Ito categories, in the order categorical colormaps use.
    pub const OKABE: [u32; 8] = [ORANGE, CYAN, GREEN, YELLOW, BLUE, RED, MAGENTA, GREY];

    const NAMES: [(&str, u32); 30] = [
        ("black", BLACK),
        ("brown", BROWN),
        ("blue", BLUE),
        ("cyan", CYAN),
        ("sky", CYAN),
        ("grey", GREY),
        ("gray", GREY),
        ("dark_grey", DARK_GREY),
        ("dark_gray", DARK_GREY),
        ("green", GREEN),
        ("lime_green", LIME_GREEN),
        ("magenta", MAGENTA),
        ("pink", PINK),
        ("purple", PURPLE),
        ("orange", ORANGE),
        ("red", RED),
        ("vermillion", RED),
        ("white", WHITE),
        ("yellow", YELLOW),
        ("translucent", TRANSLUCENT),
        ("translucent_light", TRANSLUCENT_LIGHT),
        ("translucent_dark", TRANSLUCENT_DARK),
        ("clear", CLEAR),
        ("plan", PLAN),
        ("prediction", PREDICTION),
        ("lookahead", LOOKAHEAD),
        ("goal", GOAL),
        ("obstacle", OBSTACLE),
        ("danger", DANGER),
        ("history", HISTORY),
    ];

    /// The colour of a name, `sensor` and `default` included.
    pub fn named(name: &str) -> Option<u32> {
        let name = name.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        match name.as_str() {
            "sensor" => Some(SENSOR),
            "default" | "auto" => Some(DEFAULT),
            _ => NAMES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c),
        }
    }

    /// A stable bright colour for `key` (rviz RAND without the flicker).
    pub fn random(key: &str) -> u32 {
        let mut hash: u32 = 0x811c_9dc5;
        for b in key.bytes() {
            hash = (hash ^ b as u32).wrapping_mul(0x0100_0193);
        }
        let hue = (hash % 360) as f32 / 60.0;
        let x = 1.0 - (hue % 2.0 - 1.0).abs();
        let (r, g, b) = match hue as u32 {
            0 => (1.0, x, 0.0),
            1 => (x, 1.0, 0.0),
            2 => (0.0, 1.0, x),
            3 => (0.0, x, 1.0),
            4 => (x, 0.0, 1.0),
            _ => (1.0, 0.0, x),
        };
        let lift = |v: f32| (64.0 + v * 191.0) as u8;
        rgba(lift(r), lift(g), lift(b), 255)
    }

    /// `c` with its alpha set to `a` in 0..1.
    pub fn with_alpha(c: u32, a: f32) -> u32 {
        (c & 0x00ff_ffff) | (((a.clamp(0.0, 1.0) * 255.0).round() as u32) << 24)
    }
}

/// rviz_visual_tools sizes in metres, before `global_scale`.
pub mod scale {
    pub const XXXXSMALL: f32 = 0.001;
    pub const XXXSMALL: f32 = 0.0025;
    pub const XXSMALL: f32 = 0.005;
    pub const XSMALL: f32 = 0.0065;
    pub const SMALL: f32 = 0.0075;
    pub const MEDIUM: f32 = 0.01;
    pub const LARGE: f32 = 0.025;
    pub const XLARGE: f32 = 0.05;
    pub const XXLARGE: f32 = 0.075;
    pub const XXXLARGE: f32 = 0.1;
    pub const XXXXLARGE: f32 = 0.5;
    /// Machines are vehicles: MEDIUM is 0.1 m.
    pub const GLOBAL: f32 = 10.0;

    /// A width of `n` pixels.
    pub fn px(n: f32) -> f32 {
        -n.abs()
    }

    pub fn parse(name: &str) -> Option<f32> {
        Some(match name.trim().to_ascii_uppercase().as_str() {
            "XXXXSMALL" => XXXXSMALL,
            "XXXSMALL" => XXXSMALL,
            "XXSMALL" => XXSMALL,
            "XSMALL" => XSMALL,
            "SMALL" => SMALL,
            "MEDIUM" => MEDIUM,
            "LARGE" => LARGE,
            "XLARGE" => XLARGE,
            "XXLARGE" => XXLARGE,
            "XXXLARGE" => XXXLARGE,
            "XXXXLARGE" => XXXXLARGE,
            _ => return None,
        })
    }
}

/// Section bytes per chunk, under [`crate::host::MAX_PAYLOAD_BYTES`] with the
/// header and props.
pub const DRAW_CHUNK_BYTES: usize = 12 * 1024;

/// Floats per instance: x, y, z, qw, qx, qy, qz, sx, sy, sz.
pub const INSTANCE_FLOATS: usize = 10;

/// Most points or instances one entity may hold.
pub const MAX_DRAW_ITEMS: usize = 2_000_000;

/// Most cells one grid may hold.
pub const MAX_GRID_CELLS: usize = 16_777_216;

/// Metres per `qpoints` step when `quant` is 0.
pub const DEFAULT_POINT_QUANT: f32 = 0.01;

/// Metres per `ranges` step when `quant` is 0.
pub const DEFAULT_RANGE_QUANT: f32 = 0.001;

/// A scan range meaning no return.
pub const NO_RETURN: u16 = u16::MAX;

/// RGBA8 packed as `r | g << 8 | b << 16 | a << 24` (bytes r, g, b, a).
pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from_le_bytes([r, g, b, a])
}

/// RGBA from components in 0..1.
pub fn rgba_f(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    rgba(q(r), q(g), q(b), q(a))
}

pub fn unpack_rgba(color: u32) -> [u8; 4] {
    color.to_le_bytes()
}

/// A colour from `#rgb`, `#rrggbb`, `#rrggbbaa` or a name (see [`color`]).
pub fn parse_color(text: &str) -> Option<u32> {
    let t = text.trim().to_ascii_lowercase();
    if let Some(c) = color::named(&t).filter(|c| *c != color::DEFAULT) {
        return Some(c);
    }
    let hex = t.strip_prefix('#').unwrap_or(&t);
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    match hex.len() {
        3 => {
            let nib = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok().map(|v| v * 17);
            Some(rgba(nib(0)?, nib(1)?, nib(2)?, 255))
        }
        6 => Some(rgba(byte(0)?, byte(2)?, byte(4)?, 255)),
        8 => Some(rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

/// Where an entity's coordinates are measured from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawFrame {
    /// The home site.
    World,
    /// A machine's own datum, the frame its reported pose is in.
    Odom(String),
    /// A link of a machine; an empty link is the base link.
    Link(String, String),
}

impl DrawFrame {
    pub fn parse(text: &str) -> Self {
        let t = text.trim().trim_matches('/');
        match t {
            "" | "world" | "map" | "home" => Self::World,
            _ => match t.split_once('/') {
                Some((machine, "odom")) => Self::Odom(machine.to_string()),
                Some((machine, link)) => Self::Link(machine.to_string(), link.to_string()),
                None => Self::Link(t.to_string(), String::new()),
            },
        }
    }

    pub fn machine(&self) -> Option<&str> {
        match self {
            Self::World => None,
            Self::Odom(m) | Self::Link(m, _) => Some(m),
        }
    }

    /// Whether z in it can be height above the terrain.
    pub fn drapes(text: &str) -> bool {
        matches!(Self::parse(text), Self::World | Self::Odom(_))
    }
}

/// Whether `ns` is `prefix` or a path below it; an empty prefix holds all.
pub fn ns_within(ns: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_matches('/');
    let ns = ns.trim_matches('/');
    prefix.is_empty() || ns == prefix || ns.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'))
}

/// One shape, axis triad or arrow: a pose in the entity's frame and a size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    pub position: [f32; 3],
    /// w, x, y, z.
    pub rotation: [f32; 4],
    pub size: [f32; 3],
}

impl Default for Instance {
    fn default() -> Self {
        Self { position: [0.0; 3], rotation: [1.0, 0.0, 0.0, 0.0], size: [0.0; 3] }
    }
}

impl Instance {
    pub fn at(position: [f32; 3], size: [f32; 3]) -> Self {
        Self { position, size, ..Self::default() }
    }

    /// Turned `yaw` radians about z.
    pub fn yawed(mut self, yaw: f32) -> Self {
        let half = yaw * 0.5;
        self.rotation = [half.cos(), 0.0, 0.0, half.sin()];
        self
    }

    /// Turned by the quaternion w, x, y, z.
    pub fn turned(mut self, rotation: [f32; 4]) -> Self {
        self.rotation = rotation;
        self
    }

    fn write(&self, out: &mut Vec<f32>) {
        out.extend_from_slice(&self.position);
        out.extend_from_slice(&self.rotation);
        out.extend_from_slice(&self.size);
    }

    fn read(v: &[f32]) -> Self {
        Self { position: [v[0], v[1], v[2]], rotation: [v[3], v[4], v[5], v[6]], size: [v[7], v[8], v[9]] }
    }
}

/// One retained draw entity, or one chunk of it, or a delete or clear.
/// Props: `ns`, `id`, `frame`, and per kind `text`, `max_points`, ...
/// Sections hold 0, 1 or N values: none takes the header default, one is
/// used for every item, N gives each item its own.
#[datapod::datapod(name = "gearbox.draw.v1")]
#[derive(Default)]
pub struct DrawEntity {
    /// Sim seconds the entity lives after its last send; 0 keeps it.
    pub lifetime_s: f64,
    /// Entity pose in its frame: position, then rotation w, x, y, z.
    pub px: f64,
    pub py: f64,
    pub pz: f64,
    pub qw: f64,
    pub qx: f64,
    pub qy: f64,
    pub qz: f64,
    /// Default instance size of shapes, axis length in x.
    pub sx: f32,
    pub sy: f32,
    pub sz: f32,
    /// Line, outline and default point width: > 0 metres, < 0 pixels, 0 default.
    pub width: f32,
    /// Brightness of the style's light: 1 its usual glow, more brighter.
    /// For this and the effects below, 0 is the style's default and < 0 off.
    pub glow: f32,
    /// Rate of pulses, ripples and breathing, Hz.
    pub pulse_hz: f32,
    /// Speed patterns travel along lines, m/s.
    pub flow_speed: f32,
    /// Spacing of chevrons, dashes, stripes, ticks, rings and contours, metres.
    pub dash: f32,
    /// Height of beams and light walls, metres.
    pub beam_height: f32,
    /// Opacity the style fades lines to at their faded end, 0..1.
    pub fade: f32,
    /// Urgency metadata, 0..1; does not replace the supplied colour.
    pub urgency: f32,
    /// Values `vmin..vmax` span a ramp [`colormap`]; equal ones take the
    /// data's own range, `vmin` above `vmax` runs the ramp backwards.
    pub vmin: f32,
    pub vmax: f32,
    /// Metres per step of `qpoints` (0: [`DEFAULT_POINT_QUANT`]) and of
    /// `ranges` (0: [`DEFAULT_RANGE_QUANT`]).
    pub quant: f32,
    pub op: u32,
    pub kind: u32,
    /// Line mode, point style, shape, grid encoding, panel or interaction, by kind.
    pub sub: u32,
    pub flags: u32,
    /// [`draw_style`] of the entity.
    pub style: u32,
    /// Chunks of one entity share it.
    pub seq: u32,
    /// Items before this chunk (points, instances or cells), or the first
    /// item a patch replaces.
    pub chunk_offset: u32,
    /// 1 on the chunk that completes the entity.
    pub last: u32,
    /// Default colour, RGBA8; 0 picks one from the entity's path.
    pub color: u32,
    /// Outline colour of shapes, background of text; 0 derives one.
    pub color2: u32,
    /// [`colormap`] `values` and `cells` are coloured through.
    pub colormap: u32,
    pub _pad: u32,
    /// xyz per point.
    #[dp(bytes, section = "points")]
    pub points: Vec<f32>,
    /// RGBA8 per item.
    #[dp(bytes, section = "colors")]
    pub colors: Vec<u32>,
    /// Radius per item: > 0 metres, < 0 pixels.
    #[dp(bytes, section = "radii")]
    pub radii: Vec<f32>,
    /// First point of each strip or ring; empty is one. Grid patches: the
    /// rectangles x, y, w, h.
    #[dp(bytes, section = "offsets")]
    pub offsets: Vec<u32>,
    /// [`INSTANCE_FLOATS`] per instance.
    #[dp(bytes, section = "instances")]
    pub instances: Vec<f32>,
    /// A scalar per item (per strip for lines when one each) coloured
    /// through `colormap`; heights of grid cells, joint positions of
    /// ghosts, data of panels.
    #[dp(bytes, section = "values")]
    pub values: Vec<f32>,
    /// Rings of each polygon, its outline first.
    #[dp(bytes, section = "rings")]
    pub rings: Vec<u32>,
    /// xyz per point in steps of `quant` from the entity origin, in place
    /// of `points`.
    #[dp(bytes, section = "qpoints")]
    pub qpoints: Vec<i16>,
    /// Scan ranges in steps of `quant`, beam i at props `angle_min` +
    /// i × `angle_inc` radians about z from +x, in place of `points`;
    /// [`NO_RETURN`] where nothing came back.
    #[dp(bytes, section = "ranges")]
    pub ranges: Vec<u16>,
    /// Grid cells in the encoding of `sub`; for other kinds one byte per
    /// item standing for `vmin + (vmax - vmin) × b / 255` when `values` is
    /// empty.
    #[dp(bytes, section = "cells")]
    pub cells: Vec<u8>,
    /// One label per item, newline separated.
    #[dp(bytes, section = "labels")]
    pub labels: Vec<u8>,
    #[dp(bytes, section = "props")]
    pub props: Vec<u8>,
}

/// Cells of a grid in one of the encodings.
#[derive(Debug, Clone, PartialEq)]
pub enum GridCells {
    U8(Vec<u8>),
    U16(Vec<u16>),
    F32(Vec<f32>),
}

impl GridCells {
    fn encode(&self) -> (u32, Vec<u8>) {
        match self {
            Self::U8(v) => (grid_encoding::U8, v.clone()),
            Self::U16(v) => (grid_encoding::U16, v.iter().flat_map(|c| c.to_le_bytes()).collect()),
            Self::F32(v) => (grid_encoding::F32, v.iter().flat_map(|c| c.to_le_bytes()).collect()),
        }
    }
}

impl DrawEntity {
    /// An empty entity of `kind` at `ns` + `id`, identity pose, unit scale.
    pub fn new(kind: u32, ns: &str, id: &str) -> Self {
        Self {
            op: draw_op::UPSERT,
            kind,
            last: 1,
            qw: 1.0,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            props: Props::from_pairs(&[("ns", ns), ("id", id)]).into_bytes(),
            ..Default::default()
        }
    }

    /// One polyline through `points`.
    pub fn lines(ns: &str, id: &str, points: &[[f32; 3]]) -> Self {
        Self::new(draw_kind::LINES, ns, id).with_points(points)
    }

    /// Many polylines in one entity.
    pub fn strips(ns: &str, id: &str, strips: &[Vec<[f32; 3]>]) -> Self {
        let mut e = Self::new(draw_kind::LINES, ns, id);
        for strip in strips {
            e.offsets.push((e.points.len() / 3) as u32);
            e.points.extend(strip.iter().flatten());
        }
        if e.offsets.len() <= 1 {
            e.offsets.clear();
        }
        e
    }

    pub fn points(ns: &str, id: &str, points: &[[f32; 3]]) -> Self {
        Self::new(draw_kind::POINTS, ns, id).with_points(points)
    }

    /// Arrows from each tail to its tip.
    pub fn arrows(ns: &str, id: &str, arrows: &[([f32; 3], [f32; 3])]) -> Self {
        let mut e = Self::new(draw_kind::ARROWS, ns, id);
        for (tail, tip) in arrows {
            e.points.extend(tail);
            e.points.extend(tip);
        }
        e
    }

    pub fn shapes(ns: &str, id: &str, shape: u32, instances: &[Instance]) -> Self {
        Self::new(draw_kind::SHAPES, ns, id).with_sub(shape).with_instances(instances)
            .with_scale([1.0, 1.0, if shape == shape_kind::SECTOR { 0.0 } else { 1.0 }])
    }

    pub fn triangles(ns: &str, id: &str, vertices: &[[f32; 3]]) -> Self {
        Self::new(draw_kind::TRIANGLES, ns, id).with_points(vertices)
    }

    /// `text` at each of `at`.
    pub fn text(ns: &str, id: &str, at: &[[f32; 3]], text: &str) -> Self {
        Self::new(draw_kind::TEXT, ns, id).with_points(at).with_prop("text", text)
    }

    /// An axis triad `length` long at the entity pose.
    pub fn axes(ns: &str, id: &str, length: f32) -> Self {
        Self::new(draw_kind::AXES, ns, id).with_scale([length; 3])
    }

    /// One polygon: its outline, then its holes.
    pub fn polygon(ns: &str, id: &str, rings: &[Vec<[f32; 3]>]) -> Self {
        Self::polygons(ns, id, &[rings.to_vec()])
    }

    /// Many polygons, each its outline then its holes, filled and outlined.
    pub fn polygons(ns: &str, id: &str, polygons: &[Vec<Vec<[f32; 3]>>]) -> Self {
        let mut e = Self::new(draw_kind::POLYGON, ns, id).with_fill(draw_flag::FILL_BOTH);
        for rings in polygons {
            e.rings.push(rings.len() as u32);
            for ring in rings {
                e.offsets.push(e.point_count() as u32);
                e.points.extend(ring.iter().flatten());
            }
        }
        if e.rings.len() <= 1 {
            e.rings.clear();
        }
        if e.offsets.len() <= 1 {
            e.offsets.clear();
        }
        e
    }

    /// A `cols` by `rows` grid of `cell` metre cells, row by row from the
    /// pose, in [`draw_style::HEAT`].
    pub fn grid(ns: &str, id: &str, cols: usize, rows: usize, cell: f32, cells: GridCells) -> Self {
        let (encoding, bytes) = cells.encode();
        let mut e = Self::new(draw_kind::GRID, ns, id).with_sub(encoding).with_style(draw_style::HEAT);
        e.cells = bytes;
        e.with_prop("cols", &cols.to_string()).with_prop("rows", &rows.to_string()).with_prop("cell", &cell.to_string())
    }

    /// Replaces the `w` by `h` cells at column `x`, row `y` of a live grid.
    pub fn grid_patch(ns: &str, id: &str, x: u32, y: u32, w: u32, h: u32, cells: GridCells) -> Self {
        let (encoding, bytes) = cells.encode();
        let mut e = Self::new(draw_kind::GRID, ns, id).with_sub(encoding);
        e.op = draw_op::PATCH;
        e.offsets = vec![x, y, w, h];
        e.cells = bytes;
        e
    }

    /// A scan: `ranges` metres (non-finite or above `range_max` is no
    /// return) from `angle_min` by `angle_inc`, sent as millimetres.
    pub fn scan(ns: &str, id: &str, ranges: &[f32], angle_min: f32, angle_inc: f32) -> Self {
        let q = DEFAULT_RANGE_QUANT;
        let mut e = Self::new(draw_kind::POINTS, ns, id).with_style(draw_style::SPLAT);
        e.ranges = ranges
            .iter()
            .map(|r| if r.is_finite() && *r >= 0.0 { ((r / q).round() as u32).min(NO_RETURN as u32 - 1) as u16 } else { NO_RETURN })
            .collect();
        e.with_prop("angle_min", &angle_min.to_string()).with_prop("angle_inc", &angle_inc.to_string())
    }

    /// See-through copies of `machine` at each of `poses`.
    pub fn ghost(ns: &str, id: &str, machine: &str, poses: &[Instance]) -> Self {
        Self::new(draw_kind::GHOST, ns, id).with_instances(poses).with_style(draw_style::GHOST).with_prop("machine", machine)
    }

    /// A chart of `kind` showing `values`.
    pub fn panel(ns: &str, id: &str, kind: u32, values: &[f32]) -> Self {
        Self::new(draw_kind::PANEL, ns, id).with_sub(kind).with_values(values)
    }

    /// The mesh at `asset`.
    pub fn mesh(ns: &str, id: &str, asset: &str) -> Self {
        Self::new(draw_kind::MESH, ns, id).with_prop("asset", asset)
    }

    /// A request for input of [`interact_kind`] `kind`.
    pub fn interact(ns: &str, id: &str, kind: u32) -> Self {
        Self::new(draw_kind::INTERACT, ns, id).with_sub(kind)
    }

    /// A header change to the live entity: set what changes with the
    /// builders and name the groups with [`Self::updating`].
    pub fn update(ns: &str, id: &str) -> Self {
        Self { op: draw_op::UPDATE, ..Self::new(0, ns, id) }
    }

    /// Takes only the named header groups or scalars (see [`draw_op::UPDATE`]).
    /// An empty list replaces all headers; `["props"]` changes only metadata.
    pub fn updating(self, groups: &[&str]) -> Self {
        self.with_prop("set", &groups.join(","))
    }

    /// Items to replace from `offset` on in the live entity of `kind`.
    pub fn patch(kind: u32, ns: &str, id: &str, offset: u32) -> Self {
        Self { op: draw_op::PATCH, chunk_offset: offset, ..Self::new(kind, ns, id) }
    }

    pub fn delete(ns: &str, id: &str) -> Self {
        Self { op: draw_op::DELETE, ..Self::new(0, ns, id) }
    }

    /// Remove `prefix` and everything below it; `""` removes all.
    pub fn clear(prefix: &str) -> Self {
        Self { op: draw_op::CLEAR, ..Self::new(0, prefix, "") }
    }

    pub fn with_points(mut self, points: &[[f32; 3]]) -> Self {
        self.points = points.iter().flatten().copied().collect();
        self
    }

    /// `points` sent as steps of `quant` metres (i16, ±327 m at 1 cm).
    pub fn with_quantized_points(mut self, points: &[[f32; 3]], quant: f32) -> Self {
        let q = if quant > 0.0 { quant } else { DEFAULT_POINT_QUANT };
        self.points.clear();
        self.qpoints = super::draw_geom::quantize(points, q);
        self.quant = q;
        self
    }

    pub fn with_instances(mut self, instances: &[Instance]) -> Self {
        self.instances.clear();
        for i in instances {
            i.write(&mut self.instances);
        }
        self
    }

    pub fn with_color(mut self, color: u32) -> Self {
        self.color = color;
        self
    }

    pub fn with_color2(mut self, color: u32) -> Self {
        self.color2 = color;
        self
    }

    /// Frost strength, 0..1, applied to the transmitted background.
    pub fn with_blur(self, strength: f32) -> Self {
        self.with_prop("blur", &strength.to_string())
    }

    pub fn with_colors(mut self, colors: &[u32]) -> Self {
        self.colors = colors.to_vec();
        self
    }

    /// Width of lines and outlines: > 0 metres, < 0 pixels.
    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn with_radii(mut self, radii: &[f32]) -> Self {
        self.radii = radii.to_vec();
        self
    }

    pub fn with_offsets(mut self, offsets: &[u32]) -> Self {
        self.offsets = offsets.to_vec();
        self
    }

    pub fn with_values(mut self, values: &[f32]) -> Self {
        self.values = values.to_vec();
        self
    }

    /// `values` as one byte each over `vmin..vmax`.
    pub fn with_byte_values(mut self, values: &[f32], vmin: f32, vmax: f32) -> Self {
        let span = vmax - vmin;
        self.cells = values
            .iter()
            .map(|v| if span.abs() > 1e-12 { (((v - vmin) / span).clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 })
            .collect();
        self.values.clear();
        self.vmin = vmin;
        self.vmax = vmax;
        self
    }

    /// Colours `values` and `cells` through `map` over `vmin..vmax`.
    pub fn with_colormap(mut self, map: u32, vmin: f32, vmax: f32) -> Self {
        self.colormap = map;
        self.vmin = vmin;
        self.vmax = vmax;
        self
    }

    pub fn with_rings(mut self, rings: &[u32]) -> Self {
        self.rings = rings.to_vec();
        self
    }

    pub fn with_sub(mut self, sub: u32) -> Self {
        self.sub = sub;
        self
    }

    pub fn with_flags(mut self, flags: u32) -> Self {
        self.flags |= flags;
        self
    }

    pub fn without_flags(mut self, flags: u32) -> Self {
        self.flags &= !flags;
        self
    }

    /// A [`draw_style`].
    pub fn with_style(mut self, style: u32) -> Self {
        self.style = style;
        self
    }

    pub fn with_glow(mut self, glow: f32) -> Self {
        self.glow = glow;
        self
    }

    pub fn with_pulse(mut self, hz: f32) -> Self {
        self.pulse_hz = hz;
        self
    }

    /// Patterns travel `speed` m/s along lines, `spacing` metres apart.
    pub fn with_flow(mut self, speed: f32, spacing: f32) -> Self {
        self.flow_speed = speed;
        self.dash = spacing;
        self
    }

    /// Spacing of the style's pattern: chevrons, ticks, rings, contours.
    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.dash = spacing;
        self
    }

    pub fn with_dash(mut self, period: f32) -> Self {
        self.dash = period;
        self.with_flags(draw_flag::DASHED)
    }

    pub fn with_beam(mut self, height: f32) -> Self {
        self.beam_height = height;
        self
    }

    pub fn with_fade(mut self, fade: f32) -> Self {
        self.fade = fade;
        self
    }

    pub fn with_urgency(mut self, urgency: f32) -> Self {
        self.urgency = urgency;
        self
    }

    pub fn with_fill(mut self, fill: u32) -> Self {
        self.flags = (self.flags & !draw_flag::FILL_MASK) | (fill & draw_flag::FILL_MASK);
        self
    }

    pub fn in_frame(self, frame: &str) -> Self {
        self.with_prop("frame", frame)
    }

    /// In `frame`, draped on the terrain when the frame allows it.
    pub fn draped_in(self, frame: &str) -> Self {
        let e = self.in_frame(frame);
        if DrawFrame::drapes(frame) { e.with_flags(draw_flag::DRAPE) } else { e }
    }

    pub fn with_lifetime(mut self, seconds: f64) -> Self {
        self.lifetime_s = if seconds.is_finite() { seconds.max(0.0) } else { seconds };
        self
    }

    /// Position and rotation (w, x, y, z) in the frame.
    pub fn with_pose(mut self, position: [f64; 3], rotation: [f64; 4]) -> Self {
        [self.px, self.py, self.pz] = position;
        [self.qw, self.qx, self.qy, self.qz] = rotation;
        self
    }

    /// At `position` turned `yaw` radians about z.
    pub fn at_yaw(self, position: [f64; 3], yaw: f64) -> Self {
        self.with_pose(position, [(0.5 * yaw).cos(), 0.0, 0.0, (0.5 * yaw).sin()])
    }

    pub fn with_scale(mut self, scale: [f32; 3]) -> Self {
        [self.sx, self.sy, self.sz] = scale;
        self
    }

    pub fn with_labels<S: AsRef<str>>(mut self, labels: &[S]) -> Self {
        self.labels = labels.iter().map(|s| s.as_ref().replace('\n', " ")).collect::<Vec<_>>().join("\n").into_bytes();
        self
    }

    /// `labels` shown at their items.
    pub fn labelled<S: AsRef<str>>(self, labels: &[S]) -> Self {
        self.with_labels(labels).with_flags(draw_flag::LABELS)
    }

    pub fn with_prop(mut self, key: &str, value: &str) -> Self {
        self.props = Props::from_bytes(&self.props).with(key, value).into_bytes();
        self
    }

    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }

    pub fn prop_f32(&self, key: &str) -> Option<f32> {
        self.props().get_f64(key).map(|v| v as f32).filter(|v| v.is_finite())
    }

    pub fn ns(&self) -> String {
        self.props().get("ns").unwrap_or_default()
    }

    pub fn id(&self) -> String {
        self.props().get("id").unwrap_or_default()
    }

    pub fn frame(&self) -> DrawFrame {
        DrawFrame::parse(&self.props().get("frame").unwrap_or_default())
    }

    /// Points in whichever form they came: `points`, `qpoints` or `ranges`.
    pub fn point_count(&self) -> usize {
        if !self.points.is_empty() {
            self.points.len() / 3
        } else if !self.qpoints.is_empty() {
            self.qpoints.len() / 3
        } else {
            self.ranges.len()
        }
    }

    pub fn instance_count(&self) -> usize {
        self.instances.len() / INSTANCE_FLOATS
    }

    /// Point `i` of `points` (see [`Self::expanded`] for the other forms).
    pub fn point_at(&self, i: usize) -> [f32; 3] {
        [self.points[3 * i], self.points[3 * i + 1], self.points[3 * i + 2]]
    }

    pub fn point_list(&self) -> Vec<[f32; 3]> {
        self.points.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect()
    }

    pub fn instance_list(&self) -> Vec<Instance> {
        self.instances.chunks_exact(INSTANCE_FLOATS).map(Instance::read).collect()
    }

    pub fn label_list(&self) -> Vec<String> {
        if self.labels.is_empty() {
            return Vec::new();
        }
        String::from_utf8_lossy(&self.labels).split('\n').map(str::to_string).collect()
    }

    /// Whether the entity counts instances rather than points.
    pub fn instanced(&self) -> bool {
        matches!(self.kind, draw_kind::SHAPES | draw_kind::AXES | draw_kind::GHOST | draw_kind::MESH)
            || (self.kind == draw_kind::ARROWS && !self.instances.is_empty())
    }

    /// Cells of a grid, from the bytes it holds.
    pub fn cell_count(&self) -> usize {
        self.cells.len() / grid_encoding::size(self.sub)
    }

    /// Columns, rows and cell size (x, y) of a grid.
    pub fn grid_dims(&self) -> (usize, usize, f32, f32) {
        let p = self.props();
        let n = |k: &str| p.get(k).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        let cell = self.prop_f32("cell").filter(|c| *c > 0.0).unwrap_or(1.0);
        let cell_y = self.prop_f32("cell_y").filter(|c| *c > 0.0).unwrap_or(cell);
        (n("cols"), n("rows"), cell, cell_y)
    }

    /// Cell `i` of a grid as a number, `None` for the props `unknown` value.
    pub fn cell_value(&self, i: usize) -> Option<f32> {
        let unknown = self.prop_f32("unknown");
        let v = match self.sub {
            grid_encoding::U16 => u16::from_le_bytes([self.cells[2 * i], self.cells[2 * i + 1]]) as f32,
            grid_encoding::F32 => f32::from_le_bytes(self.cells[4 * i..4 * i + 4].try_into().ok()?),
            _ => self.cells[i] as f32,
        };
        (v.is_finite() && unknown != Some(v)).then_some(v)
    }

    /// Points, instances or cells, whichever the kind counts.
    pub fn item_count(&self) -> usize {
        if self.instanced() {
            self.instance_count()
        } else if self.kind == draw_kind::GRID {
            self.cell_count()
        } else if self.kind == draw_kind::PANEL {
            self.values.len()
        } else {
            self.point_count()
        }
    }

    /// Point ranges of the strips (rings of polygons).
    pub fn strip_ranges(&self) -> Vec<Range<usize>> {
        let n = self.point_count();
        let mut starts: Vec<usize> = self.offsets.iter().map(|o| *o as usize).filter(|o| *o < n).collect();
        if starts.first() != Some(&0) {
            starts.insert(0, 0);
        }
        starts.dedup();
        let mut out = Vec::with_capacity(starts.len());
        for (i, s) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(n);
            if end > *s {
                out.push(*s..end);
            }
        }
        out
    }

    /// Polygons as their rings' point ranges, outline first.
    pub fn polygon_rings(&self) -> Vec<Vec<Range<usize>>> {
        let strips = self.strip_ranges();
        if self.rings.is_empty() {
            return if strips.is_empty() { Vec::new() } else { vec![strips] };
        }
        let mut out = Vec::with_capacity(self.rings.len());
        let mut at = 0;
        for count in &self.rings {
            let end = (at + *count as usize).min(strips.len());
            out.push(strips[at..end].to_vec());
            at = end;
        }
        out
    }

    pub fn polygon_count(&self) -> usize {
        if self.rings.is_empty() {
            (self.point_count() > 0) as usize
        } else {
            self.rings.len() - self.continues().is_some() as usize
        }
    }

    /// Items colours, radii and values count per: points, arrows, strips,
    /// polygons, instances or cells.
    pub fn unit_count(&self) -> usize {
        match self.kind {
            draw_kind::ARROWS if self.instances.is_empty() => self.point_count() / 2,
            draw_kind::POLYGON => self.polygon_count(),
            _ => self.item_count(),
        }
    }

    /// Items labels count per: strips for lines, polygons, else units.
    pub fn label_unit_count(&self) -> usize {
        match self.kind {
            draw_kind::LINES => self.strip_ranges().len(),
            draw_kind::TRIANGLES => self.point_count() / 3,
            _ => self.unit_count(),
        }
    }

    /// Checks the sections agree with each other and the kind.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(raw) = self.props().get("blur") {
            let value = raw.parse::<f32>().map_err(|_| "blur must be a finite number in 0..1")?;
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err("blur must be a finite number in 0..1".into());
            }
        }
        let header_only = matches!(self.op, draw_op::DELETE | draw_op::CLEAR | draw_op::UPDATE);
        if self.op > draw_op::PATCH {
            return Err(format!("unknown draw op {}", self.op));
        }
        if self.op != draw_op::CLEAR && self.id().is_empty() {
            return Err("draw needs an `id`".into());
        }
        if !header_only && !(draw_kind::LINES..=draw_kind::LAST).contains(&self.kind) {
            return Err(format!("unknown draw kind {}", self.kind));
        }
        if self.points.len() % 3 != 0 {
            return Err("points are not xyz triples".into());
        }
        if self.qpoints.len() % 3 != 0 {
            return Err("qpoints are not xyz triples".into());
        }
        let forms = [!self.points.is_empty(), !self.qpoints.is_empty(), !self.ranges.is_empty()];
        if forms.iter().filter(|f| **f).count() > 1 {
            return Err("points come as one of points, qpoints or ranges".into());
        }
        if self.instances.len() % INSTANCE_FLOATS != 0 {
            return Err(format!("instances are not {INSTANCE_FLOATS} floats each"));
        }
        if self.item_count() > MAX_DRAW_ITEMS && self.kind != draw_kind::GRID {
            return Err(format!("more than {MAX_DRAW_ITEMS} items"));
        }
        let units = self.unit_count();
        for (name, len) in [("colors", self.colors.len()), ("radii", self.radii.len())] {
            if len > 1 && len != units && self.kind != draw_kind::PANEL {
                return Err(format!("{len} {name} for {units} items"));
            }
        }
        let n = self.point_count() as u32;
        match self.kind {
            draw_kind::GRID => self.validate_grid()?,
            _ => {
                if self.offsets.windows(2).any(|w| w[0] >= w[1]) || self.offsets.iter().any(|o| *o > n) {
                    return Err("offsets must rise and stay within the points".into());
                }
                let per_strip = self.kind == draw_kind::LINES && self.values.len() == self.strip_ranges().len();
                if !matches!(self.kind, draw_kind::GHOST | draw_kind::PANEL)
                    && self.values.len() > 1
                    && self.values.len() != units
                    && !per_strip
                {
                    return Err(format!("{} values for {units} items", self.values.len()));
                }
                if self.cells.len() > 1 && self.cells.len() != units {
                    return Err(format!("{} byte values for {units} items", self.cells.len()));
                }
            }
        }
        if self.kind == draw_kind::POLYGON && !self.rings.is_empty() {
            let strips = self.strip_ranges().len();
            let total: u64 = self.rings.iter().map(|r| *r as u64).sum();
            if total != strips as u64 || self.rings.contains(&0) {
                return Err(format!("rings count {total} rings for {strips}"));
            }
        }
        if self.points.iter().chain(self.instances.iter()).chain(self.values.iter()).chain(self.radii.iter()).any(|v| !v.is_finite()) {
            return Err("non-finite coordinate, radius or value".into());
        }
        if [self.px, self.py, self.pz, self.qw, self.qx, self.qy, self.qz].iter().any(|v| !v.is_finite()) {
            return Err("non-finite pose".into());
        }
        if [self.sx, self.sy, self.sz].iter().any(|v| !v.is_finite()) {
            return Err("non-finite scale".into());
        }
        if !self.lifetime_s.is_finite() {
            return Err("non-finite lifetime".into());
        }
        let effects = [
            self.width,
            self.glow,
            self.pulse_hz,
            self.flow_speed,
            self.dash,
            self.beam_height,
            self.fade,
            self.urgency,
            self.vmin,
            self.vmax,
            self.quant,
        ];
        if effects.iter().any(|v| !v.is_finite()) {
            return Err("non-finite width or effect parameter".into());
        }
        Ok(())
    }

    fn validate_grid(&self) -> Result<(), String> {
        let size = grid_encoding::size(self.sub);
        if self.sub > grid_encoding::F32 {
            return Err(format!("unknown grid encoding {}", self.sub));
        }
        if self.cells.len() % size != 0 {
            return Err(format!("grid cells are not {size} bytes each"));
        }
        if self.op == draw_op::PATCH {
            if self.offsets.len() % 4 != 0 {
                return Err("a grid patch takes x, y, w, h rectangles".into());
            }
            let cells: u64 = self.offsets.chunks_exact(4).map(|r| r[2] as u64 * r[3] as u64).sum();
            if cells as usize != self.cell_count() {
                return Err(format!("the rectangles hold {cells} cells, the patch {}", self.cell_count()));
            }
            if !self.values.is_empty() && self.values.len() != self.cell_count() {
                return Err("a grid patch takes a height per cell or none".into());
            }
            return Ok(());
        }
        if self.op == draw_op::EXTEND {
            return Err("a grid is changed with patches, not extended".into());
        }
        let (cols, rows, ..) = self.grid_dims();
        let first = self.op == draw_op::UPSERT || self.chunk_offset == 0;
        if first && (cols == 0 || rows == 0) {
            return Err("a grid needs props `cols` and `rows`".into());
        }
        if cols.saturating_mul(rows) > MAX_GRID_CELLS {
            return Err(format!("more than {MAX_GRID_CELLS} cells"));
        }
        if self.op == draw_op::UPSERT {
            let n = cols * rows;
            if !self.cells.is_empty() && self.cell_count() != n {
                return Err(format!("{} cells for a {cols}×{rows} grid", self.cell_count()));
            }
            if !self.values.is_empty() && self.values.len() != n {
                return Err(format!("{} heights for a {cols}×{rows} grid", self.values.len()));
            }
        }
        Ok(())
    }

    /// The entity with its points as `points` (decoded from `qpoints` or
    /// `ranges`, beams with no return left out) and its byte values as
    /// `values`.
    pub fn expanded(&self) -> DrawEntity {
        let mut e = self.clone();
        if e.points.is_empty() && !e.qpoints.is_empty() {
            let q = if e.quant > 0.0 { e.quant } else { DEFAULT_POINT_QUANT };
            e.points = super::draw_geom::dequantize(&e.qpoints, q);
            e.qpoints.clear();
        }
        if e.points.is_empty() && !e.ranges.is_empty() {
            e.expand_scan();
        }
        if e.kind != draw_kind::GRID && e.values.is_empty() && !e.cells.is_empty() {
            let span = e.vmax - e.vmin;
            e.values = e.cells.iter().map(|b| e.vmin + span * *b as f32 / 255.0).collect();
            e.cells.clear();
        }
        e
    }

    fn expand_scan(&mut self) {
        let q = if self.quant > 0.0 { self.quant } else { DEFAULT_RANGE_QUANT };
        let a0 = self.prop_f32("angle_min").unwrap_or(0.0);
        let inc = self.prop_f32("angle_inc").unwrap_or(0.0);
        let lo = self.prop_f32("range_min").unwrap_or(0.0);
        let hi = self.prop_f32("range_max").unwrap_or(f32::MAX);
        let keep: Vec<usize> = (0..self.ranges.len())
            .filter(|i| {
                let r = self.ranges[*i];
                r != NO_RETURN && (r as f32 * q) >= lo && (r as f32 * q) <= hi
            })
            .collect();
        self.points = keep
            .iter()
            .flat_map(|i| {
                let (r, a) = (self.ranges[*i] as f32 * q, a0 + inc * *i as f32);
                [r * a.cos(), r * a.sin(), 0.0]
            })
            .collect();
        let pick = |n: usize| n == self.ranges.len();
        if pick(self.colors.len()) {
            self.colors = keep.iter().map(|i| self.colors[*i]).collect();
        }
        if pick(self.radii.len()) {
            self.radii = keep.iter().map(|i| self.radii[*i]).collect();
        }
        if pick(self.values.len()) {
            self.values = keep.iter().map(|i| self.values[*i]).collect();
        }
        if pick(self.cells.len()) {
            self.cells = keep.iter().map(|i| self.cells[*i]).collect();
        }
        self.ranges.clear();
    }

    /// The entity as `APPEND` chunks of at most `max_bytes` section bytes
    /// each, chunk 0 carrying the header; one chunk is sent as it is. Loops
    /// split across chunks are closed and sent as strips.
    pub fn chunks(&self, max_bytes: usize) -> Vec<DrawEntity> {
        let mut entity = self.clone();
        entity.normalize();
        if entity.section_bytes() <= max_bytes || matches!(entity.op, draw_op::PATCH | draw_op::UPDATE | draw_op::EXTEND) {
            return vec![entity];
        }
        if entity.kind == draw_kind::LINES && entity.sub == line_mode::LOOP {
            entity.close_loops();
        }
        let mut chunks = match entity.kind {
            draw_kind::GRID => entity.grid_chunks(max_bytes),
            draw_kind::POLYGON => entity.polygon_chunks(max_bytes),
            _ => {
                let atoms = entity.atoms(max_bytes);
                let mut chunks: Vec<DrawEntity> = Vec::new();
                let mut current: Vec<Atom> = Vec::new();
                let mut bytes = 0;
                for atom in atoms {
                    if !current.is_empty() && bytes + atom.bytes > max_bytes {
                        chunks.push(entity.chunk_of(&current));
                        current.clear();
                        bytes = 0;
                    }
                    bytes += atom.bytes;
                    current.push(atom);
                }
                if !current.is_empty() || chunks.is_empty() {
                    chunks.push(entity.chunk_of(&current));
                }
                chunks
            }
        };
        let seq = if entity.seq != 0 { entity.seq } else { next_seq() };
        let total = chunks.len();
        let mut before = 0;
        for (i, c) in chunks.iter_mut().enumerate() {
            c.op = draw_op::APPEND;
            c.seq = seq;
            c.last = (i + 1 == total) as u32;
            c.chunk_offset = before as u32;
            before += c.item_count();
        }
        chunks
    }

    /// One-value sections moved to the header.
    pub fn normalize(&mut self) {
        if self.colors.len() == 1 && self.unit_count() != 1 {
            self.color = self.colors[0];
            self.colors.clear();
        }
        if self.radii.len() == 1 && self.unit_count() != 1 {
            self.width = 2.0 * self.radii[0];
            self.radii.clear();
        }
        if self.offsets.len() == 1 && self.offsets[0] == 0 && self.kind != draw_kind::GRID {
            self.offsets.clear();
        }
    }

    fn per_point_bytes(&self) -> usize {
        12 * !self.points.is_empty() as usize + 6 * !self.qpoints.is_empty() as usize + 2 * !self.ranges.is_empty() as usize
    }

    fn per_unit_bytes(&self) -> usize {
        4 * (!self.colors.is_empty() as usize + !self.radii.is_empty() as usize + !self.values.is_empty() as usize)
            + !self.cells.is_empty() as usize
    }

    fn section_bytes(&self) -> usize {
        4 * (self.points.len()
            + self.colors.len()
            + self.radii.len()
            + self.offsets.len()
            + self.instances.len()
            + self.values.len()
            + self.rings.len())
            + 2 * (self.qpoints.len() + self.ranges.len())
            + self.cells.len()
            + self.labels.len()
    }

    fn close_loops(&mut self) {
        let e = self.expanded_points_only();
        let mut points = Vec::with_capacity(e.points.len() + 3 * e.offsets.len().max(1));
        let mut colors = Vec::new();
        let mut radii = Vec::new();
        let mut values = Vec::new();
        let mut offsets = Vec::new();
        let per_point_values = e.values.len() == e.point_count();
        for r in e.strip_ranges() {
            offsets.push((points.len() / 3) as u32);
            for i in r.clone().chain(std::iter::once(r.start)) {
                points.extend_from_slice(&e.points[3 * i..3 * i + 3]);
                if !e.colors.is_empty() {
                    colors.push(e.colors[i]);
                }
                if !e.radii.is_empty() {
                    radii.push(e.radii[i]);
                }
                if per_point_values {
                    values.push(e.values[i]);
                }
            }
        }
        self.points = points;
        self.qpoints.clear();
        self.colors = colors;
        self.radii = radii;
        if per_point_values {
            self.values = values;
        }
        self.cells.clear();
        self.offsets = if offsets.len() > 1 { offsets } else { Vec::new() };
        self.sub = line_mode::STRIP;
    }

    /// Lines with `qpoints` and byte values decoded, for reshaping.
    fn expanded_points_only(&self) -> DrawEntity {
        let mut e = self.expanded();
        e.ranges.clear();
        e
    }

    fn atoms(&self, max_bytes: usize) -> Vec<Atom> {
        let per_point = self.per_point_bytes() + self.per_unit_bytes();
        let labels = self.label_list();
        let label_bytes = |i: usize| labels.get(i).map_or(0, |l| l.len() + 1);
        let mut out = Vec::new();
        match self.kind {
            draw_kind::LINES if self.sub != line_mode::LIST => {
                let room = ((max_bytes.saturating_sub(64)) / per_point.max(1)).max(2);
                for (s, r) in self.strip_ranges().into_iter().enumerate() {
                    let mut start = r.start;
                    loop {
                        let end = (start + room).min(r.end);
                        out.push(Atom { points: start..end, units: start..end, label: (start == r.start).then_some(s), bytes: (end - start) * per_point + 4 + label_bytes(s) });
                        if end >= r.end {
                            break;
                        }
                        start = end - 1;
                    }
                }
            }
            _ if self.instanced() => {
                let per = 4 * INSTANCE_FLOATS + self.per_unit_bytes();
                for i in 0..self.instance_count() {
                    out.push(Atom { points: 0..0, units: i..i + 1, label: Some(i), bytes: per + label_bytes(i) });
                }
            }
            _ => {
                let step = match self.kind {
                    draw_kind::LINES | draw_kind::ARROWS => 2,
                    draw_kind::TRIANGLES => 3,
                    _ => 1,
                };
                let n = self.point_count() / step * step;
                for (u, start) in (0..n).step_by(step).enumerate() {
                    let unit = if self.kind == draw_kind::ARROWS { u..u + 1 } else { start..start + step };
                    let label = (self.kind != draw_kind::LINES).then_some(u);
                    out.push(Atom { points: start..start + step, units: unit, label, bytes: step * per_point + label.map_or(0, label_bytes) });
                }
            }
        }
        out
    }

    /// A copy of the header with no sections but props.
    fn header_copy(&self) -> DrawEntity {
        DrawEntity {
            points: Vec::new(),
            colors: Vec::new(),
            radii: Vec::new(),
            offsets: Vec::new(),
            instances: Vec::new(),
            values: Vec::new(),
            rings: Vec::new(),
            qpoints: Vec::new(),
            ranges: Vec::new(),
            cells: Vec::new(),
            labels: Vec::new(),
            ..self.clone()
        }
    }

    fn chunk_of(&self, atoms: &[Atom]) -> DrawEntity {
        let mut c = self.header_copy();
        let labels = self.label_list();
        let mut chunk_labels = Vec::new();
        let strips = self.kind == draw_kind::LINES && self.sub != line_mode::LIST;
        let per_strip_values = strips && self.values.len() == self.strip_ranges().len() && self.values.len() != self.point_count();
        for atom in atoms {
            if strips {
                c.offsets.push(c.point_count() as u32);
            }
            let p = &atom.points;
            copy_range(&mut c.points, &self.points, p.start * 3..p.end * 3);
            copy_range(&mut c.qpoints, &self.qpoints, p.start * 3..p.end * 3);
            copy_range(&mut c.ranges, &self.ranges, p.clone());
            if self.instanced() {
                c.instances.extend_from_slice(&self.instances[INSTANCE_FLOATS * atom.units.start..INSTANCE_FLOATS * atom.units.end]);
            }
            copy_range(&mut c.colors, &self.colors, atom.units.clone());
            copy_range(&mut c.radii, &self.radii, atom.units.clone());
            copy_range(&mut c.cells, &self.cells, atom.units.clone());
            if per_strip_values {
                if let Some(v) = atom.label.and_then(|s| self.values.get(s)) {
                    c.values.push(*v);
                } else if let Some(v) = self.values.get(self.strip_of(atom.points.start)) {
                    c.values.push(*v);
                }
            } else {
                copy_range(&mut c.values, &self.values, atom.units.clone());
            }
            if !labels.is_empty() && strips {
                chunk_labels.push(atom.label.and_then(|s| labels.get(s).cloned()).unwrap_or_default());
            } else if let Some(l) = atom.label.and_then(|s| labels.get(s)) {
                chunk_labels.push(l.clone());
            }
        }
        if c.offsets.len() == 1 {
            c.offsets.clear();
        }
        c.labels = chunk_labels.join("\n").into_bytes();
        c
    }

    fn strip_of(&self, point: usize) -> usize {
        self.strip_ranges().iter().position(|r| r.contains(&point)).unwrap_or(0)
    }

    /// Cells in runs that fit `max_bytes`, heights along with them.
    fn grid_chunks(&self, max_bytes: usize) -> Vec<DrawEntity> {
        let size = grid_encoding::size(self.sub);
        let per = size + 4 * (!self.values.is_empty() as usize);
        let run = (max_bytes.saturating_sub(64) / per).max(1);
        let n = self.cell_count().max(self.values.len());
        (0..n.max(1))
            .step_by(run)
            .map(|start| {
                let end = (start + run).min(n);
                let mut c = self.header_copy();
                copy_range(&mut c.cells, &self.cells, start * size..end * size);
                copy_range(&mut c.values, &self.values, start..end);
                c
            })
            .collect()
    }

    /// Whole polygons per chunk; a polygon bigger than a chunk is cut
    /// between points, the next chunk carrying props `cont` (`ring` when
    /// it goes on with the last ring, `poly` with the last polygon).
    fn polygon_chunks(&self, max_bytes: usize) -> Vec<DrawEntity> {
        let per_point = self.per_point_bytes().max(1);
        let budget = max_bytes.saturating_sub(64);
        let labels = self.label_list();
        let mut chunks: Vec<DrawEntity> = Vec::new();
        let mut c = self.header_copy();
        let mut chunk_labels: Vec<String> = Vec::new();
        let mut used = 0usize;
        let header = self.header_copy();
        let finish = |chunks: &mut Vec<DrawEntity>, c: &mut DrawEntity, chunk_labels: &mut Vec<String>, next: Option<&str>| {
            let mut done = std::mem::replace(c, header.clone());
            if done.offsets.len() == 1 {
                done.offsets.clear();
            }
            done.labels = std::mem::take(chunk_labels).join("\n").into_bytes();
            chunks.push(done);
            if let Some(cont) = next {
                *c = header.clone().with_prop("cont", cont);
                c.rings.push(0);
            }
        };
        let polygons = self.polygon_rings();
        for (k, rings) in polygons.iter().enumerate() {
            let unit_bytes = self.per_unit_bytes() + labels.get(k).map_or(0, |l| l.len() + 1) + 4;
            let points: usize = rings.iter().map(|r| r.len()).sum();
            if used > 0 && used + unit_bytes + points * per_point > budget {
                finish(&mut chunks, &mut c, &mut chunk_labels, None);
                used = 0;
            }
            copy_range(&mut c.colors, &self.colors, k..k + 1);
            copy_range(&mut c.radii, &self.radii, k..k + 1);
            copy_range(&mut c.values, &self.values, k..k + 1);
            copy_range(&mut c.cells, &self.cells, k..k + 1);
            if let Some(l) = labels.get(k) {
                chunk_labels.push(l.clone());
            }
            used += unit_bytes;
            c.rings.push(0);
            for (j, ring) in rings.iter().enumerate() {
                let mut start = ring.start;
                loop {
                    let room = (budget.saturating_sub(used) / per_point).max(3);
                    let end = (start + room).min(ring.end);
                    c.offsets.push(c.point_count() as u32);
                    copy_range(&mut c.points, &self.points, start * 3..end * 3);
                    copy_range(&mut c.qpoints, &self.qpoints, start * 3..end * 3);
                    *c.rings.last_mut().unwrap() += 1;
                    used += (end - start) * per_point;
                    start = end;
                    if start >= ring.end {
                        break;
                    }
                    finish(&mut chunks, &mut c, &mut chunk_labels, Some("ring"));
                    used = 0;
                }
                if used + 3 * per_point > budget && j + 1 < rings.len() {
                    finish(&mut chunks, &mut c, &mut chunk_labels, Some("poly"));
                    used = 0;
                }
            }
        }
        if c.point_count() > 0 || chunks.is_empty() {
            finish(&mut chunks, &mut c, &mut chunk_labels, None);
        }
        for chunk in &mut chunks {
            if chunk.rings.len() == 1 && chunk.props().get("cont").is_none() {
                chunk.rings.clear();
            }
        }
        chunks
    }

    /// Whether a polygon chunk goes on with the pending entity's last
    /// polygon (props `cont`).
    pub fn continues(&self) -> Option<String> {
        self.props().get("cont").filter(|c| !c.is_empty())
    }
}

fn copy_range<T: Copy>(dst: &mut Vec<T>, src: &[T], range: Range<usize>) {
    if !src.is_empty() && range.end <= src.len() {
        dst.extend_from_slice(&src[range]);
    }
}

/// An indivisible run of an entity's items while chunking.
struct Atom {
    points: Range<usize>,
    units: Range<usize>,
    label: Option<usize>,
    bytes: usize,
}

fn next_seq() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let base = (std::process::id() & 0xffff) << 16;
    base | (SEQ.fetch_add(1, Ordering::Relaxed) & 0xffff).max(1)
}
