//! Bevy-rendered colour for camera sensor links (`gearbox:sensor:render` =
//! `optimized` or `full`).
//!
//! Each such link gets a render-to-texture camera parented to its prim. The
//! camera stays inactive until the link's rig issues a sample; that frame it
//! renders once and its image is read back, then the colour is published on
//! `/machines/<id>/sensors/<link>` with the sample number and simulated time
//! of the link's Molla depth, so both channels of one sample line up.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::camera::{CameraUpdateSystems, RenderTarget};
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
                    .after(crate::sensors::run_machine_sensors)
                    .before(CameraUpdateSystems)
                    .before(bevy::light::SimulationLightSystems::UpdateDirectionalLightCascades),
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
    /// The newest sample each camera still owes, by the frame it was first owed.
    waiting: HashMap<CameraKey, (u64, RenderJob)>,
    frame: u64,
    pub stats: CameraStats,
    last_report: f64,
}

/// Most sensor cameras rendered in one frame: each is a whole view of the
/// world, so a slow frame owing many samples spreads them over the next.
const RENDERS_PER_FRAME: usize = 2;

#[derive(Clone, Copy, Debug, Default)]
pub struct CameraStats {
    pub renders: u64,
    pub published: u64,
    pub undelivered: u64,
    pub readback_bytes: u64,
    /// Samples a newer one replaced before they rendered.
    pub superseded: u64,
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

/// Background fallback when no weather plugin maintains `ClearColor`.
const SKY: Color = Color::srgb(0.53, 0.70, 0.90);
/// Maximum clustered point/spot light reach for `full` sensor cameras (m).
const CLUSTERED_REACH_M: f32 = 120.0;

fn cluster_config_for(spec: &SensorSpec, has_lamps: bool) -> bevy::light::cluster::ClusterConfig {
    use bevy::light::cluster::{ClusterConfig, ClusterFarZMode, ClusterZConfig};
    if spec.render == CameraRender::Full && has_lamps {
        ClusterConfig::FixedZ {
            total: 2048,
            z_slices: 16,
            z_config: ClusterZConfig {
                first_slice_depth: 1.0,
                far_z_mode: ClusterFarZMode::Constant(spec.range_m.clamp(1.0, CLUSTERED_REACH_M)),
            },
            dynamic_resizing: true,
        }
    } else {
        ClusterConfig::None
    }
}

fn spawn_camera(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    weather: Option<&bevy_weather::WeatherSettings>,
    has_lamps: bool,
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
    let clear_color = if weather.is_some() {
        ClearColorConfig::Default
    } else {
        ClearColorConfig::Custom(SKY)
    };
    let mut camera = commands.spawn((
        Name::new(format!("sensor camera {}", job.key.link)),
        Camera3d::default(),
        bevy::pbr::ScreenSpaceTransmission::default(),
        Camera {
            is_active: false,
            order,
            clear_color,
            ..default()
        },
        RenderTarget::Image(image.clone().into()),
        projection,
        camera_in_link(),
        cluster_config_for(&job.spec, has_lamps),
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

#[allow(clippy::too_many_arguments)]
fn drive_sensor_cameras(
    mut commands: Commands,
    mut queue: ResMut<SensorRenderQueue>,
    mut state: ResMut<SensorCameras>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<(
        Entity,
        &mut Camera,
        &mut SensorCamera,
        Option<&mut bevy::light::cluster::ClusterConfig>,
    )>,
    lights: Query<(), Or<(With<PointLight>, With<SpotLight>)>>,
    weather: Option<Res<bevy_weather::WeatherSettings>>,
    time: Res<Time>,
) {
    let state = &mut *state;
    let has_lamps = !lights.is_empty();
    // Hosts may switch every camera on before each update, so each frame
    // starts with all sensor cameras off; only due samples render.
    for (entity, mut camera, mut sensor, cluster) in &mut cameras {
        camera.is_active = false;
        if let Some(mut cluster) = cluster {
            let want = cluster_config_for(&sensor.spec, has_lamps);
            let is_clustered = !matches!(*cluster, bevy::light::cluster::ClusterConfig::None);
            let want_clustered = !matches!(want, bevy::light::cluster::ClusterConfig::None);
            if is_clustered != want_clustered {
                *cluster = want;
            }
        }
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
    state.frame += 1;
    let jobs = std::mem::take(&mut queue.jobs).into_iter().map(|job| (job.key.clone(), job));
    let jobs = take_due(&mut state.waiting, jobs, &live, state.frame, RENDERS_PER_FRAME, &mut state.stats.superseded);
    for job in jobs {
        let Some(parent) = job.entity else { continue };
        let existing = state.cameras.get(&job.key).copied();
        let reusable = existing.and_then(|entity| cameras.get_mut(entity).ok()).filter(
            |(_, _, sensor, _)| sensor.spec == job.spec && sensor.parent == parent,
        );
        let Some((_, mut camera, mut sensor, _)) = reusable else {
            if let Some(stale) = existing {
                commands.entity(stale).despawn();
            }
            let order = -1000 - state.cameras.len() as isize;
            let entity = spawn_camera(
                &mut commands,
                &mut images,
                weather.as_deref(),
                has_lamps,
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
                "gearbox-sensors: bevy cameras: {} renders, {} published, {} undelivered, {} superseded, {:.1} KB read back per render in the last {:.0} s",
                s.renders,
                s.published,
                s.undelivered,
                s.superseded,
                s.readback_bytes as f64 / 1024.0 / s.renders as f64,
                now - state.last_report
            );
        }
        state.stats = CameraStats::default();
        state.last_report = now;
    }
}

/// Files this frame's `jobs` with the samples cameras still owe, one each,
/// the newest, and hands back the `budget` owed longest; only `live` cameras
/// owe any. Counts each sample a newer one replaced in `superseded`.
fn take_due<T>(
    waiting: &mut HashMap<CameraKey, (u64, T)>,
    jobs: impl IntoIterator<Item = (CameraKey, T)>,
    live: &HashSet<CameraKey>,
    frame: u64,
    budget: usize,
    superseded: &mut u64,
) -> Vec<T> {
    for (key, job) in jobs {
        let since = waiting.get(&key).map_or(frame, |(since, _)| *since);
        if waiting.insert(key, (since, job)).is_some() {
            *superseded += 1;
        }
    }
    waiting.retain(|key, _| live.contains(key));
    let mut due: Vec<_> = waiting.iter().map(|(key, (since, _))| (*since, key.clone())).collect();
    due.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| (&a.1.bus_key, &a.1.link).cmp(&(&b.1.bus_key, &b.1.link))));
    due.into_iter()
        .take(budget)
        .filter_map(|(_, key)| waiting.remove(&key).map(|(_, job)| job))
        .collect()
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
    fn sample_activation_precedes_camera_and_shadow_preparation() {
        #[derive(Resource, Default)]
        struct Seen(Vec<bool>);
        fn observe(cameras: Query<&Camera, With<SensorCamera>>, mut seen: ResMut<Seen>) {
            seen.0.push(cameras.single().unwrap().is_active);
        }
        let mut app = App::new();
        app.init_resource::<Time>().init_resource::<Assets<Image>>().init_resource::<Seen>()
            .add_systems(PostUpdate, observe.in_set(CameraUpdateSystems))
            .add_plugins(SensorCamerasPlugin);
        let key = CameraKey { bus_key: "rig".into(), link: "camera".into() };
        let parent = app.world_mut().spawn_empty().id();
        let spec = SensorSpec {
            rate_hz: 2.0,
            vfov: 1.0,
            range_m: 20.0,
            width: 160,
            height: 120,
            render: CameraRender::Full,
            color: true,
            depth: false,
            ..SensorSpec::defaults(crate::links::SensorKind::Camera, false)
        };
        let entity = app.world_mut().spawn((Camera { is_active: false, ..default() },
            SensorCamera { key: key.clone(), link_index: 0, spec,
                image: Handle::default(), parent, pending: VecDeque::new(), armed: false },
            Pending { sample: 1, sim_time: 0.0, stamp_ms: 0 })).id();
        app.world_mut().resource_mut::<SensorRenderQueue>().live.insert(key.clone());
        app.world_mut().resource_mut::<SensorCameras>().cameras.insert(key, entity);
        for _ in 0..3 { app.update(); }
        assert_eq!(app.world().resource::<Seen>().0, [false, true, false]);
        assert_eq!(app.world().resource::<SensorCameras>().stats.renders, 1);
    }

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
    fn a_frame_renders_its_budget_of_owed_samples_oldest_first_newest_each() {
        let key = |link: &str| CameraKey { bus_key: "rover".into(), link: link.into() };
        let live: HashSet<_> = ["a", "b", "c"].map(key).into_iter().collect();
        let (mut waiting, mut superseded) = (HashMap::new(), 0);
        let first = take_due(&mut waiting, [(key("a"), 1), (key("b"), 1), (key("c"), 1)], &live, 1, 2, &mut superseded);
        assert_eq!(first.len(), 2);
        assert_eq!(waiting.len(), 1);
        let owed = waiting.keys().next().unwrap().clone();
        let next = take_due(&mut waiting, [(key("a"), 2), (key("b"), 2), (owed.clone(), 2)], &live, 2, 1, &mut superseded);
        assert_eq!((next, superseded), (vec![2], 1));
        assert!(!waiting.contains_key(&owed) && waiting.len() == 2);
        let gone = take_due(&mut waiting, [(key("x"), 3)], &live, 3, 0, &mut superseded);
        assert!(gone.is_empty() && !waiting.contains_key(&key("x")));
    }

    #[test]
    fn link_forward_and_up_become_camera_forward_and_up() {
        let t = camera_in_link();
        assert!((t.rotation * Vec3::NEG_Z - Vec3::X).length() < 1e-6);
        assert!((t.rotation * Vec3::Y - Vec3::Z).length() < 1e-6);
    }

    #[test]
    fn full_cameras_cluster_only_when_lamps_exist_and_follow_weather_clear_color() {
        use bevy::light::cluster::{ClusterConfig, ClusterFarZMode};

        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<Assets<Image>>()
            .init_resource::<bevy_weather::WeatherSettings>()
            .add_plugins(SensorCamerasPlugin);

        let parent = app.world_mut().spawn_empty().id();
        let full_key = CameraKey { bus_key: "rig".into(), link: "full_cam".into() };
        let opt_key = CameraKey { bus_key: "rig".into(), link: "opt_cam".into() };
        let full_spec = SensorSpec {
            render: CameraRender::Full,
            range_m: 45.0,
            ..SensorSpec::defaults(crate::links::SensorKind::Camera, false)
        };
        let opt_spec = SensorSpec {
            render: CameraRender::Optimized,
            range_m: 45.0,
            ..SensorSpec::defaults(crate::links::SensorKind::Camera, false)
        };
        {
            let mut queue = app.world_mut().resource_mut::<SensorRenderQueue>();
            queue.live.insert(full_key.clone());
            queue.live.insert(opt_key.clone());
            queue.jobs.push(RenderJob {
                key: full_key.clone(),
                link_index: 1,
                spec: full_spec,
                entity: Some(parent),
                sample: 1,
                sim_time: 0.0,
                stamp_ms: 0,
            });
            queue.jobs.push(RenderJob {
                key: opt_key.clone(),
                link_index: 2,
                spec: opt_spec,
                entity: Some(parent),
                sample: 1,
                sim_time: 0.0,
                stamp_ms: 0,
            });
        }
        app.update();

        let full_e = app.world().resource::<SensorCameras>().cameras[&full_key];
        let opt_e = app.world().resource::<SensorCameras>().cameras[&opt_key];
        assert!(matches!(app.world().get::<Camera>(full_e).unwrap().clear_color, ClearColorConfig::Default));
        assert!(matches!(app.world().get::<ClusterConfig>(full_e).unwrap(), ClusterConfig::None));
        assert!(matches!(app.world().get::<ClusterConfig>(opt_e).unwrap(), ClusterConfig::None));

        // Adding a lamp enables bounded clustering on Full only.
        let lamp = app.world_mut().spawn(PointLight::default()).id();
        app.update();
        match app.world().get::<ClusterConfig>(full_e).unwrap() {
            ClusterConfig::FixedZ { z_config, .. } => {
                assert!(matches!(z_config.far_z_mode, ClusterFarZMode::Constant(r) if (r - 45.0).abs() < 1e-4));
            }
            other => panic!("expected FixedZ cluster config, got {other:?}"),
        }
        assert!(matches!(app.world().get::<ClusterConfig>(opt_e).unwrap(), ClusterConfig::None));

        // Removing the lamp drops clustering back to None.
        app.world_mut().despawn(lamp);
        app.update();
        assert!(matches!(app.world().get::<ClusterConfig>(full_e).unwrap(), ClusterConfig::None));
    }
}
