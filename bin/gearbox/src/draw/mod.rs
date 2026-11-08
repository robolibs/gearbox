//! gearbox draw in the viewer: retained drawings. Every entity the host keeps
//! ([`gearbox_api::DrawStoreRes`]) gets a root placed in its frame, with a
//! stroke mesh and a fill mesh under it, rebuilt when the entity changes.
//! Labels are projected to the viewport for the host to paint.
//!
//! Frames: `world` hangs under the home site, `<machine>/odom` under the
//! home site at the machine's own datum, `<machine>/<link>` under the link
//! itself. Entity coordinates are Z-up; world and odom turn them into the
//! site's Y-up axes (x east, y north, z up becomes z, x, y), link frames use
//! the link's own axes. Draped entities in world and odom frames measure z
//! from the terrain.

mod assets;
mod drape;
mod ghost;
mod heat;
mod material;
mod mesh;

use std::collections::{HashMap, HashSet};

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::asset::UntypedAssetId;
use bevy::core_pipeline::tonemapping::TonemappingLuts;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use gearbox_api::draw::{DrawItem, DrawKey};
use gearbox_api::{DrawClock, DrawEntity, DrawFrame, DrawStoreRes, DrawSystems, draw_flag, draw_kind};
use mara::ui::modules::bevy::ChaseCamera;

use material::{StrokeMaterial, SurfaceMaterial};
use mesh::LabelSpec;

/// Draws show in the view and in captures, never to sensor cameras.
const DRAW_LAYER: usize = crate::viewer::recorder::RECORDED_OVERLAY_LAYER;

pub struct DrawPlugin;

impl Plugin for DrawPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DrawLabels>()
            .init_resource::<DrawPanels>()
            .add_systems(Update, tick_draw_clock.in_set(DrawSystems::Clock));
        if app.get_sub_app(bevy::render::RenderApp).is_none() {
            return;
        }
        bevy::shader::load_shader_library!(app, "draw_common.wgsl");
        bevy::asset::embedded_asset!(app, "draw_stroke.wgsl");
        bevy::asset::embedded_asset!(app, "draw_surface.wgsl");
        bevy::asset::embedded_asset!(app, "draw_heat.wgsl");
        bevy::asset::embedded_asset!(app, "draw_ghost.wgsl");
        app.add_plugins((
            MaterialPlugin::<StrokeMaterial>::default(),
            MaterialPlugin::<SurfaceMaterial>::default(),
            MaterialPlugin::<heat::HeatMaterial>::default(),
            MaterialPlugin::<ghost::GhostMaterial>::default(),
        ))
            .init_resource::<DrawVisuals>()
            .add_systems(Update, (sync_draws, place_draws, ghost::build_ghosts).chain().after(DrawSystems::Serve))
            .add_systems(PostUpdate, project_draw_labels.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// Lifetimes count in simulated seconds.
fn tick_draw_clock(physics: Option<Res<crate::physics::PhysicsWorld>>, mut clock: ResMut<DrawClock>) {
    clock.0 = physics.map(|p| p.simulated_seconds);
}

#[derive(Component)]
struct DrawRoot;

struct Part {
    entity: Entity,
    mesh: Handle<Mesh>,
    material: UntypedAssetId,
}

#[derive(Clone, Copy, PartialEq)]
struct Placement {
    parent: Entity,
    transform: Transform,
}

struct Visual {
    root: Entity,
    revision: u64,
    /// Hash of everything but the pose, to move without rebuilding.
    content: u64,
    frame: DrawFrame,
    pose: Transform,
    snapshot: bool,
    frozen: Option<Placement>,
    /// Fill and strokes.
    parts: [Option<Part>; 2],
    /// A grid's heat map, its material and what its mesh was built from.
    heat: Option<Part>,
    heat_material: Option<Handle<heat::HeatMaterial>>,
    heat_geometry: u64,
    labels: Vec<LabelSpec>,
    /// A panel's chart and its anchor in the root's space.
    panel: Option<(PanelChart, Vec3)>,
    /// The machine whose TF tree this entity switches on.
    tf: Option<String>,
    /// A ghost's request, its mesh copies and whether they are built.
    ghost: Option<ghost::GhostSpec>,
    ghost_copies: Vec<Entity>,
    ghost_built: bool,
}

#[derive(Default)]
struct MaterialCache {
    strokes: HashMap<(bool, bool), Handle<StrokeMaterial>>,
    surfaces: HashMap<(bool, bool, bool, bool), Handle<SurfaceMaterial>>,
}

#[derive(Resource, Default)]
struct DrawVisuals {
    map: HashMap<DrawKey, Visual>,
    seen: u64,
    materials: MaterialCache,
    /// Machines whose TF tree draws switched on.
    tf_on: Vec<String>,
}

/// Turns entity Z-up axes (x east, y north, z up) into site axes (x north,
/// y up, z east).
pub(crate) fn enu_to_site() -> Transform {
    Transform::from_rotation(Quat::from_mat3(&Mat3::from_cols(Vec3::Z, Vec3::X, Vec3::Y)))
}

/// A hash of an entity's header and sections, its pose and lifetime left out.
fn content_hash(e: &DrawEntity) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let floats = [e.sx, e.sy, e.sz, e.width, e.glow, e.pulse_hz, e.flow_speed, e.dash, e.beam_height, e.fade, e.urgency, e.vmin, e.vmax, e.quant];
    floats.map(f32::to_bits).hash(&mut h);
    [e.kind, e.sub, e.flags, e.style, e.color, e.color2, e.colormap].hash(&mut h);
    let bits = |v: &[f32]| v.iter().map(|f| f.to_bits()).collect::<Vec<u32>>();
    bits(&e.points).hash(&mut h);
    bits(&e.radii).hash(&mut h);
    bits(&e.instances).hash(&mut h);
    bits(&e.values).hash(&mut h);
    (&e.colors, &e.offsets, &e.rings, &e.qpoints, &e.ranges, &e.cells, &e.labels, &e.props).hash(&mut h);
    h.finish()
}

