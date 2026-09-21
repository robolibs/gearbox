//! CDLOD quadtree: per-frame node selection, heightmap-tile cache (atlas layer
//! LRU), and the pooled node entities that get drawn.
//!
//! Selection walks each cube face's implicit quadtree by camera distance.
//! Refinement into 4 children is *gated on their tiles being resident* in the
//! atlas — until baked, the parent keeps rendering (coarser, never holes).

use bevy::camera::primitives::{Frustum, Sphere};
use bevy::mesh::MeshTag;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use big_space::prelude::*;

use crate::config::*;
use crate::debug::DebugSettings;
use super::bake::{TileBakeQueue, TileBakeRequest};
use super::cube_sphere::face_uv_to_dir;
use super::material::{TerrainGlobals, TerrainMaterial, TerrainNodeGpu, WaterMaterial};
use super::RootGrid;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId {
    pub face: u8,
    pub depth: u8,
    pub x: u32,
    pub y: u32,
}

impl NodeId {
    pub fn root(face: u8) -> Self {
        Self { face, depth: 0, x: 0, y: 0 }
    }

    /// (min-corner face uv, extent in face-uv units). Exact dyadic values.
    pub fn origin_scale(&self) -> (Vec2, f32) {
        let scale = 2.0 / (1u32 << self.depth) as f32;
        (
            Vec2::new(-1.0 + scale * self.x as f32, -1.0 + scale * self.y as f32),
            scale,
        )
    }

    pub fn lod(&self, cfg: &PlanetConfig) -> u32 {
        (cfg.max_depth - self.depth) as u32
    }

    pub fn children_if_below_leaf(&self, cfg: &PlanetConfig) -> Option<[NodeId; 4]> {
        (self.depth < cfg.max_depth).then(|| self.children())
    }

    pub fn children(&self) -> [NodeId; 4] {
        let (f, d) = (self.face, self.depth + 1);
        let (x, y) = (self.x * 2, self.y * 2);
        [
            NodeId { face: f, depth: d, x, y },
            NodeId { face: f, depth: d, x: x + 1, y },
            NodeId { face: f, depth: d, x, y: y + 1 },
            NodeId { face: f, depth: d, x: x + 1, y: y + 1 },
        ]
    }

    /// Planet-local bounding sphere of the node's base (sea-level) patch:
    /// (center, corner-chord radius). Height inflation is the caller's
    /// business — culling needs it fully conservative, but the LOD distance
    /// metric must not add ~height_amp to small nodes or everything near the
    /// ground reads as distance ~0 and splits to leaves (thousands of nodes).
    pub fn bounding_sphere(&self, cfg: &PlanetConfig) -> (Vec3, f32) {
        let (origin, scale) = self.origin_scale();
        let center_dir = face_uv_to_dir(self.face, origin + Vec2::splat(scale * 0.5));
        let center = center_dir * cfg.radius;
        let mut radius: f32 = 0.0;
        for (cx, cy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            let dir = face_uv_to_dir(self.face, origin + Vec2::new(cx, cy) * scale);
            radius = radius.max((dir * cfg.radius - center).length());
        }
        (center, radius)
    }

    /// Approximate planet-local side length of the node's patch.
    pub fn world_size(&self, cfg: &PlanetConfig) -> f32 {
        // A face spans ~pi/2 of arc over 2.0 uv units.
        cfg.radius * 0.785 * self.origin_scale().1
    }
}

struct TileEntry {
    layer: u32,
    requested_frame: u64,
    /// Last frame this tile was touched at all (emit or child-prefetch).
    last_touch: u64,
    /// Last frame this tile was touched by an emitted (rendered) node.
    last_emit: u64,
}

/// Per-planet bake context: which atlas the tiles go to and the noise domain
/// that defines this world.
#[derive(Clone, Copy)]
pub struct BakeCtx {
    pub atlas: AssetId<Image>,
    pub seed: Vec3,
    pub freq: f32,
}

