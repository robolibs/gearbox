//! Validated scene field layout and rectangular region partitioning.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_regions_cover_the_domain_without_overlap() {
        let layout: FieldLayout = serde_json::from_str(include_str!("layouts/mixed.json")).unwrap();
        let domain = FieldBounds {
            min: Vec2::splat(-400.0),
            max: Vec2::splat(400.0),
        };
        let regions = layout.regions(domain);
        let area: f32 = regions
            .iter()
            .map(|field| {
                let bounds = field.bounds();
                assert!(domain.contains(bounds.min) && domain.contains(bounds.max));
                let size = bounds.max - bounds.min;
                size.x * size.y
            })
            .sum();
        assert!((area - 640_000.0).abs() < 0.1);
        for (i, field) in regions.iter().enumerate() {
            for other in &regions[..i] {
                assert_ne!(field.name, other.name);
                assert!(!field.bounds().overlaps(other.bounds()));
            }
        }
    }

    #[test]
    fn default_layout_is_the_bundled_mixed_layout() {
        let domain = FieldBounds {
            min: Vec2::splat(-400.0),
            max: Vec2::splat(400.0),
        };
        let regions = FieldLayout::default().regions(domain);
        assert!(regions.iter().any(|field| field.profile == "grassland"));
        assert!(regions.iter().any(|field| field.profile == "harvested_wheat"));
    }

    // The shader reads the points by position out of the matrix columns, so the
    // packing is a contract between two files and not an implementation detail.
    #[test]
    fn a_way_packs_two_points_to_a_column() {
        let points: Vec<Vec2> = (0..8).map(|i| Vec2::new(i as f32, 10.0 + i as f32)).collect();
        let (matrix, shape) = Way::bend(&points, 2.5).packed();
        assert_eq!(shape.x, 8.0);
        assert_eq!(shape.y, 2.5);
        for (i, point) in points.iter().enumerate() {
            let column = matrix.col(i / 2);
            let packed = if i % 2 == 0 { column.xy() } else { column.zw() };
            assert_eq!(packed, *point);
        }
    }

    #[test]
    fn a_way_needs_two_points_and_keeps_at_most_eight() {
        assert_eq!(Way::bend(&[Vec2::ZERO], 1.0).points(), 0);
        assert_eq!(Way::straight().points(), 0);
        let many: Vec<Vec2> = (0..12).map(|i| Vec2::splat(i as f32)).collect();
        assert_eq!(Way::bend(&many, 1.0).points(), Way::MOST as u32);
    }

    // Points of a way, in order, read back the way a shader reads them.
    fn line_of(way: &Way) -> Vec<Vec2> {
        let (points, shape) = way.packed();
        (0..shape.x as usize)
            .map(|i| {
                let column = points.col(i / 2);
                if i % 2 == 0 { column.xy() } else { column.zw() }
            })
            .collect()
    }

    // The whole point of clipping rather than resampling: two fields either
    // side of a boundary must describe the road with the *same* points, or it
    // kinks where they meet. The arc offset has to be right too, or the ruts
    // restart their wander on the boundary and step sideways there.
    #[test]
    fn two_fields_clip_one_road_to_the_same_line() {
        let road: Vec<Vec2> = (0..12).map(|i| Vec2::new(i as f32 * 10.0, 0.0)).collect();
        let west = FieldBounds { min: Vec2::new(0.0, -20.0), max: Vec2::new(50.0, 20.0) };
        let east = FieldBounds { min: Vec2::new(50.0, -20.0), max: Vec2::new(110.0, 20.0) };
        for bounds in [west, east] {
            let way = Way::clipped(&road, 3.0, bounds).unwrap();
            let kept = line_of(&way);
            // A contiguous run of the road itself — nothing moved or resampled.
            let at = road.iter().position(|p| *p == kept[0]).unwrap();
            assert_eq!(kept, road[at..at + kept.len()]);
            // The road runs along x from the origin, so arc length is just x.
            assert_eq!(way.packed().1.z, kept[0].x);
        }
        let west_line = line_of(&Way::clipped(&road, 3.0, west).unwrap());
        let east_line = line_of(&Way::clipped(&road, 3.0, east).unwrap());
        let shared: Vec<Vec2> =
            west_line.iter().copied().filter(|p| east_line.contains(p)).collect();
        assert!(!shared.is_empty(), "the two fields must overlap on the road");
    }

    // The chunk cull tells a chunk it is unworn when the way does not reach it.
    // If that were ever stricter than the clip, wear would vanish from chunks
    // that should have it and the road would break into dashes.
    #[test]
    fn a_way_reaches_exactly_where_it_clips() {
        let road: Vec<Vec2> =
            (0..6).map(|i| Vec2::new(i as f32 * 20.0, (i % 2) as f32 * 8.0)).collect();
        let way = Way::bend(&road, 3.0);
        for x in (-40..160).step_by(8) {
            for z in (-40..60).step_by(8) {
                let chunk = FieldBounds {
                    min: Vec2::new(x as f32, z as f32),
                    max: Vec2::new(x as f32 + 16.0, z as f32 + 16.0),
                };
                assert_eq!(
                    way.reaches(chunk),
                    Way::clipped(&road, 3.0, chunk).is_some(),
                    "chunk at {x},{z}"
                );
            }
        }
    }

    // The hollow sinks the terrain grid, which is what the collider is built
    // from, so getting its shape wrong is felt and not just seen.
    #[test]
    fn a_hollow_is_deepest_on_the_way_and_nothing_off_it() {
        let layout: FieldLayout = serde_json::from_str(
            r#"{"default":"grassland","ways":[{"name":"lane","width":6.0,"wear":0.5,
                "points":[[0,0],[100,0]]}]}"#,
        )
        .unwrap();
        let hollows = Hollows::of(&layout);
        let on_it = hollows.depth_at(50.0, 0.0);
        // 0.08 + 0.5 * 0.35, the sink a half-worn way settles to.
        assert!((on_it - 0.255).abs() < 1e-5, "on the way: {on_it}");
        // A wheel must ride in and out, so the steepest side stays gentle.
        let slope = (0..60)
            .map(|step| {
                let at = step as f32 * 0.1;
                (hollows.depth_at(50.0, at) - hollows.depth_at(50.0, at + 0.1)).abs() / 0.1
            })
            .fold(0.0f32, f32::max);
        assert!(slope < 0.2, "sides of one in {:.0}", 1.0 / slope);
        // Level with the field again well outside the way and its sides.
        assert_eq!(hollows.depth_at(50.0, 3.0 + FADE_M + 0.1), 0.0);
        assert_eq!(hollows.depth_at(50.0, 400.0), 0.0);
        assert_eq!(hollows.depth_at(-400.0, 0.0), 0.0);
        // Sides that fall away, never a step.
        let mut last = on_it;
        for step in 1..40 {
            let here = hollows.depth_at(50.0, step as f32 * 0.15);
            assert!(here <= last + 1e-6, "the sides must not rise again");
            assert!(last - here < 0.05, "no step in the sides");
            last = here;
        }
        assert!(Hollows::of(&FieldLayout::default()).depth_at(0.0, 0.0) >= 0.0);
    }

    // The same track authored two ways — as a road over the layout, or as one
    // field's own way — must sink the ground identically, or a lane changes
    // depth depending on which file it happens to be written in.
    #[test]
    fn a_field_way_sinks_like_a_layout_road() {
        let road: FieldLayout = serde_json::from_str(
            r#"{"default":"grassland","ways":[{"name":"lane","width":6.0,"wear":0.7,
                "points":[[0,0],[100,0]]}]}"#,
        )
        .unwrap();
        let lane: FieldLayout = serde_json::from_str(
            r#"{"default":"grassland","fields":[{"name":"lane","profile":"track",
                "min":[0,-20],"max":[100,20],"wear":0.7,"way_width":6.0,
                "way":[[0,0],[100,0]]}]}"#,
        )
        .unwrap();
        let (a, b) = (Hollows::of(&road), Hollows::of(&lane));
        for step in 0..30 {
            let z = step as f32 * 0.3;
            assert_eq!(a.depth_at(50.0, z), b.depth_at(50.0, z), "at z={z}");
        }
        assert!(a.depth_at(50.0, 0.0) > 0.3);
    }

    #[test]
    fn a_field_with_no_way_of_its_own_sinks_nothing() {
        let plain: FieldLayout = serde_json::from_str(
            r#"{"default":"grassland","fields":[{"name":"plot","profile":"ploughed",
                "min":[0,0],"max":[100,100]}]}"#,
        )
        .unwrap();
        assert!(Hollows::of(&plain).is_empty());
    }

    #[test]
    fn a_road_that_misses_a_field_clips_to_nothing() {
        let road = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
        let away = FieldBounds { min: Vec2::new(0.0, 60.0), max: Vec2::new(100.0, 90.0) };
        assert!(Way::clipped(&road, 3.0, away).is_none());
    }

    #[test]
    fn a_layout_way_defaults_to_the_field_width() {
        let spec: FieldSpec = serde_json::from_str(
            r#"{"name":"lane","profile":"track","min":[0,0],"max":[10,100],
                "way":[[5,0],[5,100]]}"#,
        )
        .unwrap();
        assert_eq!(spec.way().packed().1.y, 5.0);
    }
}

