//! Per-planet configuration, plus the global policy constants shared by every
//! planet (pool size, morph bands, cache tuning).

use bevy::prelude::*;

/// Everything that varies between planets. Lives on the planet entity; the
/// tile pools are its children and are driven through `PlanetRenderer`.
#[derive(Component, Clone, Debug)]
pub struct PlanetConfig {
    pub radius: f32,
    pub height_amp: f32,
    /// Quadtree leaf depth (lod = max_depth - depth).
    pub max_depth: u8,
    /// CDLOD range of lod 0; range(l) = lod0_range * 2^l.
    pub lod0_range: f32,
    /// Heightmap atlas layers = max resident tiles for this planet.
    pub atlas_layers: u32,
    /// Noise domain offset — distinct seeds give distinct worlds.
    pub seed: Vec3,
    /// Noise domain frequency over the unit sphere (feature scale).
    pub noise_freq: f32,
    /// Rotation rate about the planet's local Y axis, rad/s.
    pub spin: f32,
    /// Rayleigh scattering coefficients (m^-1, Earth-air values ~1e-5) of this
    /// world's atmosphere, or None for an airless body. Blue skies come from
    /// scattering that rises with frequency; swap the channels for alien skies.
    pub atmosphere: Option<Vec3>,
}

impl Default for PlanetConfig {
    fn default() -> Self {
        Self {
            radius: PLANET_RADIUS,
            height_amp: HEIGHT_AMP,
            max_depth: MAX_DEPTH,
            lod0_range: LOD0_RANGE,
            atlas_layers: ATLAS_LAYERS,
            seed: Vec3::ZERO,
            noise_freq: 3.0,
            spin: 0.0,
            atmosphere: Some(EARTH_RAYLEIGH),
        }
    }
}

/// Earth-air Rayleigh scattering, m^-1 (blue sky).
pub const EARTH_RAYLEIGH: Vec3 = Vec3::new(5.802e-6, 13.558e-6, 33.100e-6);

impl PlanetConfig {
    pub fn lod_range(&self, lod: u32) -> f32 {
        self.lod0_range * (1u64 << lod) as f32
    }

    /// A secondary world. `lod0_range` is derived from the leaf node size:
    /// it must comfortably exceed a leaf's diameter, or neighbouring nodes
    /// can differ by more than one LOD and crack.
    pub fn world(radius: f32, height_amp: f32, seed: Vec3, noise_freq: f32, spin: f32) -> Self {
        let leaf = leaf_size(radius, MAX_DEPTH);
        Self {
            radius,
            height_amp,
            lod0_range: leaf * 4.0,
            // Full-size atlas: the near-surface working set is ~1500 tiles on
            // any planet, so a smaller atlas starves (refinement freezes) as
            // soon as you land. Costs ~143 MB of VRAM per planet — lower this
            // for worlds you only ever see from orbit.
            atlas_layers: ATLAS_LAYERS,
            seed,
            noise_freq,
            spin,
            ..Default::default()
        }
    }

    /// Earth. Upstream's worlds are four to fifty kilometres across, where ten
    /// levels of quadtree put a leaf at twenty metres; on a planet six
    /// thousand kilometres across the same ten levels leave leaves ten
    /// kilometres wide, which is a ball with facets. Every doubling of depth
    /// halves the leaf, and the tile cache does not care how big the world is,
    /// so the depth goes up with the radius instead — sixteen levels put a
    /// leaf at about a hundred and fifty metres, under what the simulated
    /// ground square covers anyway.
    pub fn earth(radius: f32) -> Self {
        const DEPTH: u8 = 16;
        Self {
            radius,
            // Sea level to summit, over the whole planet.
            height_amp: 2_400.0,
            max_depth: DEPTH,
            lod0_range: leaf_size(radius, DEPTH) * 4.0,
            atlas_layers: ATLAS_LAYERS,
            seed: Vec3::new(7.0, -3.0, 11.0),
            noise_freq: 3.0,
            spin: 0.0,
            atmosphere: Some(EARTH_RAYLEIGH),
        }
    }

    /// Depth of the atmosphere shell above sea level: proportionally much
    /// thicker than Earth's (1.6% of R) so mountains stay inside it and the
    /// sky band is visible at flying altitudes on 4-50 km worlds.
    pub fn atmosphere_shell(&self) -> f32 {
        (self.height_amp * 4.0).max(self.radius * 0.2)
    }