/// Which heightmap tiles live in which atlas layer (one per planet).
pub struct TileCache {
    entries: HashMap<NodeId, TileEntry>,
    free_layers: Vec<u32>,
    pub frame: u64,
    /// Allocation failures this frame (atlas saturated). Nonzero sustained
    /// values mean demand exceeds the planet's atlas_layers and quality is
    /// frozen.
    pub starved: u32,
}

impl TileCache {
    pub fn new(atlas_layers: u32) -> Self {
        Self {
            entries: HashMap::new(),
            free_layers: (0..atlas_layers).rev().collect(),
            frame: 0,
            starved: 0,
        }
    }
}

impl TileCache {
    /// Touch the tile, requesting a bake if absent (subject to `budget`).
    /// Returns true if the tile is resident (safe to render).
    /// `for_emit` marks the touch as "this tile is being rendered", which
    /// protects it from eviction and lets emit-requests evict prefetch-only
    /// tiles under atlas pressure.
    fn ensure(
        &mut self,
        id: NodeId,
        queue: &mut TileBakeQueue,
        budget: &mut usize,
        for_emit: bool,
        ctx: &BakeCtx,
    ) -> bool {
        let frame = self.frame;
        if let Some(e) = self.entries.get_mut(&id) {
            e.last_touch = frame;
            if for_emit {
                e.last_emit = frame;
            }
            return frame >= e.requested_frame + BAKE_LATENCY_FRAMES;
        }
        if *budget == 0 {
            return false;
        }
        let layer = match self.free_layers.pop() {
            Some(l) => l,
            None => match self.evict(for_emit) {
                Some(l) => l,
                None => {
                    self.starved += 1;
                    return false;
                }
            },
        };
        *budget -= 1;
        self.entries.insert(
            id,
            TileEntry {
                layer,
                requested_frame: frame,
                last_touch: frame,
                last_emit: if for_emit { frame } else { 0 },
            },
        );
        let (origin, scale) = id.origin_scale();
        queue.requests.push((
            ctx.atlas,
            TileBakeRequest {
                origin,
                scale,
                face: id.face as u32,
                seed: ctx.seed,
                layer,
                freq: ctx.freq,
            },
        ));
        false
    }

    /// Evict the least-recently-touched tile. In-flight ("warming") bakes are
    /// never victims — evicting one wastes the bake and resets its sibling
    /// group's refinement, which livelocks under atlas pressure. Cold tiles
    /// (grace period expired) go first; under pressure, an emit-request may
    /// also evict tiles that were merely prefetched recently.
    fn evict(&mut self, for_emit: bool) -> Option<u32> {
        let frame = self.frame;
        let warming =
            |e: &TileEntry| frame < e.requested_frame + BAKE_LATENCY_FRAMES;
        let victim = self
            .entries
            .iter()
            .filter(|(id, e)| {
                id.depth > PIN_DEPTH
                    && !warming(e)
                    && e.last_touch + EVICT_GRACE_FRAMES < frame
            })
            .min_by_key(|(_, e)| e.last_touch)
            .map(|(id, _)| *id)
            .or_else(|| {
                if !for_emit {
                    return None;
                }
                // Pressure pass: still never touch the pinned spine, warming
                // bakes, or anything referenced this frame (an already-checked
                // sibling may be emitted later this same frame).
                self.entries
                    .iter()
                    .filter(|(id, e)| {
                        id.depth > PIN_DEPTH && !warming(e) && e.last_touch < frame
                    })
                    .min_by_key(|(_, e)| (e.last_emit, e.last_touch))
                    .map(|(id, _)| *id)
            })?;
        self.entries.remove(&victim).map(|e| e.layer)
    }

    fn layer_of(&self, id: &NodeId) -> Option<u32> {
        self.entries.get(id).map(|e| e.layer)
    }

    pub fn resident_count(&self) -> usize {
        self.entries.len()
    }
}

