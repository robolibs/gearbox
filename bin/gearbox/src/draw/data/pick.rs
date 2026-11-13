//! Data picking for the main view: the nearest cloud sample or octree cell under
//! the cursor exactly as this frame draws it (tier, instances, clipping, glyph
//! coverage), in front of the nearest terrain or collider along the ray.
//!
//! A click queues a [`Pick`]; `PostUpdate` adds the occluder depth while the physics
//! world is home; the render world resolves it after the frame's selection and
//! returns a [`Found`]. A pick whose camera moved before its frame was drawn is
//! answered stale rather than resolved against a view the user never saw.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::view::ExtractedView;
use bevy::render::{Render, RenderApp, RenderSystems};
use gearbox_api::wire::draw_data::{
    AttributeSemantic, ColumnView, DataKey, DataShape, DatasetRevision, Normalization,
    OctreeConfig, PointSize, ScalarType, VoxelKey,
};

use super::coarse::Tier;
use super::render::selection::Views;

#[cfg(test)]
#[path = "pick_tests.rs"]
mod tests;

/// Picks in flight for the main view (profile `max_inflight_picks_per_view`); one more supersedes the oldest.
pub(crate) const MAX_PICKS: usize = 2;
/// Camera motion between the click and the drawn frame that makes a pick stale.
const STILL_M: f32 = 1e-3;
const STILL_RAD: f32 = 1e-4;
/// Farthest occluder searched along the ray, in metres.
const OCCLUDER_RANGE_M: f32 = 4000.0;

/// One click: the camera as the user saw it and the cursor in NDC (y up).
#[derive(Clone, Debug)]
pub(crate) struct Pick {
    pub id: u64,
    pub camera: Entity,
    pub eye: GlobalTransform,
    pub fov: f32,
    pub aspect: f32,
    pub ndc: Vec2,
    /// Namespace prefix the pick may return; empty for any.
    pub prefix: String,
    /// View depth of the nearest terrain or collider, infinite for none.
    pub occluder: f32,
}

