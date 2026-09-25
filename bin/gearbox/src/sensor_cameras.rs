//! Bevy-rendered colour for camera sensor links (`gearbox:sensor:render` =
//! `optimized` or `full`).
//!
//! Each such link gets a render-to-texture camera parented to its prim. The
//! camera stays inactive until the link's rig issues a sample; that frame it
//! renders once and its image is read back, then the colour is published on
//! `/machines/<id>/sensors/<link>` with the sample number and simulated time
//! of the link's Molla depth, so both channels of one sample line up.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use gearbox_api::{CAMERA_COLOR, GearboxBus};

use crate::links::{CameraRender, SensorSpec};
use crate::sensors::camera_chunks;

pub struct SensorCamerasPlugin;

impl Plugin for SensorCamerasPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SensorRenderQueue>()
            .init_resource::<SensorCameras>()
            .add_systems(
                PostUpdate,
                (drive_sensor_cameras, arm_spawned_cameras)
                    .chain()
                    .after(crate::sensors::run_machine_sensors),
            );
    }
}

/// A camera link, by the machine agent's bus key and the link name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CameraKey {
    pub bus_key: String,
    pub link: String,
}

/// One colour image to render this frame.
pub struct RenderJob {
    pub key: CameraKey,
    pub link_index: u32,
    pub spec: SensorSpec,
    pub entity: Option<Entity>,
    pub sample: u64,
    pub sim_time: f64,
    pub stamp_ms: u32,
}

/// Render jobs issued by the rigs this frame, and every Bevy camera link
/// that still exists.
#[derive(Resource, Default)]
pub struct SensorRenderQueue {
    pub jobs: Vec<RenderJob>,
    pub live: HashSet<CameraKey>,
}