/// All render-side state for one planet: its tile cache, slot assignment,
/// GPU handles and the pooled tile entities (which are children of the planet
/// entity, so their world transform follows the planet).
#[derive(Component)]
pub struct PlanetRenderer {
    pub cache: TileCache,
    pub slots: SlotMap,
    pub stats: TerrainStats,
    pub material: Handle<TerrainMaterial>,
    pub water_material: Handle<WaterMaterial>,
    pub node_buffer: Handle<ShaderBuffer>,
    /// Also keeps the atlas asset alive (the bake queue only carries ids).
    pub atlas: Handle<Image>,
    /// index == MeshTag == node-buffer slot; water mirrors terrain slot-for-slot.
    pub terrain_pool: Vec<Entity>,
    /// Used by the per-planet visibility gate (far planets hide both pools).
    #[allow(dead_code)]
    pub water_pool: Vec<Entity>,
    /// Last debug flags written into this planet's materials.
    pub last_flags: Option<u32>,
    /// Whether this planet is close enough to be worth drawing/selecting.
    /// Hysteretic, so a planet at the threshold doesn't blink.
    pub active: bool,
    /// Atmosphere anchor entity + this planet's real scattering medium
    /// (restored after the underwater vacuum swap). None for airless bodies.
    pub atmosphere: Option<(Entity, Handle<bevy::light::atmosphere::ScatteringMedium>)>,
}

impl PlanetRenderer {
    pub fn bake_ctx(&self, cfg: &PlanetConfig) -> BakeCtx {
        BakeCtx {
            atlas: self.atlas.id(),
            seed: cfg.seed,
            freq: cfg.noise_freq,
        }
    }
}

/// Persistent node -> pool-slot assignment. Nodes keep their slot for as long
/// as they stay selected, so entity visibility (and the retained transparent
/// phase that renders the water tiles) only changes at true selection-set
/// changes — per-frame slot shuffling made water tiles toggle
/// Visible/Hidden by the hundreds every frame and flicker during movement.
pub struct SlotEntry {
    slot: usize,
    /// Time the node was first emitted (drives the reveal morph blend).
    since: f32,
}

pub struct SlotMap {
    by_node: HashMap<NodeId, SlotEntry>,
    free: Vec<usize>,
}

impl Default for SlotMap {
    fn default() -> Self {
        Self {
            by_node: HashMap::new(),
            free: (0..MAX_VISIBLE).rev().collect(),
        }
    }
}

/// Per-frame, per-planet stats for the debug overlay.
#[derive(Default)]
pub struct TerrainStats {
    pub selected: usize,
    pub resident_tiles: usize,
    pub bakes_requested: usize,
    /// Nodes dropped because selection hit MAX_VISIBLE (should stay 0).
    pub dropped: usize,
    /// Atlas allocation failures this frame (should stay 0 once settled).
    pub starved: u32,
}

struct Selector<'a> {
    /// Camera position in *planet-local* space (planet rotation undone).
    cam: Vec3,
    /// Frustum built from the camera pose expressed in planet-local space.
    frustum: Frustum,
    cfg: &'a PlanetConfig,
    ctx: BakeCtx,
    cache: &'a mut TileCache,
    queue: &'a mut TileBakeQueue,
    budget: usize,
    dropped: usize,
    out: Vec<(NodeId, u32)>, // (node, atlas layer)
}