impl Pick {
    pub fn direction(&self) -> Vec3 {
        let t = (0.5 * self.fov).tan();
        let view = Vec3::new(self.ndc.x * t * self.aspect, self.ndc.y * t, -1.0).normalize();
        (self.eye.rotation() * view).normalize()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CloudHit {
    pub key: DataKey,
    pub revision: DatasetRevision,
    pub scan: u64,
    pub page: u64,
    pub source_id: Option<u64>,
    /// Drawn as a coarse representative rather than the page's full samples.
    pub coarse: bool,
    /// Render-space centre of the hit sample.
    pub at: Vec3,
    /// The sample's source coordinates in its dataset frame.
    pub source_m: Option<[f64; 3]>,
    /// `(name, value, units)` of every other attribute; `invalid` when unobserved.
    pub attributes: Vec<(String, String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OctreeHit {
    pub key: DataKey,
    pub revision: DatasetRevision,
    pub leaf: VoxelKey,
    pub state: &'static str,
    pub probability: Option<f32>,
    /// A renderer summary below the display depth limit, not a stored leaf.
    pub aggregate: bool,
    /// Unknown space shown inside the ROI, not a stored leaf.
    pub implicit: bool,
    pub at: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SurfaceHit {
    pub key: DataKey,
    pub revision: DatasetRevision,
    /// `surfels` or `mesh`.
    pub kind: &'static str,
    pub page: u64,
    /// A surfel's source ID, or the triangle's index in its block.
    pub item: Option<u64>,
    pub normal_supplied: bool,
    pub confidence: Option<f32>,
    pub at: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FieldHit {
    pub key: DataKey,
    pub revision: DatasetRevision,
    pub slice: u64,
    /// The sampled field value, or none where its support is unknown.
    pub value: Option<f32>,
    pub at: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainHit {
    pub key: DataKey,
    pub revision: DatasetRevision,
    pub tile: [i32; 2],
    /// The quad's lowest sample within its tile.
    pub cell: [u32; 2],
    /// Mean elevation of the quad's four samples.
    pub elevation: f32,
    pub at: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Found {
    Cloud(CloudHit),
    Octree(OctreeHit),
    Surface(SurfaceHit),
    Field(FieldHit),
    Terrain(TerrainHit),
    Nothing,
    Stale(&'static str),
}

#[derive(Default)]
pub(crate) struct Exchange {
    issued: u64,
    waiting: VecDeque<Pick>,
    ready: VecDeque<Pick>,
    found: VecDeque<(u64, Found)>,
}

/// A request waiting for its answer: the interaction entity and the site the
/// pick was made against, to answer in the entity's frame.
#[derive(Clone)]
pub(crate) struct Pending {
    pub ns: String,
    pub id: String,
    pub frame: gearbox_api::DrawFrame,
    pub site: GlobalTransform,
    pub home: crate::globe::SiteEntry,
    pub ndc: Vec2,
    /// Leave a world-anchored label with the hit's details at it.
    pub inspect: bool,
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub(crate) struct Picks(Arc<Mutex<Exchange>>);

impl Picks {
    fn lock(&self) -> MutexGuard<'_, Exchange> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues a click; past [`MAX_PICKS`] the oldest is answered stale.
    pub fn submit(&self, pick: Pick) {
        let mut exchange = self.lock();
        if exchange.waiting.len() + exchange.ready.len() >= MAX_PICKS
            && let Some(old) = exchange.ready.pop_front().or_else(|| exchange.waiting.pop_front())
        {
            exchange.found.push_back((old.id, Found::Stale("superseded by a newer pick")));
        }
        exchange.waiting.push_back(pick);
    }

    pub fn cancel(&self, id: u64) {
        let mut exchange = self.lock();
        exchange.waiting.retain(|p| p.id != id);
        exchange.ready.retain(|p| p.id != id);
        exchange.found.retain(|(found, _)| *found != id);
    }

    pub fn next_id(&self) -> u64 {
        let mut exchange = self.lock();
        exchange.issued += 1;
        exchange.issued
    }

    pub fn take(&self) -> Vec<(u64, Found)> {
        self.lock().found.drain(..).collect()
    }

    /// The answers to the picks `owned` claims, leaving the others queued.
    pub fn take_owned(&self, owned: impl Fn(u64) -> bool) -> Vec<(u64, Found)> {
        let mut exchange = self.lock();
        let (mine, rest) = exchange.found.drain(..).partition(|(id, _)| owned(*id));
        exchange.found = rest;
        mine.into()
    }

    pub fn pending(&self) -> usize {
        let exchange = self.lock();
        exchange.waiting.len() + exchange.ready.len()
    }
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Picks>()
        .add_plugins(ExtractResourcePlugin::<Picks>::default())
        .add_systems(
            PostUpdate,
            (api_picks, occlude)
                .chain()
                .after(bevy::transform::TransformSystems::Propagate),
        );
    app.sub_app_mut(RenderApp).add_systems(
        Render,
        resolve
            .after(super::receipts::prepare_frame)
            .in_set(RenderSystems::QueueMeshes),
    );
}

/// The dataset's stored revision now.
pub(crate) fn current(world: &World, key: &DataKey) -> Option<DatasetRevision> {
    let shared = world.get_resource::<super::runtime::Shared>()?;
    let registry = shared.backend.registry();
    let registry = registry.lock().unwrap_or_else(PoisonError::into_inner);
    registry.get_parts(&key.namespace, &key.id).map(|entry| entry.revision)
}

/// A hit on a generation that is no longer the stored one becomes stale.
pub(crate) fn verdict(found: Found, current: impl Fn(&DataKey) -> Option<DatasetRevision>) -> Found {
    let hit = match &found {
        Found::Cloud(hit) => Some((&hit.key, hit.revision)),
        Found::Octree(hit) => Some((&hit.key, hit.revision)),
        Found::Surface(hit) => Some((&hit.key, hit.revision)),
        Found::Field(hit) => Some((&hit.key, hit.revision)),
        Found::Terrain(hit) => Some((&hit.key, hit.revision)),
        Found::Nothing | Found::Stale(_) => None,
    };
    match hit {
        Some((key, revision)) if current(key) != Some(revision) => Found::Stale("the dataset changed before the answer"),
        _ => found,
    }
}

/// Answers `pending` with what its pick found, as a draw event in the
/// interaction's frame, and removes the interaction.
pub(crate) fn publish(world: &mut World, pending: &Pending, found: Found) {
    use gearbox_api::{DrawEntity, DrawStoreRes, GearboxBus, SceneEvent, event_kind};
    let to_entity = super::super::site_transform(&pending.frame, Transform::IDENTITY, &pending.home)
        .unwrap_or_else(super::super::enu_to_site)
        .compute_affine()
        .inverse();
    let place = |at: Vec3| to_entity.transform_point3(pending.site.affine().inverse().transform_point3(at));
    let epoch = world.get_resource::<GearboxBus>().map_or(0, |bus| bus.host.draw_data_lane().epoch());
    let identity = |key: &DataKey, revision: DatasetRevision, kind: &str| {
        vec![
            ("kind".to_string(), kind.to_string()),
            ("data_ns".to_string(), key.namespace.clone()),
            ("data_id".to_string(), key.id.clone()),
            ("host_epoch".to_string(), epoch.to_string()),
            ("incarnation".to_string(), revision.incarnation.to_string()),
            ("generation".to_string(), revision.generation.to_string()),
        ]
    };
    let found = verdict(found, |key| current(world, key));
    let (what, at, mut props) = match found {
        Found::Cloud(hit) => {
            let mut props = identity(&hit.key, hit.revision, "cloud");
            props.push(("scan".into(), hit.scan.to_string()));
            props.push(("page".into(), hit.page.to_string()));
            props.push(("source_id".into(), hit.source_id.map_or("none".into(), |id| id.to_string())));
            props.push(("lod".into(), if hit.coarse { "coarse" } else { "full" }.into()));
            if let Some(m) = hit.source_m {
                props.push(("source_m".into(), format!("{} {} {}", m[0], m[1], m[2])));
            }
            for (name, value, units) in hit.attributes {
                props.push((format!("unit.{name}"), units));
                props.push((format!("attr.{name}"), value));
            }
            ("data", place(hit.at), props)
        }
        Found::Octree(hit) => {
            let mut props = identity(&hit.key, hit.revision, "octree");
            let [x, y, z] = hit.leaf.tile;
            props.push(("leaf".into(), format!("{x},{y},{z}/{}/{}", hit.leaf.depth, hit.leaf.morton)));
            props.push(("state".into(), hit.state.into()));
            props.push(("probability".into(), hit.probability.map_or("none".into(), |p| p.to_string())));
            let lod = if hit.implicit { "implicit" } else if hit.aggregate { "aggregate" } else { "leaf" };
            props.push(("lod".into(), lod.into()));
            ("data", place(hit.at), props)
        }
        Found::Surface(hit) => {
            let mut props = identity(&hit.key, hit.revision, hit.kind);
            props.push(("page".into(), hit.page.to_string()));
            let item = if hit.kind == "surfels" { "source_id" } else { "triangle" };
            props.push((item.into(), hit.item.map_or("none".into(), |i| i.to_string())));
            props.push(("normal".into(), if hit.normal_supplied { "supplied" } else { "missing" }.into()));
            props.push(("confidence".into(), hit.confidence.map_or("none".into(), |c| c.to_string())));
            ("data", place(hit.at), props)
        }
        Found::Terrain(hit) => {
            let mut props = identity(&hit.key, hit.revision, "elevation");
            props.push(("tile".into(), format!("{},{}", hit.tile[0], hit.tile[1])));
            props.push(("cell".into(), format!("{},{}", hit.cell[0], hit.cell[1])));
            props.push(("elevation".into(), hit.elevation.to_string()));
            ("data", place(hit.at), props)
        }
        Found::Field(hit) => {
            let mut props = identity(&hit.key, hit.revision, "volume");
            props.push(("slice".into(), hit.slice.to_string()));
            props.push(("support".into(), if hit.value.is_some() { "valid" } else { "unknown" }.into()));
            props.push(("value".into(), hit.value.map_or("unknown".into(), |v| v.to_string())));
            ("data", place(hit.at), props)
        }
        Found::Nothing => ("no_data", Vec3::ZERO, Vec::new()),
        Found::Stale(reason) => ("stale", Vec3::ZERO, vec![("reason".into(), reason.into())]),
    };
    if pending.inspect && what == "data" {
        inspect(world, pending, at, &props);
    }
    props.push(("ndc".into(), format!("{} {}", pending.ndc.x, pending.ndc.y)));
    let mut event = SceneEvent::new(event_kind::DRAW, &pending.id)
        .at(at.x, at.y, at.z)
        .with_prop("ns", &pending.ns)
        .with_prop("what", what);
    for (key, value) in &props {
        event = event.with_prop(key, value);
    }
    if let Some(mut bus) = world.get_resource_mut::<GearboxBus>() {
        bus.publish_event(event);
    }
    if let Some(store) = world.get_resource::<DrawStoreRes>() {
        let _ = store.lock().apply(DrawEntity::delete(&pending.ns, &pending.id), 0.0);
    }
}

fn frame_name(frame: &gearbox_api::DrawFrame) -> String {
    use gearbox_api::DrawFrame;
    match frame {
        DrawFrame::World => "world".into(),
        DrawFrame::Odom(machine) => format!("{machine}/odom"),
        DrawFrame::Link(machine, link) if link.is_empty() => machine.clone(),
        DrawFrame::Link(machine, link) => format!("{machine}/{link}"),
    }
}

/// A Halo label at the hit, `<id>-inspect` in the interaction's namespace.
fn inspect(world: &World, pending: &Pending, at: Vec3, props: &[(String, String)]) {
    use gearbox_api::{DrawEntity, DrawStoreRes, draw_style};
    let get = |key: &str| props.iter().find(|(k, _)| k == key).map_or("", |(_, v)| v.as_str());
    let mut text = match get("kind") {
        "cloud" => format!("{} / #{} / {}", get("data_id"), get("source_id"), get("lod")),
        "octree" => format!("{} / {} {} / {}", get("data_id"), get("state"), get("leaf"), get("lod")),
        "surfels" => format!("{} / #{} / normal {}", get("data_id"), get("source_id"), get("normal")),
        "volume" => format!("{} / slice {} / {}", get("data_id"), get("slice"), get("value").to_uppercase()),
        "elevation" => format!("{} / tile {} cell {} / {} m", get("data_id"), get("tile"), get("cell"), get("elevation")),
        _ => format!("{} / block {} triangle {} / normal {}", get("data_id"), get("page"), get("triangle"), get("normal")),
    };
    for (key, value) in props.iter().filter(|(k, _)| k.starts_with("attr.")).take(3) {
        let name = &key["attr.".len()..];
        let unit = get(&format!("unit.{name}"));
        text += &format!(" / {name} {value}{}{unit}", if unit.is_empty() { "" } else { " " });
    }
    let label = DrawEntity::text(&pending.ns, &format!("{}-inspect", pending.id), &[at.to_array()], &text)
        .in_frame(&frame_name(&pending.frame))
        .with_style(draw_style::LABEL)
        .with_width(-12.0)
        .with_prop("label_category", "DATA PICK");
    if let Some(store) = world.get_resource::<DrawStoreRes>() {
        let _ = store.lock().apply(label, 0.0);
    }
}

/// Picks requested through the API: a pick interaction carrying `ndc` ("x y",
/// y up) is resolved at once against the main camera, without a pointer.
#[derive(Resource, Default)]
struct ApiPicks {
    pending: std::collections::HashMap<u64, Pending>,
}

fn api_picks(world: &mut World) {
    use gearbox_api::{DrawStoreRes, draw_kind, interact_kind};
    let Some(picks) = world.get_resource::<Picks>().cloned() else {
        return;
    };
    let mut api = world.remove_resource::<ApiPicks>().unwrap_or_default();
    for (id, found) in picks.take_owned(|id| api.pending.contains_key(&id)) {
        if let Some(pending) = api.pending.remove(&id) {
            publish(world, &pending, found);
        }
    }
    let asked: Vec<(String, String, gearbox_api::DrawFrame, gearbox_api::Props)> = world
        .get_resource::<DrawStoreRes>()
        .map(|store| {
            store
                .lock()
                .iter()
                .filter(|(_, item)| item.entity.kind == draw_kind::INTERACT && item.entity.sub == interact_kind::PICK_DATA)
                .filter(|(_, item)| item.entity.props().get("ndc").is_some())
                .map(|((ns, id), item)| (ns.clone(), id.clone(), item.entity.frame(), item.entity.props()))
                .collect()
        })
        .unwrap_or_default();
    let fresh: Vec<_> = asked
        .into_iter()
        .filter(|(ns, id, ..)| !api.pending.values().any(|p| p.ns == *ns && p.id == *id))
        .collect();
    if !fresh.is_empty() {
        let mut cameras = world.query_filtered::<(Entity, &GlobalTransform, &Projection, &Camera), With<mara::ui::modules::bevy::ChaseCamera>>();
        let camera = cameras.iter(world).next().map(|(e, g, p, c)| (e, *g, p.clone(), c.logical_viewport_size()));
        let home = world.get_resource::<crate::globe::Sites>().map(|sites| sites.home().clone());
        let site = home.as_ref().and_then(|home| world.get::<GlobalTransform>(home.entity).copied());
        for (ns, id, frame, props) in fresh {
            let ndc = props.get("ndc").and_then(|text| {
                let mut parts = text.split_whitespace().map(str::parse::<f32>);
                Some(Vec2::new(parts.next()?.ok()?, parts.next()?.ok()?))
            });
            let (Some((camera, eye, Projection::Perspective(projection), size)), Some(home), Some(site), Some(ndc)) =
                (camera.clone(), home.clone(), site, ndc)
            else {
                continue;
            };
            let id_number = picks.next_id();
            picks.submit(Pick {
                id: id_number,
                camera,
                eye,
                fov: projection.fov,
                aspect: size.map_or(projection.aspect_ratio, |s| s.x / s.y.max(1.0)),
                ndc,
                prefix: props.get("prefix").unwrap_or_default(),
                occluder: f32::INFINITY,
            });
            let inspect = props.get("inspect").is_some();
            api.pending.insert(id_number, Pending { ns, id, frame, site, home, ndc, inspect });
        }
    }
    world.insert_resource(api);
}

/// Distance along unit `d` from `o` (both in a site's frame) to its ground.
pub(crate) fn terrain_distance(o: Vec3, d: Vec3) -> Option<f32> {
    let above = |t: f32| {
        let p = o + d * t;
        p.y - crate::globe::ground_height_at_physics(p.x as f64, p.z as f64) as f32
    };
    if above(0.0) < 0.0 {
        return None;
    }
    let (mut lo, mut t) = (0.0f32, 0.3f32);
    while t < OCCLUDER_RANGE_M {
        if above(t) <= 0.0 {
            let mut hi = t;
            for _ in 0..24 {
                let mid = 0.5 * (lo + hi);
                if above(mid) > 0.0 { lo = mid } else { hi = mid }
            }
            return Some(hi);
        }
        lo = t;
        t += (0.02 * t).max(0.25);
    }
    None
}

fn occlude(
    picks: Res<Picks>,
    physics: Option<Res<crate::physics::PhysicsWorld>>,
    sites: Option<Res<crate::globe::Sites>>,
    globals: Query<&GlobalTransform>,
) {
    let mut exchange = picks.lock();
    if exchange.waiting.is_empty() {
        return;
    }
    let site = sites.and_then(|sites| globals.get(sites.home().entity).ok().copied());
    while let Some(mut pick) = exchange.waiting.pop_front() {
        let d = pick.direction();
        let mut t = f32::INFINITY;
        if let Some(site) = site {
            let inverse = site.affine().inverse();
            let o = inverse.transform_point3(pick.eye.translation());
            let ds = inverse.transform_vector3(d).normalize();
            t = terrain_distance(o, ds).unwrap_or(t);
            if let Some(physics) = &physics {
                use crate::physics::backend::DVec3;
                let at = |v: Vec3| DVec3::new(v.x.into(), v.y.into(), v.z.into());
                if let Some((_, hit)) = physics.cast_ray(at(o), at(ds), OCCLUDER_RANGE_M.into()) {
                    t = t.min(hit as f32);
                }
            }
        }
        pick.occluder = t * d.dot(pick.eye.forward().as_vec3());
        exchange.ready.push_back(pick);
    }
}

fn moved(view: &GlobalTransform, eye: &GlobalTransform) -> bool {
    view.translation().distance(eye.translation()) > STILL_M
        || view.rotation().angle_between(eye.rotation()) > STILL_RAD
}

fn resolve(
    picks: Res<Picks>,
    selections: Res<Views>,
    views: Query<&ExtractedView>,
    nodes: Query<&super::runtime::Node>,
    octrees: Query<&super::octree::Node>,
) {
    let ready: Vec<Pick> = picks.lock().ready.drain(..).collect();
    for pick in ready {
        let started = std::time::Instant::now();
        let found = views
            .iter()
            .find(|view| view.retained_view_entity.main_entity.id() == pick.camera)
            .map_or(Found::Stale("the view was not drawn"), |view| {
                if moved(&view.world_from_view, &pick.eye) {
                    return Found::Stale("the camera moved before the frame was drawn");
                }
                nearest(&pick, view, &selections, &nodes, &octrees)
            });
        let (eye, direction) = (pick.eye.translation(), pick.direction());
        info!(
            "data pick {} at ndc {:?} in {:.2} ms: eye {eye:?} looking {direction:?}, occluder {}: {found:?}",
            pick.id,
            pick.ndc,
            started.elapsed().as_secs_f64() * 1e3,
            pick.occluder
        );
        picks.lock().found.push_back((pick.id, found));
    }
}

fn nearest(
    pick: &Pick,
    view: &ExtractedView,
    selections: &Views,
    nodes: &Query<&super::runtime::Node>,
    octrees: &Query<&super::octree::Node>,
) -> Found {
    let mut best: Option<(f32, Found)> = None;
    let mut offer = |depth: f32, found: Found| {
        if depth <= pick.occluder && best.as_ref().is_none_or(|(near, _)| depth < *near) {
            best = Some((depth, found));
        }
    };
    let clip_from_world = view.clip_from_view * view.world_from_view.to_matrix().inverse();
    let size = view.viewport.zw().as_vec2();
    let cursor = pick.ndc * size * 0.5;
    let projection_y = view.clip_from_view.y_axis.y.abs();
    for drawn in selections.drawn(view.retained_view_entity) {
        let key = drawn.dataset.key();
        if !gearbox_api::wire::draw::ns_within(&key.namespace, &pick.prefix) {
            continue;
        }
        let Ok(node) = nodes.get(drawn.entity) else {
            continue;
        };
        let (metres, pixels) = match drawn.dataset.generation.descriptor().presentation.size {
            PointSize::Metres { radius } => (radius, 0.0),
            PointSize::Pixels { radius } => (0.0, radius),
        };
        let geometry = &drawn.geometry;
        let mvp = clip_from_world * drawn.world;
        let widest = metres.max(drawn.appearance.max_radius_m);
        if let Some(bounds) = geometry.bounds_m
            && !covers(mvp, bounds, geometry.rebase_m, cursor, size, |w| widest * 0.5 * size.y * projection_y / w + pixels + 1.0)
        {
            continue;
        }
        let mut closest: Option<(f32, u32, Vec3)> = None;
        for instance in drawn.range.clone() {
            let Some(record) = drawn.values.map_or(Some(instance), |v| v.get(instance as usize).copied()) else {
                continue;
            };
            let record = record as usize;
            let (ordinal, xyz, sample) = match drawn.tier {
                Tier::Full => (
                    record as u32,
                    geometry.positions.values.get(record).map(|p| p.xyz),
                    drawn.appearance.samples.values.get(record),
                ),
                Tier::Coarse => (
                    geometry.coarse.ordinals.values.get(record).copied().unwrap_or(u32::MAX),
                    geometry.coarse.positions.values.get(record).map(|p| p.xyz),
                    drawn.appearance.coarse.records.values.get(record),
                ),
            };
            let (Some(xyz), Some(sample)) = (xyz, sample) else {
                continue;
            };
            if !node.page.clip.accepts(geometry, ordinal) {
                continue;
            }
            let clip = mvp * Vec3::from_array(xyz).extend(1.0);
            if clip.w <= 1e-6 {
                continue;
            }
            let metric = if sample.flags & super::appearance::RADIUS_VALID != 0 { sample.radius_m } else { metres };
            let radius = if metric > 0.0 { metric * 0.5 * size.y * projection_y / clip.w } else { pixels };
            let offset = clip.truncate().truncate() / clip.w * size * 0.5 - cursor;
            if offset.length() <= radius.max(0.5) + 0.5 && closest.is_none_or(|(w, ..)| clip.w < w) {
                closest = Some((clip.w, ordinal, drawn.world.transform_point3(Vec3::from_array(xyz))));
            }
        }
        if let Some((depth, ordinal, at)) = closest {
            let index = ordinal as usize;
            offer(
                depth,
                Found::Cloud(CloudHit {
                    key: key.clone(),
                    revision: drawn.dataset.revision,
                    scan: drawn.key.scan,
                    page: drawn.key.page,
                    source_id: geometry.source_id(index),
                    coarse: drawn.tier == Tier::Coarse,
                    at,
                    source_m: geometry.source_position(index).ok(),
                    attributes: attributes(&drawn.dataset.generation.descriptor().shape, geometry, index),
                }),
            );
        }
    }
    let origin = pick.eye.translation();
    let direction = pick.direction();
    let forward = pick.eye.forward().as_vec3();
    for node in octrees.iter().filter(|node| node.visible) {
        if !gearbox_api::wire::draw::ns_within(&node.dataset.key.namespace, &pick.prefix) {
            continue;
        }
        let world = node.uniform.world_from_local;
        let inverse = world.inverse();
        let (o, d) = (inverse.transform_point3(origin), inverse.transform_vector3(direction));
        let records = &node.page.cells.records.values;
        let closest = match node.page.shape {
            super::octree::Shape::Boxes => records
                .iter()
                .enumerate()
                .filter(|(_, cell)| cell.code & 0xf != super::octree::Shown::Free as u32)
                .filter_map(|(i, cell)| {
                    let center = Vec3::from_array(cell.center);
                    enter(o, d, center - Vec3::splat(cell.half), center + Vec3::splat(cell.half)).map(|t| (t, i))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0)),
            super::octree::Shape::Discs => records
                .iter()
                .enumerate()
                .filter_map(|(i, cell)| disc(o, d, cell).map(|t| (t, i)))
                .min_by(|a, b| a.0.total_cmp(&b.0)),
            super::octree::Shape::Triangles => records
                .chunks_exact(3)
                .enumerate()
                .filter(|(_, tri)| tri[0].code & super::surface::ARROW == 0 || tri[0].code & super::volume::SLICE != 0)
                .filter_map(|(i, tri)| triangle(o, d, tri).map(|t| (t, i)))
                .min_by(|a, b| a.0.total_cmp(&b.0)),
        };
        let Some((t, index)) = closest else {
            continue;
        };
        let at = world.transform_point3(o + d * t);
        let found = match node.page.shape {
            super::octree::Shape::Boxes => octree_hit(node, index, at).map(Found::Octree),
            super::octree::Shape::Triangles if records[index * 3].code & super::elevation::TERRAIN != 0 => {
                Some(Found::Terrain(terrain_hit(node, index, at)))
            }
            super::octree::Shape::Triangles if records[index * 3].code & super::volume::SLICE != 0 => {
                Some(Found::Field(field_hit(node, index, at)))
            }
            shape => Some(Found::Surface(surface_hit(node, shape, index, at))),
        };
        if let Some(found) = found {
            offer(t * direction.dot(forward), found);
        }
    }
    best.map_or(Found::Nothing, |(_, found)| found)
}

/// Ray parameter where `o + t d` meets a disc record, facing the ray when its normal is missing.
pub(super) fn disc(o: Vec3, d: Vec3, cell: &super::octree::Cell) -> Option<f32> {
    let center = Vec3::from_array(cell.center);
    let normal = if cell.code & super::surface::NORMAL_MISSING != 0 {
        -d.normalize()
    } else {
        super::surface::oct_decode(cell.spare)
    };
    let along = d.dot(normal);
    if along.abs() < 1e-9 {
        return None;
    }
    let t = (center - o).dot(normal) / along;
    (t >= 0.0 && (o + d * t).distance(center) <= cell.half).then_some(t)
}

/// Möller–Trumbore ray parameter of a three-record triangle, either side.
pub(super) fn triangle(o: Vec3, d: Vec3, tri: &[super::octree::Cell]) -> Option<f32> {
    let [a, b, c] = [0, 1, 2].map(|k| Vec3::from_array(tri[k].center));
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = o - a;
    let u = s.dot(p) / det;
    let q = s.cross(e1);
    let v = d.dot(q) / det;
    let t = e2.dot(q) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t >= 0.0).then_some(t)
}

fn terrain_hit(node: &super::octree::Node, index: usize, at: Vec3) -> TerrainHit {
    let cell = &node.page.cells.records.values[index * 3];
    let (tile, _) = gearbox_api::wire::draw_data::elevation_tile_of(node.page.address.page_id);
    TerrainHit {
        key: node.dataset.key.clone(),
        revision: node.dataset.revision,
        tile,
        cell: [cell.spare & 0xffff, cell.spare >> 16],
        elevation: cell.probability,
        at,
    }
}

fn field_hit(node: &super::octree::Node, index: usize, at: Vec3) -> FieldHit {
    let cell = &node.page.cells.records.values[index * 3];
    FieldHit {
        key: node.dataset.key.clone(),
        revision: node.dataset.revision,
        slice: super::volume::SLICE_SCAN - node.page.address.scan_key,
        value: (cell.code & super::volume::UNKNOWN == 0).then_some(cell.probability),
        at,
    }
}

fn surface_hit(node: &super::octree::Node, shape: super::octree::Shape, index: usize, at: Vec3) -> SurfaceHit {
    let records = &node.page.cells.records.values;
    let cell = match shape {
        super::octree::Shape::Triangles => &records[index * 3],
        _ => &records[index],
    };
    let (kind, item) = match shape {
        super::octree::Shape::Discs => ("surfels", node.page.ids.as_ref().and_then(|ids| ids.values.get(index).copied())),
        _ => ("mesh", Some(index as u64)),
    };
    SurfaceHit {
        key: node.dataset.key.clone(),
        revision: node.dataset.revision,
        kind,
        page: node.page.address.page_id,
        item,
        normal_supplied: cell.code & super::surface::NORMAL_MISSING == 0,
        confidence: (cell.probability >= 0.0).then_some(cell.probability),
        at,
    }
}

/// Whether a page's projected bounds, widened by `pad` pixels at the nearest
/// corner's depth, can hold the cursor; a page straddling the eye always can.
pub(super) fn covers(
    mvp: Mat4,
    bounds: [[f64; 3]; 2],
    rebase: [f64; 3],
    cursor: Vec2,
    size: Vec2,
    pad: impl Fn(f32) -> f32,
) -> bool {
    let local = |corner: usize, axis: usize| (bounds[(corner >> axis) & 1][axis] - rebase[axis]) as f32;
    let (mut low, mut high, mut nearest) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN), f32::MAX);
    for corner in 0..8 {
        let clip = mvp * Vec3::new(local(corner, 0), local(corner, 1), local(corner, 2)).extend(1.0);
        if clip.w <= 1e-6 {
            return true;
        }
        let at = clip.truncate().truncate() / clip.w * size * 0.5;
        (low, high, nearest) = (low.min(at), high.max(at), nearest.min(clip.w));
    }
    let pad = pad(nearest);
    cursor.cmpge(low - pad).all() && cursor.cmple(high + pad).all()
}

/// Ray parameter where `o + t d` enters the box, if it does at `t >= 0`.
pub(super) fn enter(o: Vec3, d: Vec3, low: Vec3, high: Vec3) -> Option<f32> {
    let (mut near, mut far) = (0.0f32, f32::INFINITY);
    for axis in 0..3 {
        if d[axis].abs() < 1e-12 {
            if o[axis] < low[axis] || o[axis] > high[axis] {
                return None;
            }
            continue;
        }
        let (a, b) = ((low[axis] - o[axis]) / d[axis], (high[axis] - o[axis]) / d[axis]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
        if near > far {
            return None;
        }
    }
    Some(near)
}

fn octree_hit(node: &super::octree::Node, index: usize, at: Vec3) -> Option<OctreeHit> {
    let cell = node.page.cells.records.values.get(index)?;
    let depth = (cell.code >> 8 & 0xff) as u8;
    let leaf = leaf_key(node.config.as_ref()?, node.page.rebase_m, cell.center, depth)?;
    let state = match cell.code & 0xf {
        0 => "unknown",
        1 => "free",
        2 => "uncertain",
        3 => "occupied",
        _ => "mixed",
    };
    Some(OctreeHit {
        key: node.dataset.key.clone(),
        revision: node.dataset.revision,
        leaf,
        state,
        probability: (cell.probability >= 0.0).then_some(cell.probability),
        aggregate: cell.code & super::octree::AGGREGATE != 0,
        implicit: cell.code & super::octree::IMPLICIT != 0,
        at,
    })
}

/// The key of the depth-`depth` cell centred at rebased `center`.
pub(super) fn leaf_key(config: &OctreeConfig, rebase_m: [f64; 3], center: [f32; 3], depth: u8) -> Option<VoxelKey> {
    let cells = 1i128 << (config.max_depth.checked_sub(depth)?);
    let size = config.leaf_resolution_m * cells as f64;
    let finest = std::array::from_fn(|axis| {
        let low = rebase_m[axis] + f64::from(center[axis]) - 0.5 * size;
        (low / config.leaf_resolution_m).round() as i128
    });
    VoxelKey::from_grid_cell(finest, depth, config).ok()
}

/// Every attribute but position and source ID at one render sample.
fn attributes(shape: &DataShape, geometry: &super::cloud::Geometry, index: usize) -> Vec<(String, String, String)> {
    let (DataShape::Cloud { schema }, Some(source), Some(row)) =
        (shape, geometry.source(), geometry.positions.values.get(index).map(|p| p.source_index as usize))
    else {
        return Vec::new();
    };
    schema
        .attributes
        .iter()
        .enumerate()
        .filter(|(_, a)| !matches!(a.semantic, AttributeSemantic::Position | AttributeSemantic::SourceId))
        .filter_map(|(column, a)| {
            let view = source.column(column)?;
            Some((a.name.clone(), value(view, a.scalar, a.components, a.normalization, row), a.units.clone()))
        })
        .collect()
}

fn value(column: ColumnView<'_>, scalar: ScalarType, components: u8, normalization: Normalization, row: usize) -> String {
    if !super::cloud::valid(column, row) {
        return "invalid".into();
    }
    let size = scalar.size();
    let stride = size * components as usize;
    let Some(bytes) = column.values.get(row * stride..row * stride + stride) else {
        return "invalid".into();
    };
    let parts: Vec<String> = bytes
        .chunks_exact(size)
        .map(|raw| {
            let mut wide = [0u8; 8];
            wide[..size].copy_from_slice(raw);
            let text = match scalar {
                ScalarType::F32 => f32::from_le_bytes(raw.try_into().unwrap()).to_string(),
                ScalarType::F64 => f64::from_le_bytes(wide).to_string(),
                ScalarType::I16 => i16::from_le_bytes(raw.try_into().unwrap()).to_string(),
                ScalarType::I32 => i32::from_le_bytes(raw.try_into().unwrap()).to_string(),
                _ => u64::from_le_bytes(wide).to_string(),
            };
            text
        })
        .collect();
    let joined = parts.join(" ");
    match normalization {
        Normalization::None => joined,
        _ => format!("{joined} ({normalization:?})").to_lowercase(),
    }
}