use super::profile::FieldProfiles;
use bevy::prelude::*;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldBounds {
    pub min: Vec2,
    pub max: Vec2,
}

impl Default for FieldBounds {
    fn default() -> Self {
        Self { min: Vec2::ZERO, max: Vec2::ZERO }
    }
}

impl FieldBounds {
    pub fn contains(&self, point: Vec2) -> bool {
        point.cmpge(self.min).all() && point.cmple(self.max).all()
    }

    pub fn overlaps(&self, other: Self) -> bool {
        self.min.cmplt(other.max).all() && other.min.cmplt(self.max).all()
    }

    pub fn nearest_distance(&self, point: Vec2) -> f32 {
        point.distance(point.clamp(self.min, self.max))
    }
}

/// The line the wheels follow through a field, in world XZ. A field is always a
/// rectangle, so without this a way can only run straight down one; with it the
/// field is merely the corridor a track winds along inside.
///
/// A road longer than one field is clipped to each field it crosses, and `arc`
/// is how far along the whole road the first kept point lies. The shader adds
/// it back, so the ruts wander by one noise down the road's whole length rather
/// than restarting at every boundary and stepping sideways there.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Way {
    points: [Vec2; Way::MOST],
    count: u32,
    half_width: f32,
    arc: f32,
}