impl Selector<'_> {
    fn select(&mut self, id: NodeId) {
        if self.out.len() >= MAX_VISIBLE {
            self.dropped += 1;
            return;
        }
        let cfg = self.cfg;
        let lod = id.lod(cfg);
        let (center, chord) = id.bounding_sphere(cfg);
        // Frustum cull with the fully height-inflated radius (skip far plane;
        // margin softens edge popping during fast rotation). Culled subtrees
        // are skipped entirely — they re-refine when back in view.
        if !self.frustum.intersects_sphere(
            &Sphere {
                center: center.into(),
                radius: (chord + cfg.height_amp * 1.65) * 1.05 + 50.0,
            },
            false,
        ) {
            return;
        }
        // Horizon cull: a planet hides its own far side, and the frustum has
        // no idea. Standing on the surface, twelve thousand kilometres of the
        // far hemisphere sit inside the frustum, directly overhead. Their
        // ground faces away and is culled, but the skirts that close the
        // cracks between tiles face inward and are not — so they drew a grid
        // across the whole sky, and the same tiles laid its shadow on the
        // ground when seen from above.
        //
        // A point is over the horizon for an eye at `cam` when its dot with
        // the eye, both from the planet's centre, falls below the occluding
        // radius squared. Taking the most favourable point of the node's ball
        // and the lowest ground there can be keeps it conservative: nothing
        // that could be seen is ever dropped. In f64, because the radius
        // squared is 4e13 and an f32 cannot tell two of those apart.
        if beyond_horizon(
            self.cam,
            center,
            chord + cfg.height_amp * 1.65,
            cfg.radius - cfg.height_amp,
        ) {
            return;
        }
        // LOD distance: cap the height term at the node's own size so small
        // nodes don't all read as "distance zero" near the surface.
        let lod_radius = chord + (cfg.height_amp * 1.5).min(id.world_size(cfg));
        let d_min = (self.cam - center).length() - lod_radius;
        let want_split = lod > 0 && d_min < cfg.lod_range(lod - 1);
        // Warm up children a bit before the split distance so the bake is
        // usually done when refinement actually happens.
        let want_prefetch = lod > 0 && d_min < cfg.lod_range(lod - 1) * PREFETCH_MARGIN;

        if want_prefetch {
            let mut kids = id.children();
            // Request nearest children first so close terrain refines first.
            kids.sort_by(|a, b| {
                let da = (self.cam - a.bounding_sphere(cfg).0).length_squared();
                let db = (self.cam - b.bounding_sphere(cfg).0).length_squared();
                da.total_cmp(&db)
            });
            let mut all_resident = true;
            for c in &kids {
                // No short-circuit: touch/request every child each frame.
                all_resident &=
                    self.cache
                        .ensure(*c, self.queue, &mut self.budget, false, &self.ctx);
            }
            if want_split && all_resident {
                for c in kids {
                    self.select(c);
                }
                return;
            }
        }
        // Emit self. Walk invariant: we only get here if our parent verified
        // us resident this frame (or we're a pinned-spine root), so this
        // succeeds except transiently at startup. There is deliberately no
        // downward fallback into resident descendants — that fragments the
        // emitted set explosively after teleports (missing mid-level tiles
        // with surviving fine ones), blowing past MAX_VISIBLE and dropping
        // whole subtrees as holes. Coarser-for-a-few-frames beats holes.
        if self
            .cache
            .ensure(id, self.queue, &mut self.budget, true, &self.ctx)
        {
            let layer = self.cache.layer_of(&id).unwrap();
            self.out.push((id, layer));
        }
    }
}