/// Render-to-texture cameras by link, with frame and cost counters.
#[derive(Resource, Default)]
pub struct SensorCameras {
    cameras: HashMap<CameraKey, Entity>,
    pub stats: CameraStats,
    last_report: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CameraStats {
    pub renders: u64,
    pub published: u64,
    pub undelivered: u64,
    pub readback_bytes: u64,
}

/// A sensor link's render-to-texture camera.
#[derive(Component)]
pub struct SensorCamera {
    key: CameraKey,
    link_index: u32,
    spec: SensorSpec,
    image: Handle<Image>,
    parent: Entity,
    /// Samples rendered and awaiting their readback, oldest first.
    pending: VecDeque<(u64, f64, u32)>,
    /// Rendered this frame; switched off again next frame.
    armed: bool,
}

/// Rotation from the link frame (+X forward, +Z up) to Bevy's camera axes
/// (-Z forward, +Y up).
fn camera_in_link() -> Transform {
    Transform::IDENTITY.looking_to(Vec3::X, Vec3::Z)
}

fn target_image(spec: &SensorSpec) -> Image {
    let mut image = Image::new_target_texture(
        spec.width,
        spec.height,
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image
}

/// Background for rays that reach the sky: sensor cameras do not draw the
/// viewer's cloud skybox, which is rendered from the viewer's own position.
const SKY: Color = Color::srgb(0.53, 0.70, 0.90);

fn spawn_camera(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    weather: Option<&bevy_weather::WeatherSettings>,
    job: &RenderJob,
    parent: Entity,
    order: isize,
) -> Entity {
    let image = images.add(target_image(&job.spec));
    let projection = Projection::Perspective(PerspectiveProjection {
        fov: job.spec.vfov,
        aspect_ratio: job.spec.width as f32 / job.spec.height as f32,
        near: 0.05,
        far: job.spec.range_m.max(1.0),
        ..default()
    });
    let mut camera = commands.spawn((
        Name::new(format!("sensor camera {}", job.key.link)),
        Camera3d::default(),
        Camera {
            is_active: false,
            order,
            clear_color: ClearColorConfig::Custom(SKY),
            ..default()
        },
        RenderTarget::Image(image.clone().into()),
        projection,
        camera_in_link(),
        bevy_weather::WeatherOptOut,
        SensorCamera {
            key: job.key.clone(),
            link_index: job.link_index,
            spec: job.spec,
            image,
            parent,
            pending: VecDeque::new(),
            armed: false,
        },
        ChildOf(parent),
    ));
    let lean = job.spec.render == CameraRender::Optimized;
    match weather {
        Some(weather) => bevy_weather::camera_look(&mut camera, weather, lean),
        None => {
            camera.insert(Msaa::Off);
        }
    }
    if lean {
        camera.insert(gearbox_fields::NoVegetation);
    }
    camera.observe(publish_readback);
    camera.id()
}

fn drive_sensor_cameras(
    mut commands: Commands,
    mut queue: ResMut<SensorRenderQueue>,
    mut state: ResMut<SensorCameras>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<(Entity, &mut Camera, &mut SensorCamera)>,
    weather: Option<Res<bevy_weather::WeatherSettings>>,
    time: Res<Time>,
) {
    let state = &mut *state;
    // Hosts may switch every camera on before each update, so each frame
    // starts with all sensor cameras off; only due samples render.
    for (entity, mut camera, mut sensor) in &mut cameras {
        camera.is_active = false;
        if sensor.armed {
            sensor.armed = false;
            commands.entity(entity).remove::<Readback>();
        }
    }
    let live = std::mem::take(&mut queue.live);
    state.cameras.retain(|key, entity| {
        let keep = live.contains(key);
        if !keep {
            commands.entity(*entity).despawn();
        }
        keep
    });
    for job in queue.jobs.drain(..) {
        let Some(parent) = job.entity else { continue };
        let existing = state.cameras.get(&job.key).copied();
        let reusable = existing.and_then(|entity| cameras.get_mut(entity).ok()).filter(
            |(_, _, sensor)| sensor.spec == job.spec && sensor.parent == parent,
        );
        let Some((_, mut camera, mut sensor)) = reusable else {
            if let Some(stale) = existing {
                commands.entity(stale).despawn();
            }
            let order = -1000 - state.cameras.len() as isize;
            let entity = spawn_camera(
                &mut commands,
                &mut images,
                weather.as_deref(),
                &job,
                parent,
                order,
            );
            state.cameras.insert(job.key.clone(), entity);
            commands.entity(entity).insert(Pending::from(&job));
            continue;
        };
        camera.is_active = true;
        sensor.armed = true;
        sensor.pending.push_back((job.sample, job.sim_time, job.stamp_ms));
        let image = sensor.image.clone();
        let entity = state.cameras[&job.key];
        commands.entity(entity).insert(Readback::texture(image));
        state.stats.renders += 1;
    }
    queue.live = live;
    let now = time.elapsed_secs_f64();
    if now - state.last_report >= 10.0 {
        let s = state.stats;
        if s.renders > 0 {
            info!(
                "gearbox-sensors: bevy cameras: {} renders, {} published, {} undelivered, {:.1} KB read back per render in the last {:.0} s",
                s.renders,
                s.published,
                s.undelivered,
                s.readback_bytes as f64 / 1024.0 / s.renders as f64,
                now - state.last_report
            );
        }
        state.stats = CameraStats::default();
        state.last_report = now;
    }
}

/// The first sample of a freshly spawned camera, rendered once it exists.
#[derive(Component)]
struct Pending {
    sample: u64,
    sim_time: f64,
    stamp_ms: u32,
}

impl From<&RenderJob> for Pending {
    fn from(job: &RenderJob) -> Self {
        Self {
            sample: job.sample,
            sim_time: job.sim_time,
            stamp_ms: job.stamp_ms,
        }
    }
}

/// Arms cameras spawned on an earlier frame, once transform propagation has
/// placed them, for the sample that created them.
fn arm_spawned_cameras(
    mut commands: Commands,
    mut cameras: Query<(Entity, &mut Camera, &mut SensorCamera, Ref<Pending>)>,
    mut state: ResMut<SensorCameras>,
) {
    for (entity, mut camera, mut sensor, pending) in &mut cameras {
        if pending.is_added() {
            continue;
        }
        camera.is_active = true;
        sensor.armed = true;
        sensor
            .pending
            .push_back((pending.sample, pending.sim_time, pending.stamp_ms));
        commands
            .entity(entity)
            .remove::<Pending>()
            .insert(Readback::texture(sensor.image.clone()));
        state.stats.renders += 1;
    }
}

/// Strips the 256-byte row padding of a texture readback.
fn unpad_rows(data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let row = width as usize * 4;
    let stride = row.div_ceil(256) * 256;
    if data.len() < stride * (height as usize - 1) + row {
        return None;
    }
    let mut pixels = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        pixels.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    Some(pixels)
}

fn publish_readback(
    event: On<ReadbackComplete>,
    mut cameras: Query<&mut SensorCamera>,
    bus: Option<ResMut<GearboxBus>>,
    mut state: ResMut<SensorCameras>,
) {
    let entity = event.entity;
    let Ok(mut sensor) = cameras.get_mut(entity) else {
        return;
    };
    let Some((sample, sim_time, stamp_ms)) = sensor.pending.pop_front() else {
        return;
    };
    state.stats.readback_bytes += event.data.len() as u64;
    let spec = sensor.spec;
    let Some(pixels) = unpad_rows(&event.data, spec.width, spec.height) else {
        warn!(
            "gearbox-sensors: camera `{}` read back {} bytes for {}x{}",
            sensor.key.link,
            event.data.len(),
            spec.width,
            spec.height
        );
        return;
    };
    let Some(mut bus) = bus else { return };
    let Some(agent) = bus.machines.get_mut(&sensor.key.bus_key) else {
        return;
    };
    let link = sensor.key.link.clone();
    let delivered = camera_chunks(
        &link,
        sensor.link_index,
        &spec,
        sim_time,
        stamp_ms,
        sample,
        CAMERA_COLOR,
        &pixels,
        &mut |wire| agent.publish_sensor_camera(&link, wire),
    );
    if delivered {
        state.stats.published += 1;
    } else {
        state.stats.undelivered += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readback_rows_drop_their_alignment_padding() {
        let (width, height) = (3u32, 2u32);
        let mut data = vec![0u8; 256 + 12];
        data[..12].copy_from_slice(&[1; 12]);
        data[256..268].copy_from_slice(&[2; 12]);
        let pixels = unpad_rows(&data, width, height).unwrap();
        assert_eq!(pixels.len(), 24);
        assert!(pixels[..12].iter().all(|b| *b == 1));
        assert!(pixels[12..].iter().all(|b| *b == 2));
        assert!(unpad_rows(&data[..200], width, height).is_none());
        let exact = vec![7u8; 64 * 4 * 2];
        assert_eq!(unpad_rows(&exact, 64, 2).unwrap(), exact);
    }

    #[test]
    fn link_forward_and_up_become_camera_forward_and_up() {
        let t = camera_in_link();
        assert!((t.rotation * Vec3::NEG_Z - Vec3::X).length() < 1e-6);
        assert!((t.rotation * Vec3::Y - Vec3::Z).length() < 1e-6);
    }
}