impl Way {
    pub const MOST: usize = 8;

    /// Nothing, for a field whose wear runs down its own long axis.
    pub fn straight() -> Self {
        Self::default()
    }

    /// How many points bend it; fewer than two is no way at all.
    pub fn points(&self) -> u32 {
        self.count
    }

    /// Fewer than two points is no way at all; beyond `MOST` the tail is cut,
    /// which is said out loud rather than authored points going quietly missing.
    pub fn bend(points: &[Vec2], half_width: f32) -> Self {
        let mut way = Self { half_width, ..Self::default() };
        if points.len() < 2 {
            return way;
        }
        if points.len() > Self::MOST {
            warn!(
                "a way of {} points keeps only its first {}; the rest of it is cut",
                points.len(),
                Self::MOST
            );
        }
        let taken = points.len().min(Self::MOST);
        way.points[..taken].copy_from_slice(&points[..taken]);
        way.count = taken as u32;
        way
    }

    /// The stretch of a longer road that `bounds` can see, or `None` where the
    /// road does not come near this field at all. Both sides of a boundary clip
    /// the *same* points, so the two halves of a road meet exactly; only the
    /// ends are cut, never moved, and nothing is resampled.
    pub fn clipped(points: &[Vec2], half_width: f32, bounds: FieldBounds) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        // Far enough out that the verge and its noise are still decided by the
        // real line and not by where the clip happened to fall.
        let reach = Vec2::splat(half_width + 2.0);
        let (min, max) = (bounds.min - reach, bounds.max + reach);
        let seen: Vec<usize> = (0..points.len() - 1)
            .filter(|&i| segment_meets(points[i], points[i + 1], min, max))
            .collect();
        let (&first, &last) = (seen.first()?, seen.last()?);
        // One leg beyond each end, so the way still has a direction at the
        // field's edge instead of stopping dead on it.
        let start = first.saturating_sub(1);
        let end = (last + 2).min(points.len() - 1);
        let kept = &points[start..=end];
        let arc = points[..=start].windows(2).map(|p| p[0].distance(p[1])).sum();
        if kept.len() > Self::MOST {
            warn!(
                "way of {} points needs {} of them over [{:?}..{:?}]; only {} fit, so the \
                 road is cut short there — split the field or use fewer points",
                points.len(), kept.len(), bounds.min, bounds.max, Self::MOST
            );
        }
        Some(Self { arc, ..Self::bend(kept, half_width) })
    }

    /// Whether this way comes near enough to `bounds` to wear any of it. The
    /// shader's search costs a loop per blade and per pixel, so a chunk the
    /// road never touches is told it has no wear at all and pays one compare.
    pub fn reaches(&self, bounds: FieldBounds) -> bool {
        let reach = Vec2::splat(self.half_width + 2.0);
        let (min, max) = (bounds.min - reach, bounds.max + reach);
        let count = self.count as usize;
        (0..count.saturating_sub(1))
            .any(|i| segment_meets(self.points[i], self.points[i + 1], min, max))
    }

    /// For a shader: the points two to a column, then how many, half the width,
    /// and how far along the whole road the first of them lies.
    pub fn packed(&self) -> (Mat4, Vec4) {
        let pair = |i: usize| {
            let (a, b) = (self.points[i * 2], self.points[i * 2 + 1]);
            Vec4::new(a.x, a.y, b.x, b.y)
        };
        (
            Mat4::from_cols(pair(0), pair(1), pair(2), pair(3)),
            Vec4::new(self.count as f32, self.half_width, self.arc, 0.0),
        )
    }
}

