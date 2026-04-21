//! Spawn helpers for turning a `VehicleSpec` into Bevy entities.
//!
//! - Chassis is the root; each wheel is a top-level sibling (rapier's
//!   vehicle controller computes wheel poses in world space directly).
//! - Body **parts** (hitches, karosseries, tanks) are children of the
//!   chassis — they have a fixed local offset, so Bevy's transform
//!   propagation keeps them glued to the chassis for free.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use big_space::prelude::BigSpatialBundle;

use gearbox::{PartKind, VehicleId, VehicleSpec};

use super::{VehicleBody, VehicleWheel};

/// Target physical arc-length of one "^" stripe on the tyre (metres).
/// Using a fixed arc length rather than a fixed count means every
/// wheel, big or small, shows roughly the same-sized chevron blocks.
const TYRE_STRIPE_ARC_M: f32 = 0.40;

/// Marker for all meshes that make up the currently-dragging ghost
/// spawn preview. Despawning the tagged root with `despawn_recursive`
/// removes every child mesh as well.
#[derive(Component)]
pub struct GhostTag;

pub fn spawn_vehicle_visuals(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    id: VehicleId,
    spec: &VehicleSpec,
    big_space_root: Entity,
) -> Entity {
    let [r, g, b] = spec.chassis.color;
    let chassis_color = Color::srgb(r, g, b);

    let chassis_mesh = meshes.add(Cuboid::new(
        spec.chassis.size.x as f32,
        spec.chassis.size.y as f32,
        spec.chassis.size.z as f32,
    ));
    let chassis_mat = materials.add(StandardMaterial {
        base_color: chassis_color,
        perceptual_roughness: 0.6,
        metallic: 0.1,
        ..default()
    });
    let root = commands
        .spawn((
            Name::new(spec.name.clone()),
            BigSpatialBundle::default(),
            Mesh3d(chassis_mesh),
            MeshMaterial3d(chassis_mat),
            VehicleBody { id },
        ))
        .insert(ChildOf(big_space_root))
        .id();

    // Shared tread image — one repeat of the chevron block.  Each
    // wheel gets its own material below with a `uv_transform` that
    // tiles this image based on circumference, so the stripe size on
    // the tyre stays physically consistent regardless of wheel radius.
    let tread_tex = images.add(make_tyre_tread_texture());
    // Flat dark material for the circular tyre caps (shared across
    // every wheel) — the tread texture doesn't land on them.
    let cap_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.06, 0.06, 0.07),
        perceptual_roughness: 0.95,
        metallic: 0.0,
        ..default()
    });

    // Wheels — tracked separately, not parented (pose from controller).
    for (idx, wheel) in spec.wheels.iter().enumerate() {
        let circumference = std::f32::consts::TAU * wheel.radius;
        // Tiles-per-revolution = circumference / desired-stripe-arc.
        let uv_tile = (circumference / TYRE_STRIPE_ARC_M).max(1.0);
        let tread_mat = materials.add(StandardMaterial {
            // Dark multiplier: texture samples are multiplied by this,
            // so the overall tyre is always darker than the raw chevron
            // texture, regardless of scene lighting.
            base_color: Color::srgb(0.45, 0.45, 0.45),
            base_color_texture: Some(tread_tex.clone()),
            uv_transform: Affine2::from_scale(Vec2::new(uv_tile, 1.0)),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            ..default()
        });

        // Side (tread) mesh — cylinder without caps.
        let side_mesh = meshes.add(
            Cylinder::new(wheel.radius, wheel.width)
                .mesh()
                .resolution(32)
                .without_caps()
                .build(),
        );

        let wheel_entity = commands
            .spawn((
                Name::new(format!("{}::wheel[{}]", spec.name, idx)),
                BigSpatialBundle::default(),
                Mesh3d(side_mesh),
                MeshMaterial3d(tread_mat),
                VehicleWheel { id, index: idx },
            ))
            .insert(ChildOf(big_space_root))
            .id();

        // Two cap discs as children of the wheel entity. Circle is a
        // 2-D primitive in the XY plane (normal +Z); rotate ±90° around
        // X so the normal faces the axle direction (local +Y / -Y).
        let cap_mesh = meshes.add(
            Circle::new(wheel.radius).mesh().resolution(32).build(),
        );
        commands
            .spawn((
                Name::new(format!("{}::wheel[{}]::cap+", spec.name, idx)),
                Transform::from_xyz(0.0, wheel.width * 0.5, 0.0)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                Mesh3d(cap_mesh.clone()),
                MeshMaterial3d(cap_mat.clone()),
            ))
            .insert(ChildOf(wheel_entity));
        commands
            .spawn((
                Name::new(format!("{}::wheel[{}]::cap-", spec.name, idx)),
                Transform::from_xyz(0.0, -wheel.width * 0.5, 0.0)
                    .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                Mesh3d(cap_mesh),
                MeshMaterial3d(cap_mat.clone()),
            ))
            .insert(ChildOf(wheel_entity));
    }

    // Parts — parented to the chassis so they inherit its pose.
    for part in &spec.parts {
        let [pr, pg, pb] = part.color;
        let p_color = Color::srgb(pr, pg, pb);
        let mesh = meshes.add(Cuboid::new(
            part.size.x as f32,
            part.size.y as f32,
            part.size.z as f32,
        ));
        let mat = materials.add(StandardMaterial {
            base_color: p_color,
            perceptual_roughness: match part.kind {
                PartKind::Hitch => 0.3, // slightly glossy marker
                _ => 0.7,
            },
            metallic: match part.kind {
                PartKind::Hitch => 0.4,
                _ => 0.1,
            },
            ..default()
        });
        commands
            .spawn((
                Name::new(format!("{}::{}", spec.name, part.name)),
                Transform::from_xyz(
                    part.position.x as f32,
                    part.position.y as f32,
                    part.position.z as f32,
                ),
                Mesh3d(mesh),
                MeshMaterial3d(mat),
            ))
            .insert(ChildOf(root));
    }

    root
}

