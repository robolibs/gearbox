//! The persistent world: 6 371 km planet sphere, camera-following
//! shadow patch (so vehicle shadows actually land somewhere with
//! enough triangles to render cleanly), translucent cloud shell at
//! 4 km altitude, atmospheric DistanceFog, sun with a single tight
//! shadow cascade, ChaseCamera + ground-grid configuration, static
//! cuboid ground in `usd_bevy::physics::PhysicsWorld`. Mirrors the
//! `gearbox_old` setup so loaded USDs sit in the same world the
//! simulator was tuned for.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, NotShadowCaster};
use bevy::math::DVec3;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_glacial::{ChaseCamera, GroundGrid};
use rapier3d::prelude::ColliderBuilder;
use usd_bevy::physics::PhysicsWorld;

/// Earth-radius planet sphere. The simulator was tuned for this
/// radius — vehicle wheel friction, camera fog distances, cloud
/// altitude, and shadow cascades all assume ~6 371 km.
const PLANET_RADIUS_M: f32 = 6_371_000.0;
/// Cloud deck height above the planet surface. ~4 km gives visible
/// separation from the terrain when zoomed out.
const CLOUD_ALTITUDE_M: f64 = 4_000.0;
/// Half-side of the camera-following shadow patch (m).
const SHADOW_PATCH_HALF: f32 = 300.0;
/// Triangle density of the shadow patch. 200×200 cells = 80 k tris,
/// fine enough to receive crisp directional shadows.
const SHADOW_PATCH_TESS: u32 = 200;

/// Tag for the spherical-cap mesh that follows the camera. Same
/// material as the planet, curved to match it exactly — gives
/// vehicle shadows a high-density local surface to land on without
/// having to tessellate the whole 6 371 km ball.
#[derive(Component)]
pub struct ShadowPatch;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.55, 0.70, 0.86)))
            // 8 k shadow map (4× Bevy's 2048 default). One cascade has
            // to cover the whole ~100 m vehicle neighbourhood, so the
            // extra texels go directly into shadow sharpness.
            .insert_resource(DirectionalLightShadowMap { size: 8192 })
            .add_systems(Startup, (spawn_world, spawn_physics_ground))
            .add_systems(Update, follow_camera_shadow_patch);
    }
}

fn spawn_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let radius = PLANET_RADIUS_M;
    let radius_f64 = PLANET_RADIUS_M as f64;

    // ── Planet sphere ────────────────────────────────────────────────
    // Warm sandy / tan ground colour. Higher UV resolution than a
    // toy sphere because this fills the whole horizon — at 1024×512
    // the equator triangles are still ~40 km wide, which is why the
    // ShadowPatch below carries the local detail.
    let planet_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.62, 0.48, 0.33),
        perceptual_roughness: 0.95,
        ..default()
    });
    let planet_mesh = meshes.add(Sphere::new(radius).mesh().uv(1024, 512));
    commands.spawn((
        Name::new("Planet"),
        Transform::from_xyz(0.0, -radius, 0.0),
        Mesh3d(planet_mesh),
        MeshMaterial3d(planet_mat.clone()),
        NotShadowCaster,
        bevy::light::NotShadowReceiver,
    ));

    // ── Shadow patch (camera-follows) ────────────────────────────────
    let shadow_patch_mesh = meshes.add(spherical_cap_mesh(
        radius,
        SHADOW_PATCH_HALF,
        SHADOW_PATCH_TESS,
    ));
    commands.spawn((
        Name::new("ShadowPatch"),
        ShadowPatch,
        Transform::default(),
        Mesh3d(shadow_patch_mesh),
        MeshMaterial3d(planet_mat),
        NotShadowCaster,
    ));

    // ── Ground grid: dim warm tone, soft alpha so it reads as a hint
    // not ink stains on the tan ground. The bevy_glacial
    // GroundGridPlugin will pick this resource up.
    commands.insert_resource(GroundGrid {
        color: Color::srgba(80.0 / 255.0, 70.0 / 255.0, 70.0 / 255.0, 0.26),
        ..GroundGrid::default()
    });

    // ── Cloud shell ──────────────────────────────────────────────────
    spawn_cloud_shell(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        radius_f64,
    );

    // ── Sun + tight cascade ──────────────────────────────────────────
    // Single 100 m cascade so all texels land on the vehicle
    // neighbourhood. Steep angle for a clear horizontal direction.
    let sun_shadow = CascadeShadowConfigBuilder {
        num_cascades: 1,
        minimum_distance: 0.1,
        maximum_distance: 100.0,
        first_cascade_far_bound: 100.0,
        overlap_proportion: 0.0,
    }
    .build();
    commands.spawn((
        Name::new("Sun"),
        Transform::from_xyz(5.0, 50.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        DirectionalLight {
            illuminance: 10_000.0,
            shadows_enabled: true,
            ..default()
        },
        sun_shadow,
    ));

    // ── Atmospheric fog ──────────────────────────────────────────────
    let fog = DistanceFog {
        color: Color::srgb(0.55, 0.70, 0.86),
        falloff: FogFalloff::Atmospheric {
            extinction: Vec3::new(0.00008, 0.00012, 0.00020),
            inscattering: Vec3::new(0.00010, 0.00015, 0.00025),
        },
        ..default()
    };

    // ── Camera ──────────────────────────────────────────────────────
    commands.spawn((
        Name::new("Camera"),
        Camera3d::default(),
        Transform::from_xyz(0.0, 8.0, -15.0).looking_at(Vec3::ZERO, Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            near: 0.1,
            far: radius * 2.5,
            ..default()
        }),
        fog,
        AmbientLight {
            color: Color::WHITE,
            brightness: 120.0,
            ..default()
        },
        ChaseCamera {
            focus: Vec3::new(0.0, 0.5, 0.0),
            distance: 14.0,
            elevation: 25_f32.to_radians(),
            max_distance: radius * 3.0,
            ..default()
        },
        bevy_glacial::AxisGizmo::default(),
        bevy_glacial::GizmoCamera,
    ));
}