pub(crate) fn pose_of(e: &DrawEntity) -> Transform {
    let q = Quat::from_xyzw(e.qx as f32, e.qy as f32, e.qz as f32, e.qw as f32);
    let rotation = if q.length_squared() > 1e-8 { q.normalize() } else { Quat::IDENTITY };
    Transform::from_xyz(e.px as f32, e.py as f32, e.pz as f32).with_rotation(rotation)
}

fn sync_draws(
    mut commands: Commands,
    store: Res<DrawStoreRes>,
    mut visuals: ResMut<DrawVisuals>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut strokes: ResMut<Assets<StrokeMaterial>>,
    mut surfaces: ResMut<Assets<SurfaceMaterial>>,
    mut heats: ResMut<Assets<heat::HeatMaterial>>,
    mut images: ResMut<Assets<Image>>,
    luts: Option<Res<TonemappingLuts>>,
    sites: Option<Res<crate::globe::Sites>>,
    roots: Query<(), With<DrawRoot>>,
) {
    let (Some(luts), Some(sites)) = (luts, sites) else {
        return;
    };
    let visuals = visuals.as_mut();
    visuals.map.retain(|_, v| roots.contains(v.root));
    let (live, changed) = {
        let store = store.lock();
        if store.revision() == visuals.seen && visuals.map.len() == store.len() {
            return;
        }
        visuals.seen = store.revision();
        let live: HashSet<DrawKey> = store.iter().map(|(k, _)| k.clone()).collect();
        let changed: Vec<(DrawKey, DrawItem)> = store
            .iter()
            .filter(|(k, item)| visuals.map.get(*k).is_none_or(|v| v.revision != item.revision))
            .map(|(k, item)| (k.clone(), item.clone()))
            .collect();
        (live, changed)
    };
    visuals.map.retain(|k, v| {
        let keep = live.contains(k);
        if !keep {
            commands.entity(v.root).try_despawn();
        }
        keep
    });
    let home_site = sites.home();
    let home = home_site.entity;
    for (key, item) in &changed {
        let e = &item.entity;
        let frame = e.frame();
        let draping = e.flags & draw_flag::DRAPE != 0 && !matches!(frame, DrawFrame::Link(..));
        let content = content_hash(e);
        if let Some(v) = visuals.map.get_mut(key)
            && v.content == content
            && !draping
            && v.frame == frame
        {
            v.revision = item.revision;
            v.pose = pose_of(e);
            v.frozen = None;
            continue;
        }
        let expanded = e.expanded();
        let place = if draping {
            let Some(place) = site_transform(&frame, pose_of(e), home_site) else {
                visuals.seen = u64::MAX;
                continue;
            };
            Some(place)
        } else {
            None
        };
        let draped = place.map(|p| drape::drape(&expanded, &p));
        let rise = place.map(drape::rise_fn);
        let ground: mesh::Ground = rise.as_ref().map(|r| r as &dyn Fn(f32, f32) -> f32);
        let built = mesh::build_on(draped.as_ref().unwrap_or(&expanded), &key.0, &key.1, ground);
        let on_top = e.flags & draw_flag::ON_TOP != 0;
        let snapshot = e.flags & draw_flag::SNAPSHOT != 0;
        let visual = visuals.map.entry(key.clone()).or_insert_with(|| Visual {
            root: commands
                .spawn((Name::new(format!("Draw[{}/{}]", key.0, key.1)), DrawRoot, Transform::IDENTITY, Visibility::Hidden, ChildOf(home)))
                .id(),
            revision: 0,
            content: 0,
            frame: frame.clone(),
            pose: Transform::IDENTITY,
            snapshot,
            frozen: None,
            parts: Default::default(),
            heat: None,
            heat_material: None,
            heat_geometry: 0,
            labels: Vec::new(),
            panel: None,
            tf: None,
            ghost: None,
            ghost_copies: Vec::new(),
            ghost_built: false,
        });
        if visual.frame != frame || visual.snapshot != snapshot || (snapshot && visual.revision != item.revision) {
            visual.frozen = None;
        }
        visual.revision = item.revision;
        visual.content = content;
        visual.frame = frame;
        visual.snapshot = snapshot;
        visual.pose = pose_of(e);
        visual.labels = built.labels;
        visual.panel = (e.kind == draw_kind::PANEL).then(|| (PanelChart::of(e), Vec3::Z * ground.map_or(0.0, |g| g(0.0, 0.0))));
        visual.tf = e.props().get("tf").filter(|_| e.kind == draw_kind::AXES);
        for copy in visual.ghost_copies.drain(..) {
            commands.entity(copy).try_despawn();
        }
        visual.ghost_built = false;
        visual.ghost = (e.kind == draw_kind::GHOST).then(|| {
            let fx = mesh::Fx::of(e, gearbox_api::draw_style::GHOST);
            ghost::GhostSpec {
                machine: e.props().get("machine").unwrap_or_default(),
                instances: draped.as_ref().unwrap_or(&expanded).instance_list(),
                color: if e.color != 0 { gearbox_api::wire::draw::unpack_rgba(e.color) } else { [0x56, 0xB4, 0xE9, 255] },
                glow: fx.glow,
                pulse: fx.pulse_hz,
                fade: fx.fade,
                phase: 0.0,
            }
        });

        let on_top = on_top || built.on_top;
        let cache = &mut visuals.materials;
        let splat = built.splat;
        let stroke = (!built.strokes.is_empty()).then(|| {
            let material = cache
                .strokes
                .entry((on_top, splat))
                .or_insert_with(|| strokes.add(StrokeMaterial { unused: Vec4::ZERO, lut: luts.agx.clone(), on_top, splat }))
                .clone();
            (built.strokes.into_mesh(), material)
        });
        let fill = (!built.fill.is_empty()).then(|| {
            let translucent = built.fill.translucent();
            let double_sided = built.double_sided || translucent;
            let material = cache
                .surfaces
                .entry((on_top, translucent, double_sided, built.shaded))
                .or_insert_with(|| {
                    surfaces.add(SurfaceMaterial {
                        flags: Vec4::new(built.shaded as u32 as f32, 0.0, 0.0, 0.0),
                        lut: luts.agx.clone(),
                        on_top,
                        translucent,
                        double_sided,
                    })
                })
                .clone();
            (built.fill.into_mesh(), material)
        });
        let root = visual.root;
        let parts = &mut visual.parts;
        set_part(&mut commands, &mut parts[0], root, fill, &mut meshes);
        set_part(&mut commands, &mut parts[1], root, stroke, &mut meshes);
        let grid = (e.kind == draw_kind::GRID).then(|| heat::images(&expanded)).flatten();
        match grid {
            Some((cells, values, size)) => {
                let (cells, values) = (images.add(cells), images.add(values));
                let params = heat::params(&expanded, size);
                let material = match visual.heat_material.as_ref().filter(|h| heats.get(*h).is_some()) {
                    Some(h) => {
                        let mut m = heats.get_mut(h).unwrap();
                        (m.cells, m.values, m.params, m.on_top) = (cells, values, params, on_top);
                        h.clone()
                    }
                    None => {
                        let h = heats.add(heat::HeatMaterial { params, lut: luts.agx.clone(), cells, values, on_top });
                        visual.heat_material = Some(h.clone());
                        h
                    }
                };
                let geometry = heat::geometry_key(&expanded, draping);
                if visual.heat_geometry != geometry || visual.heat.is_none() {
                    let mesh = heat::mesh(&expanded, ground).map(|m| (m, material));
                    set_part(&mut commands, &mut visual.heat, root, mesh, &mut meshes);
                    visual.heat_geometry = geometry;
                }
            }
            None => {
                set_part::<heat::HeatMaterial>(&mut commands, &mut visual.heat, root, None, &mut meshes);
                visual.heat_material = None;
                visual.heat_geometry = 0;
            }
        }
    }
}