/// Pick a spawn Y that guarantees the chassis starts a bit above the
/// ground (wheels hang down, settle on contact). Dynamic so we don't
/// hard-code 1.4 for every preset regardless of size.
pub fn spawn_height_for(spec: &VehicleSpec) -> f64 {
    // ~0.8 m of clearance under the chassis bottom — enough for
    // wheels (rear tractor wheel radius ~1 m and stroke ~0.35 m) to
    // hang in air at rest, regardless of preset.
    spec.chassis.size.y * 0.5 + 0.8
}

/// Procedural tyre-tread texture — **exactly one chevron period**, so
/// the material can tile it `circumference / TYRE_STRIPE_ARC_M` times
/// around the wheel via `uv_transform`.  Sampler set to `Repeat` on
/// the U axis so tiling works.
///
/// UV convention: `u` wraps around the wheel; `v` runs along the axle.
/// Apex of the "^" sits on the tyre centre line (`v = 0.5`).
fn make_tyre_tread_texture() -> Image {
    const W: u32 = 64;
    const H: u32 = 64;
    const CHEVRON_SLOPE: f32 = 0.55;   // sharper V — visible bend from apex to edges
    const STRIPE_FRACTION: f32 = 0.40; // chunky tread block

    let base:  [u8; 4] = [18, 18, 20, 255];
    let tread: [u8; 4] = [70, 70, 72, 255];

    let mut data = Vec::with_capacity((W * H * 4) as usize);
    for vp in 0..H {
        let fv = vp as f32 / H as f32;
        let dv = (fv - 0.5).abs();
        for up in 0..W {
            let fu = up as f32 / W as f32;
            let u_shifted = (fu + dv * CHEVRON_SLOPE).rem_euclid(1.0);
            let c = if u_shifted < STRIPE_FRACTION { tread } else { base };
            data.extend_from_slice(&c);
        }
    }
    let mut img = Image::new(
        Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    // Repeat on U so `uv_transform` tiling actually tiles; clamp on V
    // so the chevron apex stays dead-centre on the tyre.
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::default()
    });
    img
}

