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
mod data;
pub(crate) use data::pick;
mod drape;
mod frost;
mod ghost;
mod heat;
#[cfg(test)]
mod lifecycle_tests;
mod material;
mod mesh;
mod structure;
pub(crate) mod visual;

use std::collections::{HashMap, HashSet};

use bevy::asset::UntypedAssetId;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::core_pipeline::tonemapping::TonemappingLuts;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use gearbox_api::draw::{DrawItem, DrawKey};
use gearbox_api::{
    DrawClock, DrawEntity, DrawFrame, DrawStoreRes, DrawSystems, draw_flag, draw_kind,
};
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
        bevy::shader::load_shader_library!(app, "draw_frost.wgsl");
        bevy::shader::load_shader_library!(app, "draw_cloud_types.wgsl");
        bevy::shader::load_shader_library!(app, "draw_cloud_color.wgsl");
        bevy::asset::embedded_asset!(app, "draw_cloud.wgsl");
        bevy::asset::embedded_asset!(app, "draw_octree.wgsl");
        bevy::asset::embedded_asset!(app, "draw_stroke.wgsl");
        bevy::asset::embedded_asset!(app, "draw_surface.wgsl");
        bevy::asset::embedded_asset!(app, "draw_heat.wgsl");
        bevy::asset::embedded_asset!(app, "draw_ghost.wgsl");
        app.add_plugins((
            frost::FrostPlugin,
            data::DataPlugin,
            MaterialPlugin::<StrokeMaterial>::default(),
            MaterialPlugin::<SurfaceMaterial>::default(),
            MaterialPlugin::<heat::HeatMaterial>::default(),
            MaterialPlugin::<ghost::GhostMaterial>::default(),
        ))
        .init_resource::<DrawVisuals>()
        .add_systems(
            Update,
            (sync_draws, place_draws, ghost::build_ghosts)
                .chain()
                .after(DrawSystems::Serve),
        )
        .add_systems(
            PostUpdate,
            (project_draw_labels, ghost::update_patterns)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

/// Lifetimes count in simulated seconds.
fn tick_draw_clock(
    physics: Option<Res<crate::physics::PhysicsWorld>>,
    mut clock: ResMut<DrawClock>,
) {
    clock.0 = physics.map(|p| p.simulated_seconds);
}

#[derive(Component)]
struct DrawRoot;

struct Part {
    entity: Entity,
    mesh: Handle<Mesh>,
    material: UntypedAssetId,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    parent: Entity,
    transform: Transform,
}

impl Placement {
    fn with_pose(self, pose: Transform) -> Self {
        Self {
            transform: self.transform * pose,
            ..self
        }
    }
}

struct FrameState {
    frame: DrawFrame,
    pose: Transform,
    snapshot: bool,
    generation: u64,
    frozen: Option<Placement>,
}

impl FrameState {
    fn new(item: &DrawItem) -> Self {
        Self {
            frame: item.entity.frame(),
            pose: pose_of(&item.entity),
            snapshot: item.entity.flags & draw_flag::SNAPSHOT != 0,
            generation: item.generation,
            frozen: None,
        }
    }

    fn anchor_for(&self, item: &DrawItem) -> Option<Placement> {
        (self.snapshot
            && item.entity.flags & draw_flag::SNAPSHOT != 0
            && self.generation == item.generation
            && self.frame == item.entity.frame())
        .then_some(self.frozen)
        .flatten()
    }

    fn update(&mut self, item: &DrawItem) {
        let frozen = self.anchor_for(item);
        *self = Self {
            frozen,
            ..Self::new(item)
        };
    }
}

struct Visual {
    root: Entity,
    revision: u64,
    /// Hash of everything but the pose, to move without rebuilding.
    content: u64,
    placement: FrameState,
    /// Fill and strokes.
    parts: [Option<Part>; 2],
    compact: Option<std::sync::Arc<data::legacy::Points>>,
    compact_node: Option<Entity>,
    /// A grid's heat map, its material and what its mesh was built from.
    heat: Option<Part>,
    heat_material: Option<Handle<heat::HeatMaterial>>,
    heat_geometry: u64,
    labels: Vec<LabelSpec>,
    /// A panel's chart and its anchor in the root's space.
    panel: Option<(PanelChart, Vec3)>,
    /// The machine whose TF tree this entity switches on.
    tf: Option<String>,
    /// A ghost's request, mesh copies and source topology.
    ghost: Option<ghost::GhostSpec>,
    ghost_copies: Vec<Entity>,
    ghost_sources: Vec<(Entity, AssetId<Mesh>)>,
}

#[derive(Default)]
struct MaterialCache {
    strokes: HashMap<(bool, bool, u8), Handle<StrokeMaterial>>,
    surfaces: HashMap<(bool, bool, bool, bool, u8), Handle<SurfaceMaterial>>,
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
    let floats = [
        e.sx,
        e.sy,
        e.sz,
        e.width,
        e.glow,
        e.pulse_hz,
        e.flow_speed,
        e.dash,
        e.beam_height,
        e.fade,
        e.urgency,
        e.vmin,
        e.vmax,
        e.quant,
    ];
    floats.map(f32::to_bits).hash(&mut h);
    [
        e.kind, e.sub, e.flags, e.style, e.color, e.color2, e.colormap,
    ]
    .hash(&mut h);
    let bits = |v: &[f32]| v.iter().map(|f| f.to_bits()).collect::<Vec<u32>>();
    bits(&e.points).hash(&mut h);
    bits(&e.radii).hash(&mut h);
    bits(&e.instances).hash(&mut h);
    bits(&e.values).hash(&mut h);
    (
        &e.colors, &e.offsets, &e.rings, &e.qpoints, &e.ranges, &e.cells, &e.labels, &e.props,
    )
        .hash(&mut h);
    h.finish()
}

pub(crate) fn pose_of(e: &DrawEntity) -> Transform {
    let q = Quat::from_xyzw(e.qx as f32, e.qy as f32, e.qz as f32, e.qw as f32);
    let rotation = if q.length_squared() > 1e-8 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
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
    shared: Option<Res<data::runtime::Shared>>,
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
            .filter(|(k, item)| {
                visuals
                    .map
                    .get(*k)
                    .is_none_or(|v| v.revision != item.revision)
            })
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
            && v.placement.frame == frame
        {
            v.revision = item.revision;
            v.placement.update(item);
            continue;
        }
        let expanded = e.expanded();
        let place = if draping {
            let frozen = visuals
                .map
                .get(key)
                .and_then(|v| v.placement.anchor_for(item));
            let Some(place) = frozen
                .map(|anchor| anchor.with_pose(pose_of(e)).transform)
                .or_else(|| site_transform(&frame, pose_of(e), home_site))
            else {
                visuals.seen = u64::MAX;
                continue;
            };
            Some(place)
        } else {
            None
        };
        let draped = place.map(|p| drape::drape(&expanded, &p));
        let rise = place.map(|p| drape::rise_fn(&expanded, p));
        let ground: mesh::Ground = rise.as_ref().map(|r| r as &dyn Fn(f32, f32) -> f32);
        let compact = if let Some(shared) = &shared {
            match data::legacy::Points::prepare(
                shared.backend.budget(),
                &expanded,
                &key.0,
                &key.1,
                visuals
                    .map
                    .get(key)
                    .and_then(|visual| visual.compact.as_deref()),
            ) {
                Ok(points) => points,
                Err(error) => {
                    warn!("compact legacy {}/{} deferred: {error}", key.0, key.1);
                    visuals.seen = u64::MAX;
                    continue;
                }
            }
        } else {
            None
        };
        let built = if compact.is_some() {
            mesh::Built::default()
        } else {
            mesh::build_on(draped.as_ref().unwrap_or(&expanded), &key.0, &key.1, ground)
        };
        let on_top = e.flags & draw_flag::ON_TOP != 0;
        let visual = visuals.map.entry(key.clone()).or_insert_with(|| Visual {
            root: commands
                .spawn((
                    Name::new(format!("Draw[{}/{}]", key.0, key.1)),
                    DrawRoot,
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    ChildOf(home),
                ))
                .id(),
            revision: 0,
            content: 0,
            placement: FrameState::new(item),
            parts: Default::default(),
            compact: None,
            compact_node: None,
            heat: None,
            heat_material: None,
            heat_geometry: 0,
            labels: Vec::new(),
            panel: None,
            tf: None,
            ghost: None,
            ghost_copies: Vec::new(),
            ghost_sources: Vec::new(),
        });
        if let Some(shared) = &shared {
            if let Err(error) = data::legacy::set(
                &mut commands,
                &mut visual.compact_node,
                visual.root,
                compact.as_ref(),
                shared.backend.budget(),
            ) {
                warn!("compact legacy {}/{} node deferred: {error}", key.0, key.1);
                visuals.seen = u64::MAX;
                continue;
            }
        }
        visual.revision = item.revision;
        visual.content = content;
        visual.placement.update(item);
        visual.compact = compact;
        visual.labels = built.labels;
        visual.panel = (e.kind == draw_kind::PANEL).then(|| {
            (
                PanelChart::of(e),
                Vec3::Z * ground.map_or(0.0, |g| g(0.0, 0.0)),
            )
        });
        visual.tf = e.props().get("tf").filter(|_| e.kind == draw_kind::AXES);
        ghost::clear_copies(&mut commands, &mut visual.ghost_copies);
        visual.ghost_sources.clear();
        visual.ghost = (e.kind == draw_kind::GHOST).then(|| {
            let fx = mesh::Fx::of(e, gearbox_api::draw_style::GHOST);
            ghost::GhostSpec {
                machine: e.props().get("machine").unwrap_or_default(),
                instances: draped.as_ref().unwrap_or(&expanded).instance_list(),
                color: if e.color != 0 {
                    gearbox_api::wire::draw::unpack_rgba(e.color)
                } else {
                    [0x56, 0xB4, 0xE9, 255]
                },
                colors: e.colors.clone(),
                glow: fx.glow,
                pulse: fx.pulse_hz,
                fade: fx.fade,
                phase: 0.0,
                blur: visual::blur(e, 0.0),
            }
        });

        let on_top = on_top || built.on_top;
        let cache = &mut visuals.materials;
        let splat = built.splat;
        let blur_key = visual::blur_key(e);
        let blur = blur_key as f32 / 255.0;
        let stroke = (!built.strokes.is_empty()).then(|| {
            let material = cache
                .strokes
                .entry((on_top, splat, blur_key))
                .or_insert_with(|| {
                    strokes.add(StrokeMaterial {
                        unused: Vec4::new(blur, 0.0, 0.0, 0.0),
                        lut: luts.agx.clone(),
                        on_top,
                        splat,
                    })
                })
                .clone();
            (built.strokes.into_mesh(), material)
        });
        let fill = (!built.fill.is_empty()).then(|| {
            let translucent = built.fill.translucent();
            let double_sided = built.double_sided || translucent;
            let material = cache
                .surfaces
                .entry((on_top, translucent, double_sided, built.shaded, blur_key))
                .or_insert_with(|| {
                    surfaces.add(SurfaceMaterial {
                        flags: Vec4::new(built.shaded as u32 as f32, blur, 0.0, 0.0),
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
        let grid = (e.kind == draw_kind::GRID)
            .then(|| heat::images(&expanded))
            .flatten();
        match grid {
            Some((cells, values, size)) => {
                let (cells, values) = (images.add(cells), images.add(values));
                let params = heat::params(&expanded, size);
                let material = match visual
                    .heat_material
                    .as_ref()
                    .filter(|h| heats.get(*h).is_some())
                {
                    Some(h) => {
                        let mut m = heats.get_mut(h).unwrap();
                        (m.cells, m.values, m.params, m.on_top) = (cells, values, params, on_top);
                        h.clone()
                    }
                    None => {
                        let h = heats.add(heat::HeatMaterial {
                            params,
                            lut: luts.agx.clone(),
                            cells,
                            values,
                            on_top,
                        });
                        visual.heat_material = Some(h.clone());
                        h
                    }
                };
                let geometry = heat::geometry_key(&expanded, place);
                if visual.heat_geometry != geometry || visual.heat.is_none() {
                    let mesh = heat::mesh(&expanded, ground).map(|m| (m, material));
                    set_part(&mut commands, &mut visual.heat, root, mesh, &mut meshes);
                    visual.heat_geometry = geometry;
                }
            }
            None => {
                set_part::<heat::HeatMaterial>(
                    &mut commands,
                    &mut visual.heat,
                    root,
                    None,
                    &mut meshes,
                );
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
                commands
                    .entity(part.entity)
                    .insert(MeshMaterial3d(material));
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
            *slot = Some(Part {
                entity,
                mesh: handle,
                material: material_id,
            });
        }
    }
}

/// Entity coordinates into the home site's for the frames that hang under
/// it, `None` for a link frame or a machine not yet put down.
pub(crate) fn site_transform(
    frame: &DrawFrame,
    pose: Transform,
    home: &crate::globe::SiteEntry,
) -> Option<Transform> {
    match frame {
        DrawFrame::World => Some(enu_to_site() * pose),
        DrawFrame::Odom(machine) => {
            let own = crate::globe::machine_datum(machine)?;
            let turn = home.frame.rotation.inverse();
            let datum =
                Transform::from_translation((turn * (own.origin - home.frame.origin)).as_vec3())
                    .with_rotation((turn * own.rotation).as_quat());
            Some(datum * enu_to_site() * pose)
        }
        DrawFrame::Link(..) => None,
    }
}

/// Where a draw root hangs and how, or `None` while its frame is missing.
fn resolve(
    v: &mut FrameState,
    home: &crate::globe::SiteEntry,
    inventory: &crate::controller::ControllerInventory,
    index: &crate::prim_index::PrimIndex,
    globals: &Query<&GlobalTransform>,
) -> Option<Placement> {
    if let Some(frozen) = v.frozen {
        return Some(frozen.with_pose(v.pose));
    }
    let pose = if v.snapshot {
        Transform::IDENTITY
    } else {
        v.pose
    };
    let placement = match &v.frame {
        DrawFrame::World | DrawFrame::Odom(_) => Placement {
            parent: home.entity,
            transform: site_transform(&v.frame, pose, home)?,
        },
        DrawFrame::Link(machine, link) => {
            let spec = inventory.machines.iter().find(|m| m.id == *machine)?;
            let scene_root = spec.scene_root?;
            let link = if link.is_empty() {
                spec.links.base().or(spec.links.links.first())
            } else {
                spec.links.get(link)
            }?;
            let entity = index.find(scene_root, &link.prim_path)?;
            let scale = globals
                .get(entity)
                .ok()?
                .to_scale_rotation_translation()
                .0
                .x;
            let s = if scale.abs() > 1e-6 { 1.0 / scale } else { 1.0 };
            let transform = Transform {
                translation: pose.translation * s,
                rotation: pose.rotation,
                scale: Vec3::splat(s),
            };
            Placement {
                parent: entity,
                transform,
            }
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
            Placement {
                parent: home.entity,
                transform,
            }
        };
        v.frozen = Some(frozen);
        return Some(frozen.with_pose(v.pose));
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
        let placement = resolve(&mut visual.placement, home, &inventory, &index, &globals);
        let Ok((mut transform, mut visibility, parent)) = roots.get_mut(visual.root) else {
            continue;
        };
        let shown = if placement.is_some() && toggles.show_draws {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != shown {
            *visibility = shown;
        }
        let Some(placement) = placement else {
            continue;
        };
        if parent.parent() != placement.parent {
            commands
                .entity(visual.root)
                .insert(ChildOf(placement.parent));
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PanelGlass {
    Color {
        light: bool,
    },
    White,
    #[default]
    Black,
}

impl PanelGlass {
    pub(crate) fn of(finish: &str, theme: &str) -> Self {
        let light = theme.eq_ignore_ascii_case("light");
        match finish.to_ascii_lowercase().as_str() {
            "color" => Self::Color { light },
            "white" => Self::White,
            "black" => Self::Black,
            _ if light => Self::White,
            _ => Self::Black,
        }
    }

    pub fn light(self) -> bool {
        matches!(self, Self::White | Self::Color { light: true })
    }
}

/// What a panel shows, read from its entity.
#[derive(Debug, Clone, Default)]
pub struct PanelChart {
    pub blur: f32,
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
    pub world_anchor: bool,
    pub max_distance: Option<f32>,
    pub glow: f32,
    pub glass: PanelGlass,
}

impl PanelChart {
    fn of(e: &DrawEntity) -> Self {
        let p = e.props();
        let get = |k: &str| p.get(k).unwrap_or_default();
        let color = if e.color != 0 {
            gearbox_api::wire::draw::unpack_rgba(e.color)
        } else {
            visual::ACCENT
        };
        Self {
            kind: e.sub,
            blur: visual::blur(e, 1.0),
            title: get("title"),
            unit: get("unit"),
            text: get("text"),
            values: e.values.clone(),
            labels: e.label_list(),
            colors: e
                .colors
                .iter()
                .map(|c| gearbox_api::wire::draw::unpack_rgba(*c))
                .collect(),
            color,
            min: e.prop_f32("min").unwrap_or(0.0),
            max: e.prop_f32("max").unwrap_or(100.0),
            max_given: e.prop_f32("max").is_some(),
            rows: get("rows")
                .lines()
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .collect(),
            map: (e.colormap, e.vmin, e.vmax),
            badge: p.get("badge").is_some(),
            world_anchor: matches!(p.get("world_anchor").as_deref(), Some("true" | "1")),
            max_distance: e
                .prop_f32("label_max_distance")
                .filter(|distance| distance.is_finite() && *distance > 0.0),
            glow: visual::effect(e.glow, 0.65),
            glass: PanelGlass::of(&get("panel_finish"), &get("panel_theme")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnnotationMeta {
    pub blur: Option<f32>,
    pub category: String,
    pub detail: String,
    pub icon: String,
    pub selected: bool,
    pub background_explicit: bool,
    pub max_distance: Option<f32>,
    pub glow: Option<f32>,
}

impl AnnotationMeta {
    pub(super) fn background(entity: &DrawEntity) -> Option<[u8; 4]> {
        entity
            .props()
            .get("label_background")
            .and_then(|value| {
                value
                    .parse::<u32>()
                    .ok()
                    .or_else(|| gearbox_api::wire::draw::parse_color(&value))
            })
            .or_else(|| {
                (!matches!(entity.kind, draw_kind::SHAPES | draw_kind::POLYGON)
                    && entity.color2 != 0)
                    .then_some(entity.color2)
            })
            .map(gearbox_api::wire::draw::unpack_rgba)
    }

    pub(super) fn of(entity: &DrawEntity) -> Self {
        let props = entity.props();
        Self {
            category: props.get("label_category").unwrap_or_default(),
            blur: entity
                .prop_f32("blur")
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 1.0)),
            detail: props.get("label_detail").unwrap_or_default(),
            icon: props.get("label_icon").unwrap_or_default(),
            selected: matches!(props.get("label_selected").as_deref(), Some("true" | "1")),
            background_explicit: Self::background(entity).is_some(),
            max_distance: props
                .get_f64("label_max_distance")
                .map(|distance| distance as f32)
                .filter(|distance| distance.is_finite() && *distance > 0.0),
            glow: (entity.glow != 0.0).then_some(entity.glow.max(0.0)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DrawLabel {
    pub text: String,
    pub at: [f32; 2],
    pub color: [u8; 4],
    pub background: [u8; 4],
    /// Structural accent, independent of text color.
    pub accent: [u8; 4],
    pub annotation: AnnotationMeta,
    pub size_px: f32,
    /// Centred on the point rather than beside it on a leader line.
    pub centred: bool,
    /// 1 near, fading to 0 far away.
    pub opacity: f32,
}

const MIN_LABEL_PX: f32 = 5.0;
const MAX_LABEL_PX: f32 = 160.0;
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
            if chart.kind == gearbox_api::panel_kind::LEGEND && !chart.world_anchor {
                panels.0.push(DrawPanel {
                    key: key.clone(),
                    chart: chart.clone(),
                    at: None,
                    opacity: 1.0,
                });
                continue;
            }
            let world = root.transform_point(*anchor);
            let Ok(screen) = camera.world_to_viewport(eye, world) else {
                continue;
            };
            let depth = -view_from_world.transform_point3(world).z;
            let fade_end = chart.max_distance.unwrap_or(PANEL_FADE_M.1);
            let fade_start = fade_end * PANEL_FADE_M.0 / PANEL_FADE_M.1;
            let opacity = 1.0 - ((depth - fade_start) / (fade_end - fade_start)).clamp(0.0, 1.0);
            if opacity > 0.0 && depth > 0.0 {
                panels.0.push(DrawPanel {
                    key: key.clone(),
                    chart: chart.clone(),
                    at: Some([screen.x / size.x, screen.y / size.y]),
                    opacity,
                });
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
    let mut ordered: Vec<_> = visuals
        .map
        .iter()
        .filter(|(_, v)| !v.labels.is_empty())
        .collect();
    ordered.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (_, visual) in ordered {
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
                fov.map_or(14.0, |f| {
                    label.size * size.y / (2.0 * (0.5 * f).tan() * depth.max(1e-3))
                })
            };
            let opacity = visual::label_opacity(depth, label.annotation.max_distance);
            if depth <= 0.0 || size_px < MIN_LABEL_PX || opacity <= 0.0 {
                continue;
            }
            labels.0.push(DrawLabel {
                text: label.text.clone(),
                at: [screen.x / size.x, screen.y / size.y],
                color: label.color,
                background: label.background,
                accent: label.accent,
                annotation: label.annotation.clone(),
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
    fn snapshot_updates_keep_the_captured_frame_but_apply_the_new_local_pose() {
        let mut store = gearbox_api::draw::DrawStore::default();
        let source = DrawEntity::points("t", "s", &[[0.0; 3]])
            .in_frame("rover/base_link")
            .with_flags(draw_flag::SNAPSHOT)
            .at_yaw([1.0, 2.0, 3.0], 0.3);
        store.apply(source.clone(), 0.0).unwrap();
        let mut state = FrameState::new(store.get("t", "s").unwrap());
        let anchor = Placement {
            parent: Entity::PLACEHOLDER,
            transform: Transform::from_xyz(10.0, 20.0, 30.0)
                .with_rotation(Quat::from_rotation_z(0.5)),
        };
        state.frozen = Some(anchor);
        for msg in [
            DrawEntity::update("t", "s")
                .with_glow(0.2)
                .updating(&["glow"]),
            DrawEntity::update("t", "s")
                .with_prop("label_detail", "still here")
                .updating(&["props"]),
            DrawEntity::update("t", "s")
                .at_yaw([2.0, 4.0, 6.0], 0.6)
                .updating(&["pose"]),
        ] {
            store.apply(msg, 1.0).unwrap();
            state.update(store.get("t", "s").unwrap());
            assert_eq!(state.frozen, Some(anchor));
        }
        let placed = state.frozen.unwrap().with_pose(state.pose);
        assert!(placed.transform.translation.abs_diff_eq(
            anchor.transform.transform_point(Vec3::new(2.0, 4.0, 6.0)),
            1e-5
        ));
        assert!(
            placed
                .transform
                .rotation
                .abs_diff_eq(Quat::from_rotation_z(1.1), 1e-5)
        );
        store.apply(source, 2.0).unwrap();
        state.update(store.get("t", "s").unwrap());
        assert!(
            state.frozen.is_none(),
            "replacement captures the frame again"
        );
    }

    #[test]
    fn changing_frame_or_snapshot_flag_releases_the_previous_anchor() {
        let mut store = gearbox_api::draw::DrawStore::default();
        for change in [
            DrawEntity::update("t", "s")
                .with_prop("frame", "world")
                .updating(&["props"]),
            DrawEntity::update("t", "s").updating(&["flags"]),
        ] {
            store
                .apply(
                    DrawEntity::points("t", "s", &[[0.0; 3]])
                        .in_frame("rover/base_link")
                        .with_flags(draw_flag::SNAPSHOT),
                    0.0,
                )
                .unwrap();
            let mut state = FrameState::new(store.get("t", "s").unwrap());
            state.frozen = Some(Placement {
                parent: Entity::PLACEHOLDER,
                transform: Transform::IDENTITY,
            });
            store.apply(change, 1.0).unwrap();
            assert!(state.anchor_for(store.get("t", "s").unwrap()).is_none());
            state.update(store.get("t", "s").unwrap());
            assert!(state.frozen.is_none());
        }
    }

    #[test]
    fn annotation_metadata_is_explicit_and_preserves_user_text() {
        let raw = DrawEntity::text("warning", "selected", &[[0.0; 3]], "robot α: 1.0 m");
        assert_eq!(AnnotationMeta::of(&raw), AnnotationMeta::default());
        let configured = raw
            .with_prop("label_category", "Sensor A")
            .with_prop("label_detail", "0.4 m · σ")
            .with_prop("label_icon", "sensor")
            .with_prop("label_selected", "true");
        let meta = AnnotationMeta::of(&configured);
        assert_eq!(meta.category, "Sensor A");
        assert_eq!(meta.detail, "0.4 m · σ");
        assert_eq!(meta.icon, "sensor");
        assert!(meta.selected);
        let transparent = DrawEntity {
            color2: 1,
            ..configured
        };
        assert!(AnnotationMeta::of(&transparent).background_explicit);
    }

    #[test]
    fn annotation_ranges_are_optional_finite_and_positive() {
        let label = DrawEntity::text("t", "label", &[[0.0; 3]], "Name");
        for value in ["0", "-1", "NaN", "inf", "invalid"] {
            assert_eq!(
                AnnotationMeta::of(&label.clone().with_prop("label_max_distance", value))
                    .max_distance,
                None
            );
        }
        assert_eq!(
            AnnotationMeta::of(&label.with_prop("label_max_distance", "500")).max_distance,
            Some(500.0)
        );
    }

    #[test]
    fn panels_and_annotations_preserve_explicit_color_and_glow_controls() {
        use gearbox_api::wire::draw::rgba;
        let e = gearbox_api::wire::draw_widgets::legend(
            "t",
            "scale",
            gearbox_api::colormap::VIRIDIS,
            -2.0,
            4.0,
            "Scale",
            "m",
        )
        .with_color(rgba(233, 47, 161, 127))
        .with_colors(&[0, rgba(31, 211, 97, 204), 1])
        .with_glow(-1.0);
        let panel = PanelChart::of(&e);
        assert_eq!(panel.color, [233, 47, 161, 127]);
        assert_eq!(panel.colors, [[0; 4], [31, 211, 97, 204], [1, 0, 0, 0]]);
        assert_eq!(panel.glow, 0.0);
        assert_eq!(panel.map, (gearbox_api::colormap::VIRIDIS, -2.0, 4.0));
        assert_eq!(AnnotationMeta::of(&e).glow, Some(0.0));
        assert_eq!(AnnotationMeta::of(&e.with_glow(1.25)).glow, Some(1.25));
    }

    #[test]
    fn world_legend_is_opt_in_and_keeps_its_scalar_mapping() {
        let legend = gearbox_api::wire::draw_widgets::legend(
            "t",
            "legend",
            gearbox_api::colormap::VIRIDIS,
            -2.0,
            4.0,
            "Pressure",
            "Pa",
        );
        let fixed = PanelChart::of(&legend);
        assert!(!fixed.world_anchor);
        let world = PanelChart::of(
            &legend
                .with_prop("world_anchor", "true")
                .with_prop("label_max_distance", "35"),
        );
        assert!(world.world_anchor);
        assert_eq!(world.max_distance, Some(35.0));
        assert_eq!(world.map, fixed.map);
        assert_eq!(world.unit, "Pa");
    }

    #[test]
    fn panel_theme_selects_contrast_without_recoloring_data() {
        let legend = gearbox_api::wire::draw_widgets::legend(
            "t",
            "scale",
            gearbox_api::colormap::VIRIDIS,
            -2.0,
            4.0,
            "Scale",
            "m",
        )
        .with_color(gearbox_api::draw_color::MAGENTA);
        let dark = PanelChart::of(&legend);
        assert!(!dark.glass.light());
        for theme in ["light", "LIGHT", "dark", "unknown"] {
            let panel = PanelChart::of(&legend.clone().with_prop("panel_theme", theme));
            assert_eq!(panel.glass.light(), theme.eq_ignore_ascii_case("light"));
            assert_eq!(
                (panel.color, panel.map, panel.glow),
                (dark.color, dark.map, dark.glow)
            );
        }
    }

    #[test]
    fn panel_finishes_select_material_and_ink_without_changing_data() {
        let source = gearbox_api::wire::draw_widgets::legend(
            "t",
            "scale",
            gearbox_api::colormap::VIRIDIS,
            -2.0,
            4.0,
            "Scale",
            "m",
        )
        .with_color(gearbox_api::draw_color::MAGENTA)
        .with_glow(-1.0)
        .with_values(&[0.25, 0.75])
        .with_colors(&[gearbox_api::draw_color::GREEN]);
        let original = PanelChart::of(&source);
        for theme in ["dark", "light"] {
            for (finish, expected) in [
                (
                    "color",
                    PanelGlass::Color {
                        light: theme == "light",
                    },
                ),
                ("WHITE", PanelGlass::White),
                ("black", PanelGlass::Black),
                (
                    "unknown",
                    if theme == "light" {
                        PanelGlass::White
                    } else {
                        PanelGlass::Black
                    },
                ),
            ] {
                let entity = source
                    .clone()
                    .with_prop("panel_theme", theme)
                    .with_prop("panel_finish", finish);
                let panel = PanelChart::of(&entity);
                assert_eq!(panel.glass, expected);
                assert_eq!(
                    (
                        &panel.values,
                        &panel.colors,
                        panel.color,
                        panel.map,
                        panel.glow
                    ),
                    (
                        &original.values,
                        &original.colors,
                        original.color,
                        original.map,
                        original.glow
                    )
                );
            }
        }
    }

    #[test]
    fn entity_axes_turn_into_the_sites_east_north_up() {
        let turn = enu_to_site();
        let close = |a: Vec3, b: Vec3| a.distance(b) < 1e-6;
        assert!(
            close(turn.transform_point(Vec3::X), Vec3::Z),
            "x east is the site's +z"
        );
        assert!(
            close(turn.transform_point(Vec3::Y), Vec3::X),
            "y north is the site's +x"
        );
        assert!(
            close(turn.transform_point(Vec3::Z), Vec3::Y),
            "z up is the site's +y"
        );
    }

    #[test]
    fn a_pose_with_no_rotation_is_upright() {
        let e = DrawEntity {
            qw: 0.0,
            ..DrawEntity::points("t", "p", &[]).with_pose([1.0, 2.0, 3.0], [0.0; 4])
        };
        assert_eq!(pose_of(&e), Transform::from_xyz(1.0, 2.0, 3.0));
    }
}