/// Puts `content` in `slot`: the mesh replaced in place, spawned, or gone.
fn set_part<M: Material>(
    commands: &mut Commands,
    slot: &mut Option<Part>,
    root: Entity,
    content: Option<(Mesh, Handle<M>)>,
    meshes: &mut Assets<Mesh>,
) {
    match (content, slot.as_mut()) {
        (None, Some(part)) => {
            commands.entity(part.entity).try_despawn();
            *slot = None;
        }
        (None, None) => {}
        (Some((mesh, material)), Some(part)) => {
            let _ = meshes.insert(&part.mesh, mesh);
            if part.material != material.id().untyped() {
                part.material = material.id().untyped();
                commands.entity(part.entity).insert(MeshMaterial3d(material));
            }
        }
        (Some((mesh, material)), None) => {
            let handle = meshes.add(mesh);
            let material_id = material.id().untyped();
            let entity = commands
                .spawn((
                    Mesh3d(handle.clone()),
                    MeshMaterial3d(material),
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    NoFrustumCulling,
                    NotShadowCaster,
                    NotShadowReceiver,
                    RenderLayers::layer(DRAW_LAYER),
                    ChildOf(root),
                ))
                .id();
            *slot = Some(Part { entity, mesh: handle, material: material_id });
        }
    }
}