/// Runs in PostUpdate (after camera movement, before visibility checks):
/// per-planet selection, node buffer upload, pool sync, material globals.
pub fn select_and_sync(
    camera: Single<(&Transform, &CellCoord, &Projection), With<Camera3d>>,
    mut planets: Query<(Entity, &PlanetConfig, &mut PlanetRenderer, &CellCoord, &Transform)>,
    grids: Grids,
    root: Res<RootGrid>,
    mut queue: ResMut<TileBakeQueue>,
    debug: Res<DebugSettings>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    mut nodes_q: Query<&mut Visibility, With<MeshTag>>,
    mut planet_vis: Query<&mut Visibility, (With<PlanetConfig>, Without<MeshTag>)>,
    time: Res<Time>,
    camera_at: Option<Res<super::CameraAt>>,
) {
    if debug.freeze {
        return;
    }
    let (cam_tf, cam_cell, projection) = *camera;
    let flags = debug.flag_bits();
    let now = time.elapsed_secs();
    let grid = grids.get(root.0);
    // f64 world position: exact regardless of how far the camera has travelled.
    let told = camera_at.as_deref().copied();
    let cam_pos = told
        .map(|at| at.position)
        .unwrap_or_else(|| grid.grid_position_double(cam_cell, cam_tf));
    let cam_rot = told.map_or(cam_tf.rotation, |at| at.rotation);

    for (planet, cfg, mut renderer, planet_cell, planet_tf) in &mut planets {
        // Skip planets that are too far to contribute pixels. Hysteresis
        // (hide beyond 3000 radii, show inside 2000) keeps a planet near the
        // threshold from blinking; sub-pixel size is reached well before
        // 2000 radii, so nothing visible is ever dropped. The pinned tile
        // spine stays resident, so re-entry is instantly coarse-complete.
        let dist = super::planet_view(grid, cam_pos, planet, cfg, planet_cell, planet_tf).dist;
        if std::env::var_os("GEARBOX_PLANET_DEBUG").is_some() {
            bevy::log::info!(
                "planet {planet}: cam {cam_pos:?} dist {dist:.0} radius {:.0} active {}",
                cfg.radius, renderer.active
            );
        }
        let far = dist > cfg.radius * 3000.0;
        let near = dist < cfg.radius * 2000.0;
        if renderer.active && far {
            renderer.active = false;
        } else if !renderer.active && near {
            renderer.active = true;
        }
        if !renderer.active {
            if let Ok(mut vis) = planet_vis.get_mut(planet) {
                if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
            }
            continue;
        }

        // One deref through the change-detection wrapper, then field-disjoint
        // borrows (cache + slots + pools are all touched in the same pass).
        let ctx = renderer.bake_ctx(cfg);
        let PlanetRenderer {
            cache,
            slots,
            stats,
            material,
            water_material,
            node_buffer,
            terrain_pool,
            last_flags,
            ..
        } = &mut *renderer;

        cache.frame += 1;
        cache.starved = 0;
        // First frame: request the whole pinned spine (all depth <= PIN_DEPTH
        // tiles) so every region of the planet always has a resident coarse
        // ancestor, wherever the camera goes later.
        if cache.frame == 1 {
            let mut startup_budget = usize::MAX;
            for face in 0..6u8 {
                for depth in 0..=PIN_DEPTH {
                    for y in 0..(1u32 << depth) {
                        for x in 0..(1u32 << depth) {
                            let id = NodeId { face, depth, x, y };
                            cache.ensure(id, &mut queue, &mut startup_budget, false, &ctx);
                        }
                    }
                }
            }
        }

        // Camera pose expressed in this planet's local frame: all node
        // geometry, distances and the frustum live in planet-local space, so
        // the planet may sit anywhere and spin freely. The offset comes from
        // f64 grid positions, so it is exact at interplanetary range.
        let view = super::planet_view(grid, cam_pos, planet, cfg, planet_cell, planet_tf);
        let inv_rot = planet_tf.rotation.inverse();
        let local_cam = Transform {
            translation: view.local_cam,
            rotation: inv_rot * cam_rot,
            scale: Vec3::ONE,
        };
        // Built from the fresh Transform (not GlobalTransform, which is stale
        // until transform propagation later in PostUpdate).
        let frustum = projection.compute_frustum(&GlobalTransform::from(local_cam));

        let mut selector = Selector {
            cam: local_cam.translation,
            frustum,
            cfg,
            ctx,
            cache,
            queue: &mut queue,
            budget: MAX_BAKES_PER_FRAME,
            dropped: 0,
            out: Vec::with_capacity(MAX_VISIBLE),
        };
        for face in 0..6u8 {
            selector.select(NodeId::root(face));
        }
        let selected = selector.out;
        stats.selected = selected.len();
        stats.bakes_requested = MAX_BAKES_PER_FRAME - selector.budget.min(MAX_BAKES_PER_FRAME);
        stats.dropped = selector.dropped;
        stats.starved = cache.starved;
        stats.resident_tiles = cache.resident_count();
        if std::env::var_os("GEARBOX_PLANET_DEBUG").is_some() {
            bevy::log::info!(
                "select: {} nodes, dropped {}, starved {}, resident {}, baked {}, eye {:.0} m from the centre ({:.0} m up)",
                stats.selected, stats.dropped, stats.starved, stats.resident_tiles,
                stats.bakes_requested,
                view.local_cam.length(),
                view.local_cam.length() - cfg.radius
            );
        }

        // Release slots of nodes that fell out of the selection.
        let SlotMap { by_node, free } = slots;
        // Snapshot of last frame's on-screen nodes, for the reveal-blend decision.
        let prev_nodes: HashSet<NodeId> = by_node.keys().copied().collect();
        let selected_ids: HashSet<NodeId> = selected.iter().map(|(id, _)| *id).collect();
        let stale: Vec<NodeId> = by_node
            .keys()
            .filter(|id| !selected_ids.contains(*id))
            .copied()
            .collect();
        for id in stale {
            if let Some(e) = by_node.remove(&id) {
                free.push(e.slot);
            }
        }

        // Upload per-node data + sync pool entities (stable slots).
        let mut gpu_nodes = vec![TerrainNodeGpu::default(); MAX_VISIBLE];
        let mut used = vec![false; MAX_VISIBLE];
        for (id, layer) in &selected {
            // Selection is capped at MAX_VISIBLE, so a free slot always exists.
            let entry = by_node.entry(*id).or_insert_with(|| {
                // Reveal (start fully morphed, ease in) applies to refinement
                // swaps and fresh tiles; a COARSENING swap (this node's children
                // were on screen, at full morph == this node's lattice) is
                // already seamless at floor 0 — a reveal would pop it coarser.
                let coarsening = id
                    .children_if_below_leaf(cfg)
                    .is_some_and(|kids| kids.iter().any(|c| prev_nodes.contains(c)));
                SlotEntry {
                    slot: free.pop().expect("slot pool exhausted"),
                    since: if coarsening { now - REVEAL_SECS } else { now },
                }
            });
            let slot = entry.slot;
            used[slot] = true;
            let (origin, scale) = id.origin_scale();
            let lod = id.lod(cfg);
            let range = cfg.lod_range(lod);
            let start = range * MORPH_START_F;
            let end = range * MORPH_END_F;
            gpu_nodes[slot] = TerrainNodeGpu {
                origin,
                scale,
                face_lod: id.face as u32 | (lod << 3),
                morph_consts: Vec2::new(start, 1.0 / (end - start)),
                atlas_layer: *layer,
                morph_floor: (1.0 - (now - entry.since) / REVEAL_SECS).clamp(0.0, 1.0),
            };
        }
        // Only terrain entities toggle visibility (opaque binned phase). Water
        // entities stay permanently Visible: unused slots carry degenerate
        // (scale 0) records that rasterize nothing, so the transparent phase sees
        // a constant entity set — no per-frame item add/remove to lag or flicker.
        for slot in 0..MAX_VISIBLE {
            let want = if used[slot] {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
            if let Ok(mut vis) = nodes_q.get_mut(terrain_pool[slot]) {
                if *vis != want {
                    *vis = want;
                }
            }
        }
        if let Some(mut buf) = buffers.get_mut(&*node_buffer) {
            buf.set_data(gpu_nodes.as_slice());
        }
        // Touch the material assets only when the debug flags change; per-frame
        // camera/time data comes from the built-in view/globals bindings.
        if *last_flags != Some(flags) {
            *last_flags = Some(flags);
            let globals = TerrainGlobals {
                radius: cfg.radius,
                height_amp: cfg.height_amp,
                debug_flags: flags,
                _pad: 0,
            };
            if let Some(mut mat) = materials.get_mut(&*material) {
                mat.extension.globals = globals;
            }
            if let Some(mut mat) = water_materials.get_mut(&*water_material) {
                mat.extension.globals = globals;
            }
        }
        // Unhide only after this frame's node records are uploaded, so a
        // returning planet never shows a stale buffer.
        if let Ok(mut vis) = planet_vis.get_mut(planet) {
            if *vis != Visibility::Visible {
                *vis = Visibility::Visible;
            }
        }
    }
}

/// Whether every point of a node's ball is over the horizon from `eye`, all
/// measured from the planet's centre.
///
/// A point `x` on a sphere of radius `r` is over the horizon for an eye at
/// `e` when `dot(x, e) < r²`: that plane is where the tangent from the eye
/// touches. The most favourable point of a ball of radius `reach` about
/// `centre` adds `reach * |e|` to the dot, and `floor` is the lowest ground
/// there can be, so the answer is conservative — nothing that could be seen is
/// ever called hidden.
///
/// In f64 throughout: `r²` is 4e13 for Earth, and an f32 cannot tell two of
/// those apart, which is the whole quantity being compared.
fn beyond_horizon(eye: Vec3, centre: Vec3, reach: f32, floor: f32) -> bool {
    let floor = floor.max(1.0) as f64;
    let eye = eye.as_dvec3();
    let far = eye.length();
    // An eye below the lowest ground has no horizon to speak of.
    if far <= floor {
        return false;
    }
    eye.dot(centre.as_dvec3()) + (reach as f64) * far < floor * floor
}

#[cfg(test)]
mod horizon_tests {
    use super::*;

    const R: f32 = 6_371_000.0;
    const RELIEF: f32 = 9_000.0;

    /// Standing on the surface, the far side of the planet is hidden by the
    /// planet. Left drawn, its tiles' edge skirts face inward and rule a grid
    /// across the whole sky.
    #[test]
    fn the_far_side_is_hidden_from_someone_standing_on_the_near_side() {
        let eye = Vec3::new(0.0, R + 6.0, 0.0);
        for reach in [1.0, 1_000.0, 100_000.0] {
            let antipode = Vec3::new(0.0, -R, 0.0);
            assert!(
                beyond_horizon(eye, antipode, reach, R - RELIEF),
                "a node of reach {reach} m at the antipode was left visible"
            );
        }
    }

    #[test]
    fn the_ground_underfoot_is_never_hidden() {
        let eye = Vec3::new(0.0, R + 6.0, 0.0);
        assert!(!beyond_horizon(eye, Vec3::new(0.0, R, 0.0), 10.0, R - RELIEF));
        // Nor is ground a few kilometres away, still inside the horizon.
        assert!(!beyond_horizon(eye, Vec3::new(3_000.0, R, 0.0), 10.0, R - RELIEF));
    }

    /// From far out the visible cap grows but never reaches the silhouette:
    /// at four radii the horizon stands 75 degrees from the point underneath,
    /// so ground well inside that is kept and the far side still goes.
    #[test]
    fn from_orbit_the_cap_in_view_is_kept_and_the_rest_goes() {
        let eye = Vec3::new(0.0, R * 4.0, 0.0);
        let at = |degrees: f32| {
            let (s, c) = degrees.to_radians().sin_cos();
            Vec3::new(R * s, R * c, 0.0)
        };
        assert!(!beyond_horizon(eye, at(60.0), 1_000.0, R - RELIEF), "60 deg is in view");
        assert!(beyond_horizon(eye, at(120.0), 1_000.0, R - RELIEF), "120 deg is round the back");
        assert!(beyond_horizon(eye, Vec3::new(0.0, -R, 0.0), 1_000.0, R - RELIEF));
    }

    /// An eye under the lowest ground is inside the sphere, where the horizon
    /// test means nothing; it must not start hiding everything.
    #[test]
    fn an_eye_below_the_ground_hides_nothing() {
        let eye = Vec3::new(0.0, R - RELIEF - 100.0, 0.0);
        assert!(!beyond_horizon(eye, Vec3::new(0.0, -R, 0.0), 1.0, R - RELIEF));
    }
}
