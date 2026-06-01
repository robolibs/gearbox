//! Procedural cloud shell — a translucent sphere slightly larger than
//! the planet, textured with a tiling noise so it reads as both a
//! ground-level overcast AND orbital cloud bands.
//!
//! Architecture notes:
//! - Mesh is a regular UV sphere at `radius = planet.radius + alt`.
//!   Bevy's `Sphere::mesh().uv()` gives us longitude/latitude UVs for
//!   free — a 2-D noise texture tiled once wraps around cleanly.
//! - `cull_mode: None` (via `double_sided`) so the camera sees clouds
//!   from below (inside the shell) AND from above (outside the shell).
//! - Tagged `NotShadowCaster` so Bevy's cascaded shadow maps don't try
//!   to fit a planet-sized caster and degenerate.
//! - Sits at the same world position as the planet so the two
//!   surfaces share an origin.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Default altitude of the cloud deck above the planet surface,
/// in metres. Real cumulus sits around 1–3 km; 4 km gives a touch
/// of visible separation from the terrain when viewed from orbit.
/// Use this when calling [`spawn_cloud_shell`] if you don't need
/// to override the altitude.
pub const DEFAULT_CLOUD_ALTITUDE_M: f64 = 4_000.0;

/// Procedural cloud texture resolution (longitude × latitude).
const CLOUD_TEX_W: u32 = 1024;
const CLOUD_TEX_H: u32 = 512;

/// Spawn the cloud shell. `cloud_altitude_m` is the height of the
/// shell above the planet surface — pass [`DEFAULT_CLOUD_ALTITUDE_M`]
/// for a stock 4 km cumulus deck.
pub fn spawn_cloud_shell(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    planet_radius: f64,
    cloud_altitude_m: f64,
) {
    let shell_radius = planet_radius + cloud_altitude_m;

    // Mesh: same UV-sphere layout as the planet so texture wrapping
    // is identical. A bit coarser (256×128) is plenty because the
    // shell is never the focus of attention.
    let mesh = meshes.add(Sphere::new(shell_radius as f32).mesh().uv(256, 128));

    let cloud_tex = images.add(make_cloud_texture());
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.92),
        base_color_texture: Some(cloud_tex),
        alpha_mode: AlphaMode::Blend,
        // Clouds receive sun shading but shouldn't cast sharp shadows;
        // keep unlit = false so they pick up the directional light.
        unlit: false,
        double_sided: true,
        cull_mode: None,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        ..default()
    });

    // Same world position as the planet: centered at (0, -R, 0).
    commands.spawn((
        Name::new("CloudShell"),
        Transform::from_xyz(0.0, -planet_radius as f32, 0.0),
        Mesh3d(mesh),
        MeshMaterial3d(material),
        NotShadowCaster,
    ));
}

/// Build a tileable cloud texture. RGB is white (cloud colour);
/// alpha carries the cloud coverage mask, so where the texture is
/// transparent you see right through the shell to the sky.
fn make_cloud_texture() -> Image {
    let w = CLOUD_TEX_W;
    let h = CLOUD_TEX_H;
    let mut data = Vec::with_capacity((w * h * 4) as usize);

    // Coverage: 0 = no clouds, 1 = solid. Cumulus-ish breaks at ~0.55.
    let coverage: f32 = 0.55;
    // Max alpha where the cloud is densest.
    let max_alpha: f32 = 0.92;

    for y in 0..h {
        for x in 0..w {
            let u = x as f32 / w as f32; // 0..1 longitude
            let v = y as f32 / h as f32; // 0..1 latitude
            let n = fbm_tileable(u, v);
            // Threshold: below coverage cutoff the cloud thins to zero,
            // above it ramps up to max_alpha. Smoothstep for a soft
            // edge rather than a hard mask.
            let t = ((n - (1.0 - coverage)) / coverage).clamp(0.0, 1.0);
            let a = (t * t * (3.0 - 2.0 * t)) * max_alpha;
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }

    Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

/// Five-octave tileable noise in 0..1. Sums sin/cos with integer
/// frequencies so the horizontal seam (u = 0 = 1) and the pole row
/// meet cleanly — prevents the visible longitude seam that a plain
/// random noise would give you.
fn fbm_tileable(u: f32, v: f32) -> f32 {
    use std::f32::consts::TAU;
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 3.0;
    let mut phase = 0.0;
    for _ in 0..5 {
        let fu = u * TAU * freq;
        let fv = v * std::f32::consts::PI * freq;
        // Two-axis sin product — tileable across longitude because
        // `freq` is an integer, and the amplitude fades toward the
        // poles via `sin(fv)`.
        sum += amp * ((fu + phase).sin() * fv.sin());
        amp *= 0.55;
        freq *= 2.07;
        phase += 1.73;
    }
    // Map roughly [-1, 1] → [0, 1], a shade biased so there's less
    // cloud near the equator and more at mid-latitudes (matches how
    // real trade-wind cumulus concentrates).
    (sum * 0.5 + 0.5).clamp(0.0, 1.0)
}