/// Entity coordinates into the home site's for the frames that hang under
/// it, `None` for a link frame or a machine not yet put down.
pub(crate) fn site_transform(frame: &DrawFrame, pose: Transform, home: &crate::globe::SiteEntry) -> Option<Transform> {
    match frame {
        DrawFrame::World => Some(enu_to_site() * pose),
        DrawFrame::Odom(machine) => {
            let own = crate::globe::machine_datum(machine)?;
            let turn = home.frame.rotation.inverse();
            let datum = Transform::from_translation((turn * (own.origin - home.frame.origin)).as_vec3())
                .with_rotation((turn * own.rotation).as_quat());
            Some(datum * enu_to_site() * pose)
        }
        DrawFrame::Link(..) => None,
    }
}

/// Where a draw root hangs and how, or `None` while its frame is missing.
fn resolve(
    v: &mut Visual,
    home: &crate::globe::SiteEntry,
    inventory: &crate::controller::ControllerInventory,
    index: &crate::prim_index::PrimIndex,
    globals: &Query<&GlobalTransform>,
) -> Option<Placement> {
    if let Some(frozen) = v.frozen {
        return Some(frozen);
    }
    let placement = match &v.frame {
        DrawFrame::World | DrawFrame::Odom(_) => Placement { parent: home.entity, transform: site_transform(&v.frame, v.pose, home)? },
        DrawFrame::Link(machine, link) => {
            let spec = inventory.machines.iter().find(|m| m.id == *machine)?;
            let scene_root = spec.scene_root?;
            let link = if link.is_empty() { spec.links.base().or(spec.links.links.first()) } else { spec.links.get(link) }?;
            let entity = index.find(scene_root, &link.prim_path)?;
            let scale = globals.get(entity).ok()?.to_scale_rotation_translation().0.x;
            let s = if scale.abs() > 1e-6 { 1.0 / scale } else { 1.0 };
            let transform = Transform { translation: v.pose.translation * s, rotation: v.pose.rotation, scale: Vec3::splat(s) };
            Placement { parent: entity, transform }
        }
    };
    if v.snapshot {
        let frozen = if placement.parent == home.entity {
            placement
        } else {
            let site = globals.get(home.entity).ok()?.affine();
            let parent = globals.get(placement.parent).ok()?.affine();
            let local = site.inverse() * parent * placement.transform.compute_affine();
            let mut transform = Transform::from_matrix(Mat4::from(local));
            transform.scale = Vec3::ONE;
            Placement { parent: home.entity, transform }
        };
        v.frozen = Some(frozen);
        return Some(frozen);
    }
    Some(placement)
}