/// Whether a segment comes inside an axis-aligned box, by clipping it against
/// each pair of slabs in turn.
fn segment_meets(a: Vec2, b: Vec2, min: Vec2, max: Vec2) -> bool {
    let run = b - a;
    let (mut enter, mut leave) = (0.0f32, 1.0f32);
    for axis in 0..2 {
        if run[axis].abs() < 1e-6 {
            if a[axis] < min[axis] || a[axis] > max[axis] {
                return false;
            }
            continue;
        }
        let near = (min[axis] - a[axis]) / run[axis];
        let far = (max[axis] - a[axis]) / run[axis];
        enter = enter.max(near.min(far));
        leave = leave.min(near.max(far));
        if enter > leave {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    pub name: String,
    pub profile: String,
    pub min: [f32; 2],
    pub max: [f32; 2],
    /// How hard this field is worn, nought to one: a green lane barely marked,
    /// two bare ruts with grass between them, or bare across its width. Left
    /// out, the profile decides.
    #[serde(default)]
    pub wear: Option<f32>,
    /// World XZ points of the line the wheels follow, at most eight. Left out,
    /// the wear runs straight down the field's long axis.
    #[serde(default)]
    pub way: Vec<[f32; 2]>,
    /// How wide the worn corridor is, in metres. Left out, it is as wide as the
    /// field is across.
    #[serde(default)]
    pub way_width: Option<f32>,
}

impl FieldSpec {
    pub fn bounds(&self) -> FieldBounds {
        FieldBounds {
            min: Vec2::from_array(self.min),
            max: Vec2::from_array(self.max),
        }
    }

    pub fn way(&self) -> Way {
        let (line, width, _) = self.laid_way();
        Way::bend(&line, width * 0.5)
    }

    /// The line this field lays down for itself, how wide it is worn and how
    /// hard — the same three things a layout's road carries, so the two sink
    /// the ground alike. A field that names a way but no wear is taken as
    /// half worn, since only its profile knows better and this does not.
    fn laid_way(&self) -> (Vec<Vec2>, f32, f32) {
        let span = self.bounds().max - self.bounds().min;
        let across = span.x.min(span.y);
        (
            self.way.iter().copied().map(Vec2::from_array).collect(),
            self.way_width.unwrap_or(across),
            self.wear.unwrap_or(0.5),
        )
    }
}

/// A road laid across the whole layout rather than down one field. Every field
/// it passes over is worn along it, whatever that field grows, so a track can
/// run from one end of the country to the other without the fields being cut
/// up to describe it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaySpec {
    pub name: String,
    /// World XZ points of the line, as many as the road needs.
    pub points: Vec<[f32; 2]>,
    /// How wide the worn corridor is, in metres.
    pub width: f32,
    /// How hard it is worn, nought to one, for the fields it crosses that do
    /// not say for themselves.
    pub wear: f32,
}

impl WaySpec {
    fn line(&self) -> Vec<Vec2> {
        self.points.iter().copied().map(Vec2::from_array).collect()
    }

    /// The stretch of this road a field can see, if it crosses that field.
    pub fn across(&self, bounds: FieldBounds) -> Option<Way> {
        Way::clipped(&self.line(), self.width * 0.5, bounds)
    }
}

/// How deep the ways of a layout sit below the ground around them, as a plain
/// function of world XZ. A track used for years is a hollow, not a stripe of
/// colour on a flat field: this is what lets one break the skyline at a
/// grazing angle, and what a wheel feels when it drops into one — the terrain
/// grid carries the collider, so sinking it here sinks it for the physics too.
///
/// Only an authored line sinks anything. A field that merely carries a `wear`
/// is worn across its whole width, and that is a shading of the ground, not a
/// trench dug down the middle of it: guessing which wide fields were meant to
/// be lanes would sink a ploughed plot along its long axis.
#[derive(Clone, Debug, Default)]
pub struct Hollows(Vec<Hollow>);

#[derive(Clone, Debug)]
struct Hollow {
    line: Vec<Vec2>,
    half: f32,
    sink: f32,
    min: Vec2,
    max: Vec2,
}

impl Hollow {
    fn new((line, width, wear): (Vec<Vec2>, f32, f32)) -> Option<Self> {
        if line.len() < 2 || width <= 0.0 {
            return None;
        }
        let half = width * 0.5;
        let reach = Vec2::splat(half + FADE_M);
        let fold = |pick: fn(Vec2, Vec2) -> Vec2| line.iter().copied().reduce(pick).unwrap();
        Some(Self {
            half,
            // Barely marked where a lane is hardly worn, a proper sunken way
            // where it is a road. Even at its deepest the sides are gentler
            // than one in five, so a wheel rides in and out of it rather than
            // catching on the lip.
            sink: 0.08 + wear.clamp(0.0, 1.0) * 0.35,
            min: fold(Vec2::min) - reach,
            max: fold(Vec2::max) + reach,
            line,
        })
    }
}

impl Hollows {
    /// A metre of cell means the ruts themselves can never be geometry; the
    /// trough the whole way sits in is several metres across and can.
    pub fn of(layout: &FieldLayout) -> Self {
        // A road laid over the layout and a way a single field names for itself
        // are the same thing authored at two scales; both sink what they cross.
        let roads = layout.ways.iter().map(|way| (way.line(), way.width, way.wear));
        let lanes = layout.fields.iter().map(FieldSpec::laid_way);
        Self(roads.chain(lanes).filter_map(Hollow::new).collect())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How far the ground drops here, in metres; nought away from every way.
    pub fn depth_at(&self, x: f32, z: f32) -> f32 {
        let place = Vec2::new(x, z);
        let mut deepest = 0.0f32;
        for hollow in &self.0 {
            // Nearly every point of a square kilometre is nowhere near a road,
            // and the walk down its legs is far dearer than four compares.
            if place.cmplt(hollow.min).any() || place.cmpgt(hollow.max).any() {
                continue;
            }
            let mut nearest = f32::MAX;
            for leg in hollow.line.windows(2) {
                let run = leg[1] - leg[0];
                let length = run.length().max(1e-4);
                let at = (place - leg[0]).dot(run / length).clamp(0.0, length);
                nearest = nearest.min(place.distance(leg[0] + run / length * at));
            }
            let sides = 1.0 - smoothstep(hollow.half * 0.55, hollow.half + FADE_M, nearest);
            deepest = deepest.max(hollow.sink * sides);
        }
        deepest
    }
}

/// How far out a hollow's sides run past the worn part of the way.
const FADE_M: f32 = 1.6;

fn smoothstep(from: f32, to: f32, at: f32) -> f32 {
    let t = ((at - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Resource, Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldLayout {
    pub default: String,
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
    /// Roads laid over the fields; the first one a field meets wears it.
    #[serde(default)]
    pub ways: Vec<WaySpec>,
}

impl Default for FieldLayout {
    fn default() -> Self {
        // Grass everywhere, with a harvested-wheat stubble region cut into
        // it — the bundled `mixed.json` is the single source of truth for
        // this so the default layout can't drift from its own test fixture.
        serde_json::from_str(include_str!("layouts/mixed.json"))
            .expect("bundled default field layout must be valid")
    }
}

impl FieldLayout {
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("GEARBOX_FIELD_LAYOUT") {
            Ok(path) => {
                let contents = std::fs::read_to_string(&path)
                    .map_err(|error| format!("field layout {path}: {error}"))?;
                serde_json::from_str(&contents)
                    .map_err(|error| format!("field layout {path}: {error}"))
            }
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(error) => Err(format!("GEARBOX_FIELD_LAYOUT: {error}")),
        }
    }

    pub fn validate(&self, profiles: &FieldProfiles) -> Result<(), String> {
        if !profiles.0.contains_key(&self.default) {
            return Err(format!("unknown default field profile: {}", self.default));
        }
        for (i, field) in self.fields.iter().enumerate() {
            let bounds = field.bounds();
            if field.name.trim().is_empty()
                || field.name.starts_with("__background/")
                || self.fields[..i].iter().any(|f| f.name == field.name)
            {
                return Err(format!("empty or duplicate field name: {}", field.name));
            }
            if !profiles.0.contains_key(&field.profile) {
                return Err(format!("unknown field profile: {}", field.profile));
            }
            if !bounds.min.is_finite()
                || !bounds.max.is_finite()
                || !bounds.min.cmplt(bounds.max).all()
            {
                return Err(format!("invalid bounds for {}", field.name));
            }
            if self.fields[..i]
                .iter()
                .any(|other| bounds.overlaps(other.bounds()))
            {
                return Err(format!("overlapping field: {}", field.name));
            }
        }
        Ok(())
    }

    /// The fields as they fall on `terrain`, cut to it, and the default cover
    /// over the rest of it. The terrain may lie anywhere over the layout.
    pub fn regions(&self, terrain: FieldBounds) -> Vec<FieldSpec> {
        let fields: Vec<FieldSpec> = self
            .fields
            .iter()
            .filter(|field| field.bounds().overlaps(terrain))
            .map(|field| FieldSpec {
                min: field.bounds().min.max(terrain.min).to_array(),
                max: field.bounds().max.min(terrain.max).to_array(),
                ..field.clone()
            })
            .collect();
        let mut xs = vec![terrain.min.x, terrain.max.x];
        let mut zs = vec![terrain.min.y, terrain.max.y];
        for field in &fields {
            xs.extend([field.min[0], field.max[0]]);
            zs.extend([field.min[1], field.max[1]]);
        }
        xs.sort_by(f32::total_cmp);
        zs.sort_by(f32::total_cmp);
        xs.dedup();
        zs.dedup();
        let mut regions = fields.clone();
        for z in zs.windows(2) {
            let mut start = None;
            for x in xs.windows(2) {
                let middle = Vec2::new((x[0] + x[1]) * 0.5, (z[0] + z[1]) * 0.5);
                let occupied = fields
                    .iter()
                    .any(|field| field.bounds().contains(middle));
                if occupied {
                    if let Some(left) = start.take() {
                        regions.push(self.background_region(left, x[0], z[0], z[1], regions.len()));
                    }
                } else {
                    start.get_or_insert(x[0]);
                }
            }
            if let Some(left) = start {
                regions.push(self.background_region(
                    left,
                    terrain.max.x,
                    z[0],
                    z[1],
                    regions.len(),
                ));
            }
        }
        regions
    }

    fn background_region(&self, x0: f32, x1: f32, z0: f32, z1: f32, index: usize) -> FieldSpec {
        FieldSpec {
            name: format!("__background/{index}"),
            profile: self.default.clone(),
            min: [x0, z0],
            max: [x1, z1],
            wear: None,
            way: Vec::new(),
            way_width: None,
        }
    }
}