    /// Scattering-density multiplier over Earth-air values. Mini planets can't
    /// have Earth's vertical optical depth AND Earth's horizontal visibility
    /// at once (compressing a 100 km column into a small shell makes ground
    /// haze opaque within ~1 km). Tuned for horizontal sea-level extinction
    /// length of roughly one planet radius: sky and limb stay visible, terrain
    /// stays readable.
    pub fn atmosphere_density(&self) -> f32 {
        // Earth-air blue-channel extinction length is ~1/33e-6 = 30 km; keep
        // haze-free viewing out to ~R by scaling density with 30 km / R.
        30_000.0 / self.radius
    }
}

/// Approximate world-space side length of a leaf node (a face spans ~pi/2 of
/// arc over 2.0 uv units).
pub fn leaf_size(radius: f32, max_depth: u8) -> f32 {
    radius * 0.785 * 2.0 / (1u32 << max_depth) as f32
}

/// Default planet radius in world units (1 unit = 1 m). Per-planet vertex math
/// stays f32, so radii live under ~100 km; big_space handles the distances
/// *between* planets.
pub const PLANET_RADIUS: f32 = 10_000.0;

/// Maximum terrain displacement above sea level, in world units.
pub const HEIGHT_AMP: f32 = 250.0;

/// Quadtree leaf depth. Node at depth d covers `2/2^d` of a cube face's [-1,1] uv.
/// lod = MAX_DEPTH - depth, so leaves are lod 0.
pub const MAX_DEPTH: u8 = 10;

/// CDLOD visibility range of lod 0 (leaves). range(l) = LOD0_RANGE * 2^l.
/// Must stay comfortably above the leaf node diameter (~22 m at R=10km, depth 10)
/// or adjacent selected nodes can differ by more than one lod (cracks).
pub const LOD0_RANGE: f32 = 60.0;

/// Quads per side of the shared grid mesh (129x129 vertices).
pub const GRID_QUADS: u32 = 128;

/// Tile heightmap side in texels: 129 vertex texels + 1 border texel on each side
/// used by fragment-shader finite differences (one high-side texel spare).
pub const TILE_TEXELS: u32 = 132;

/// Heightmap atlas layer count = max resident tiles (LRU-evicted). Must stay
/// comfortably above the near-surface working set (selected nodes + warming
/// prefetched children, worst near cube-face edges where two faces refine at
/// once) or refinement livelocks on eviction churn. Needs the matching
/// `max_texture_array_layers` limit bump in main.rs (wgpu default is 256).
pub const ATLAS_LAYERS: u32 = 2048;

/// Pool of renderable node entities == slots in the per-node storage buffer.
/// Selection drops nodes beyond this (counted in the overlay as "drop", which
/// must stay 0 — drops are visible holes). Underwater scenes are the peak:
/// seabed + surface refine together to ~830 nodes, so 512 breaks the ocean.
pub const MAX_VISIBLE: usize = 1024;

/// Tiles of quadtree depth <= PIN_DEPTH (6 + 24 + 96 = 126 tiles) are baked at
/// startup and never evicted, so every point on the planet always has a
/// resident coarse ancestor — zooms/teleports show instant coarse terrain,
/// never holes.
pub const PIN_DEPTH: u8 = 2;

/// New tile bakes requested per frame (nearest first).
pub const MAX_BAKES_PER_FRAME: usize = 16;

/// Tiles untouched for fewer than this many frames are not eviction victims
/// (except under emit pressure), preventing churn as the camera wiggles.
pub const EVICT_GRACE_FRAMES: u64 = 15;

/// Request child tiles at this multiple of the split distance, so bakes are
/// usually finished by the time refinement actually wants to recurse. Keep
/// modest: every prefetched child is a protected (untouchable) atlas layer,
/// and the touched working set must stay well under ATLAS_LAYERS or the
/// cache soft-wedges (nothing evictable -> refinement freezes).
pub const PREFETCH_MARGIN: f32 = 1.15;

/// Frames after a bake request before the tile is treated as resident.
/// Covers extract -> compute dispatch latency (not initial pipeline compilation,
/// which only affects the first moments after startup).
pub const BAKE_LATENCY_FRAMES: u64 = 4;

/// Morph zone inside a node's lod range: vertices are unmorphed below
/// MORPH_START_F * range(l) and fully morphed to the parent lattice at
/// MORPH_END_F * range(l). MORPH_END_F < 1.0 guarantees full morph strictly
/// before a coarser neighbor can begin (coarser neighbors start at range(l)).
pub const MORPH_START_F: f32 = 0.70;
pub const MORPH_END_F: f32 = 0.95;

/// Seconds over which a freshly emitted node eases from fully-morphed
/// (parent-identical) to its distance-based morph. Hides refinement-swap pops.
pub const REVEAL_SECS: f32 = 0.6;