/// Static cuboid in `PhysicsWorld` so loaded USD bodies have
/// something to land on. Top surface flush with `y = 0` to line
/// up visually with the planet tangent plane / shadow patch.
fn spawn_physics_ground(mut world: ResMut<PhysicsWorld>) {
    let ground = ColliderBuilder::cuboid(2_000.0, 0.5, 2_000.0)
        .translation(DVec3::new(0.0, -0.5, 0.0))
        .friction(1.0)
        .build();
    world.colliders.insert(ground);
}

/// Re-centre the `ShadowPatch` under the chase-camera focus every
/// frame so vehicle shadows always land on it, no matter where in
/// the world the camera is looking.
fn follow_camera_shadow_patch(
    cameras: Query<&ChaseCamera>,
    mut patches: Query<&mut Transform, With<ShadowPatch>>,
) {
    let Ok(cam) = cameras.single() else {
        return;
    };
    for mut tr in patches.iter_mut() {
        tr.translation.x = cam.focus.x;
        tr.translation.y = 0.0;
        tr.translation.z = cam.focus.z;
    }
}

/// Spherical-cap mesh: an `(n+1)²` grid of vertices over the
/// `[-half, half]²` square, projected onto the surface of a sphere
/// of radius `radius` centred at `(0, -radius, 0)` so the patch's
/// local origin sits on the tangent point at `y = 0` and its
/// curvature matches the planet exactly.
fn spherical_cap_mesh(radius: f32, half_size: f32, n: u32) -> Mesh {
    let n = n.max(1) as i32;
    let step = (2.0 * half_size) / n as f32;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
    let mut indices: Vec<u32> = Vec::with_capacity((n * n * 6) as usize);

    for i in 0..=n {
        for j in 0..=n {
            let x = -half_size + i as f32 * step;
            let z = -half_size + j as f32 * step;
            let dist2 = x * x + z * z;
            let y = (radius * radius - dist2).sqrt() - radius;
            positions.push([x, y, z]);
            normals.push([x / radius, (y + radius) / radius, z / radius]);
            uvs.push([(i as f32) / (n as f32), (j as f32) / (n as f32)]);
        }
    }
    let row = (n + 1) as u32;
    for i in 0..n as u32 {
        for j in 0..n as u32 {
            let a = i * row + j;
            let b = a + 1;
            let c = a + row;
            let d = c + 1;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Translucent cloud shell — a UV sphere at `planet_radius + 4 km`,
/// double-sided so it reads from inside (ground level overcast) and
/// outside (orbital cloud bands), `NotShadowCaster` so it doesn't
/// blow up the directional cascade.
fn spawn_cloud_shell(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    planet_radius: f64,
) {
    let shell_radius = planet_radius + CLOUD_ALTITUDE_M;
    let mesh = meshes.add(Sphere::new(shell_radius as f32).mesh().uv(256, 128));
    let cloud_tex = images.add(make_cloud_texture());
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.92),
        base_color_texture: Some(cloud_tex),
        alpha_mode: AlphaMode::Blend,
        unlit: false,
        double_sided: true,
        cull_mode: None,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        ..default()
    });
    commands.spawn((
        Name::new("CloudShell"),
        Transform::from_xyz(0.0, -planet_radius as f32, 0.0),
        Mesh3d(mesh),
        MeshMaterial3d(material),
        NotShadowCaster,
    ));
}

fn make_cloud_texture() -> Image {
    const W: u32 = 1024;
    const H: u32 = 512;
    let mut data = Vec::with_capacity((W * H * 4) as usize);
    let coverage: f32 = 0.55;
    let max_alpha: f32 = 0.92;
    for y in 0..H {
        for x in 0..W {
            let u = x as f32 / W as f32;
            let v = y as f32 / H as f32;
            let n = fbm_tileable(u, v);
            let t = ((n - (1.0 - coverage)) / coverage).clamp(0.0, 1.0);
            let a = (t * t * (3.0 - 2.0 * t)) * max_alpha;
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

fn fbm_tileable(u: f32, v: f32) -> f32 {
    use std::f32::consts::TAU;
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq: f32 = 3.0;
    let mut phase = 0.0;
    for _ in 0..5 {
        let fu = u * TAU * freq;
        let fv = v * std::f32::consts::PI * freq;
        sum += amp * ((fu + phase).sin() * fv.sin());
        amp *= 0.55;
        freq *= 2.07;
        phase += 1.73;
    }
    (sum * 0.5 + 0.5).clamp(0.0, 1.0)
}