fn place_draws(
    mut commands: Commands,
    mut visuals: ResMut<DrawVisuals>,
    sites: Option<Res<crate::globe::Sites>>,
    inventory: Res<crate::controller::ControllerInventory>,
    index: Res<crate::prim_index::PrimIndex>,
    mut toggles: ResMut<crate::viewer::overlays::DisplayToggles>,
    globals: Query<&GlobalTransform>,
    mut roots: Query<(&mut Transform, &mut Visibility, &ChildOf), With<DrawRoot>>,
) {
    let Some(sites) = sites else {
        return;
    };
    let mut wanted: Vec<String> = visuals.map.values().filter_map(|v| v.tf.clone()).collect();
    wanted.sort();
    wanted.dedup();
    if wanted != visuals.tf_on {
        let on = !wanted.is_empty();
        if on || !visuals.tf_on.is_empty() {
            toggles.show_tf_frames = on;
            toggles.show_tf_names = on;
            toggles.show_tf_links = on;
            toggles.tf_machines = wanted.clone();
        }
        visuals.tf_on = wanted;
    }
    let home = sites.home();
    for visual in visuals.map.values_mut() {
        let placement = resolve(visual, home, &inventory, &index, &globals);
        let Ok((mut transform, mut visibility, parent)) = roots.get_mut(visual.root) else {
            continue;
        };
        let shown = if placement.is_some() && toggles.show_draws { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != shown {
            *visibility = shown;
        }
        let Some(placement) = placement else {
            continue;
        };
        if parent.parent() != placement.parent {
            commands.entity(visual.root).insert(ChildOf(placement.parent));
        }
        if *transform != placement.transform {
            *transform = placement.transform;
        }
    }
}

/// Draw labels with their place in the viewport as a fraction of the render
/// target, for the host to paint over the viewport.
#[derive(Resource, Default, Debug, Clone)]
pub struct DrawLabels(pub Vec<DrawLabel>);

/// In-world charts with their place in the viewport (a fraction of the
/// render target; legends have none), for the host to paint.
#[derive(Resource, Default, Debug, Clone)]
pub struct DrawPanels(pub Vec<DrawPanel>);

#[derive(Debug, Clone)]
pub struct DrawPanel {
    pub key: (String, String),
    pub chart: PanelChart,
    /// Where its anchor is in the viewport, `None` for screen-space ones.
    pub at: Option<[f32; 2]>,
    /// 1 near, fading to 0 far away.
    pub opacity: f32,
}

/// What a panel shows, read from its entity.
#[derive(Debug, Clone, Default)]
pub struct PanelChart {
    pub kind: u32,
    pub title: String,
    pub unit: String,
    pub text: String,
    pub values: Vec<f32>,
    pub labels: Vec<String>,
    pub colors: Vec<[u8; 4]>,
    pub color: [u8; 4],
    pub min: f32,
    pub max: f32,
    /// Whether `max` was given rather than defaulted.
    pub max_given: bool,
    pub rows: Vec<(String, String)>,
    /// Colormap, vmin and vmax of a legend.
    pub map: (u32, f32, f32),
    /// Shown over a machine.
    pub badge: bool,
}

impl PanelChart {
    fn of(e: &DrawEntity) -> Self {
        let p = e.props();
        let get = |k: &str| p.get(k).unwrap_or_default();
        let color = if e.color != 0 { gearbox_api::wire::draw::unpack_rgba(e.color) } else { [0x56, 0xB4, 0xE9, 255] };
        Self {
            kind: e.sub,
            title: get("title"),
            unit: get("unit"),
            text: get("text"),
            values: e.values.clone(),
            labels: e.label_list(),
            colors: e.colors.iter().map(|c| gearbox_api::wire::draw::unpack_rgba(*c)).collect(),
            color,
            min: e.prop_f32("min").unwrap_or(0.0),
            max: e.prop_f32("max").unwrap_or(100.0),
            max_given: e.prop_f32("max").is_some(),
            rows: get("rows").lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect(),
            map: (e.colormap, e.vmin, e.vmax),
            badge: p.get("badge").is_some(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DrawLabel {
    pub text: String,
    pub at: [f32; 2],
    pub color: [u8; 4],
    pub background: [u8; 4],
    /// Colour of the bar on the plate's left edge.
    pub accent: [u8; 4],
    pub size_px: f32,
    /// Centred on the point rather than beside it on a leader line.
    pub centred: bool,
    /// 1 near, fading to 0 far away.
    pub opacity: f32,
}

const MIN_LABEL_PX: f32 = 5.0;
const MAX_LABEL_PX: f32 = 160.0;
/// Labels fade out between these distances from the eye, metres.
const LABEL_FADE_M: (f32, f32) = (60.0, 90.0);
/// Panels fade out between these distances from the eye, metres.
const PANEL_FADE_M: (f32, f32) = (80.0, 120.0);

fn project_draw_labels(
    visuals: Option<Res<DrawVisuals>>,
    roots: Query<(&GlobalTransform, &InheritedVisibility), With<DrawRoot>>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<ChaseCamera>>,
    mut labels: ResMut<DrawLabels>,
    mut panels: ResMut<DrawPanels>,
) {
    labels.0.clear();
    panels.0.clear();
    let Some(visuals) = visuals else {
        return;
    };
    if let Ok((camera, eye, _)) = cameras.single()
        && let Some(size) = camera.logical_viewport_size()
    {
        let view_from_world = eye.affine().inverse();
        for (key, visual) in visuals.map.iter() {
            let Some((chart, anchor)) = &visual.panel else {
                continue;
            };
            let Ok((root, shown)) = roots.get(visual.root) else {
                continue;
            };
            if !shown.get() {
                continue;
            }
            if chart.kind == gearbox_api::panel_kind::LEGEND {
                panels.0.push(DrawPanel { key: key.clone(), chart: chart.clone(), at: None, opacity: 1.0 });
                continue;
            }
            let world = root.transform_point(*anchor);
            let Ok(screen) = camera.world_to_viewport(eye, world) else {
                continue;
            };
            let depth = -view_from_world.transform_point3(world).z;
            let opacity = 1.0 - ((depth - PANEL_FADE_M.0) / (PANEL_FADE_M.1 - PANEL_FADE_M.0)).clamp(0.0, 1.0);
            if opacity > 0.0 && depth > 0.0 {
                panels.0.push(DrawPanel { key: key.clone(), chart: chart.clone(), at: Some([screen.x / size.x, screen.y / size.y]), opacity });
            }
        }
        panels.0.sort_by(|a, b| a.key.cmp(&b.key));
    }
    let Ok((camera, eye, projection)) = cameras.single() else {
        return;
    };
    let Some(size) = camera.logical_viewport_size() else {
        return;
    };
    let fov = match projection {
        Projection::Perspective(p) => Some(p.fov),
        _ => None,
    };
    let view_from_world = eye.affine().inverse();
    for visual in visuals.map.values().filter(|v| !v.labels.is_empty()) {
        let Ok((root, shown)) = roots.get(visual.root) else {
            continue;
        };
        if !shown.get() {
            continue;
        }
        for label in &visual.labels {
            let world = root.transform_point(label.at);
            let Ok(screen) = camera.world_to_viewport(eye, world) else {
                continue;
            };
            let depth = -view_from_world.transform_point3(world).z;
            let size_px = if label.size < 0.0 {
                -label.size
            } else {
                fov.map_or(14.0, |f| label.size * size.y / (2.0 * (0.5 * f).tan() * depth.max(1e-3)))
            };
            let opacity = 1.0 - ((depth - LABEL_FADE_M.0) / (LABEL_FADE_M.1 - LABEL_FADE_M.0)).clamp(0.0, 1.0);
            if size_px < MIN_LABEL_PX || opacity <= 0.0 {
                continue;
            }
            labels.0.push(DrawLabel {
                text: label.text.clone(),
                at: [screen.x / size.x, screen.y / size.y],
                color: label.color,
                background: label.background,
                accent: label.accent,
                size_px: size_px.min(MAX_LABEL_PX),
                centred: label.centred,
                opacity,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_axes_turn_into_the_sites_east_north_up() {
        let turn = enu_to_site();
        let close = |a: Vec3, b: Vec3| a.distance(b) < 1e-6;
        assert!(close(turn.transform_point(Vec3::X), Vec3::Z), "x east is the site's +z");
        assert!(close(turn.transform_point(Vec3::Y), Vec3::X), "y north is the site's +x");
        assert!(close(turn.transform_point(Vec3::Z), Vec3::Y), "z up is the site's +y");
    }

    #[test]
    fn a_pose_with_no_rotation_is_upright() {
        let e = DrawEntity { qw: 0.0, ..DrawEntity::points("t", "p", &[]).with_pose([1.0, 2.0, 3.0], [0.0; 4]) };
        assert_eq!(pose_of(&e), Transform::from_xyz(1.0, 2.0, 3.0));
    }
}