/// Non-physics translucent preview of a vehicle — same meshes/parts
/// as the real one, but with alpha-blended materials and no
/// `VehicleBody` / rapier tagging. Used for the "drag-to-place" UX:
/// the ghost follows the cursor until the user commits with a click.
pub fn spawn_vehicle_ghost(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    spec: &VehicleSpec,
    big_space_root: Entity,
) -> Entity {
    let alpha = 0.45;
    let [r, g, b] = spec.chassis.color;
    let chassis_color = Color::srgba(r, g, b, alpha);
    let wheel_color   = Color::srgba(0.18, 0.18, 0.18, alpha);
    let tread_tex     = images.add(make_tyre_tread_texture());

    let chassis_mesh = meshes.add(Cuboid::new(
        spec.chassis.size.x as f32,
        spec.chassis.size.y as f32,
        spec.chassis.size.z as f32,
    ));
    let chassis_mat = materials.add(StandardMaterial {
        base_color: chassis_color,
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.7,
        metallic: 0.1,
        ..default()
    });
    let root = commands
        .spawn((
            Name::new(format!("{}-ghost", spec.name)),
            BigSpatialBundle::default(),
            Mesh3d(chassis_mesh),
            MeshMaterial3d(chassis_mat),
            GhostTag,
        ))
        .insert(ChildOf(big_space_root))
        .id();

    // Wheels as children of the ghost root — at rest (suspension
    // fully extended) so the silhouette reads as a settled vehicle.
    // Cylinder default axis is +Y; rotate 90° around Z so the axle
    // lies along X.
    // Shared cap material for the ghost preview (translucent).
    let ghost_cap_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.06, 0.06, 0.07, alpha),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    for wheel in &spec.wheels {
        let circumference = std::f32::consts::TAU * wheel.radius;
        let uv_tile = (circumference / TYRE_STRIPE_ARC_M).max(1.0);
        let mat = materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 1.0, 1.0, alpha),
            base_color_texture: Some(tread_tex.clone()),
            uv_transform: Affine2::from_scale(Vec2::new(uv_tile, 1.0)),
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        let side_mesh = meshes.add(
            Cylinder::new(wheel.radius, wheel.width)
                .mesh()
                .resolution(32)
                .without_caps()
                .build(),
        );
        let cap_mesh = meshes.add(
            Circle::new(wheel.radius).mesh().resolution(32).build(),
        );
        let wy = (wheel.chassis_connection.y - wheel.suspension_rest_length as f64) as f32;
        let wheel_parent = commands
            .spawn((
                Transform::from_xyz(
                    wheel.chassis_connection.x as f32,
                    wy,
                    wheel.chassis_connection.z as f32,
                )
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                Mesh3d(side_mesh),
                MeshMaterial3d(mat),
                GhostTag,
            ))
            .insert(ChildOf(root))
            .id();
        commands
            .spawn((
                Transform::from_xyz(0.0, wheel.width * 0.5, 0.0)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                Mesh3d(cap_mesh.clone()),
                MeshMaterial3d(ghost_cap_mat.clone()),
                GhostTag,
            ))
            .insert(ChildOf(wheel_parent));
        commands
            .spawn((
                Transform::from_xyz(0.0, -wheel.width * 0.5, 0.0)
                    .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                Mesh3d(cap_mesh),
                MeshMaterial3d(ghost_cap_mat.clone()),
                GhostTag,
            ))
            .insert(ChildOf(wheel_parent));
    }

    // Body parts — children of the chassis root with local offsets.
    for part in &spec.parts {
        let [pr, pg, pb] = part.color;
        let p_color = Color::srgba(pr, pg, pb, alpha);
        let mesh = meshes.add(Cuboid::new(
            part.size.x as f32,
            part.size.y as f32,
            part.size.z as f32,
        ));
        let mat = materials.add(StandardMaterial {
            base_color: p_color,
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: match part.kind {
                PartKind::Hitch => 0.3,
                _ => 0.7,
            },
            metallic: match part.kind {
                PartKind::Hitch => 0.4,
                _ => 0.1,
            },
            ..default()
        });
        commands
            .spawn((
                Transform::from_xyz(
                    part.position.x as f32,
                    part.position.y as f32,
                    part.position.z as f32,
                ),
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                GhostTag,
            ))
            .insert(ChildOf(root));
    }

    root
}
