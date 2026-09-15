//! Clean HDR captures of the viewport. Frames are copied from the camera's
//! HDR target after bloom and before tonemapping, so panels never reach
//! them; gizmos, the selection band and machine cards sit on an overlay
//! render layer the camera drops while capturing. Stills save an OpenEXR
//! frame beside the tonemapped PNG; recordings stream HDR10 HEVC to ffmpeg.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::Duration;

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::gizmos::config::GizmoConfigStore;
use bevy::post_process::bloom::bloom;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::ViewTarget;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::TimeUpdateStrategy;
use mara::ui::modules::bevy::ChaseCamera;

/// Render layer of viewer overlays; the camera sees it except while capturing.
pub const OVERLAY_LAYER: usize = 1;
const FRAME_RATE: u32 = 60;
/// Frames the overlays take to leave the view before a still is taken.
const SETTLE_FRAMES: u8 = 2;
/// Frames that may wait for ffmpeg before the app waits with them.
const QUEUED_FRAMES: usize = 8;
/// Scene-linear BT.709 to HDR10: 1.0 is 203 nit reference white.
const HDR10_FILTER: &str = "crop=trunc(iw/2)*2:trunc(ih/2)*2,format=gbrpf32le,\
    zscale=tin=linear:pin=bt709:min=gbr:npl=203:t=smpte2084:p=bt2020:m=bt2020nc:r=tv,format=p010le";

pub struct RecorderPlugin;

impl Plugin for RecorderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Recorder>()
            .add_plugins(ExtractComponentPlugin::<CaptureInto>::default())
            .add_systems(
                Update,
                (show_overlays, keep_gizmos_on_overlay_layer, drive_recorder).chain(),
            );
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                Core3d,
                copy_hdr_frame
                    .in_set(Core3dSystems::PostProcess)
                    .after(bloom)
                    .before(tonemapping),
            );
        }
    }
}

/// What the pane asks for, and the capture under way.
#[derive(Resource)]
pub struct Recorder {
    still_requested: bool,
    start_requested: bool,
    stop_requested: bool,
    mode: Mode,
    last: Option<PathBuf>,
    folder: Option<PathBuf>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            still_requested: false,
            start_requested: false,
            stop_requested: false,
            mode: Mode::Idle,
            last: None,
            folder: load_folder(),
        }
    }
}

impl Recorder {
    pub fn request_still(&mut self) {
        self.still_requested = true;
    }

    pub fn toggle_recording(&mut self) {
        if self.is_recording() {
            self.stop_requested = true;
        } else {
            self.start_requested = true;
        }
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.mode, Mode::Recording { .. })
    }

    pub fn status(&self) -> String {
        match &self.mode {
            Mode::Idle => "ready".to_string(),
            Mode::Still { .. } => "taking screenshot".to_string(),
            Mode::Recording { frames, .. } => {
                format!("recording {:.1} s", *frames as f64 / f64::from(FRAME_RATE))
            }
        }
    }

    /// The last still or video written.
    pub fn last(&self) -> Option<&Path> {
        self.last.as_deref()
    }

    /// Where captures go; `None` keeps them in ~/Pictures and ~/Videos.
    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    pub fn set_folder(&mut self, folder: Option<PathBuf>) {
        if let Err(error) = save_folder(folder.as_deref()) {
            warn!("gearbox-capture: cannot remember the capture folder: {error}");
        }
        self.folder = folder;
    }
}

#[derive(Default)]
enum Mode {
    #[default]
    Idle,
    Still {
        stem: PathBuf,
        size: UVec2,
        settle: u8,
        reader: Option<Entity>,
        saved: bool,
    },
    Recording {
        encoder: Encoder,
        path: PathBuf,
        size: UVec2,
        reader: Entity,
        frames: u64,
    },
}

/// The HDR image a capturing camera copies its frame into.
#[derive(Component, ExtractComponent, Clone)]
pub struct CaptureInto(pub Handle<Image>);

fn show_overlays(mut commands: Commands, cameras: Query<Entity, Added<ChaseCamera>>) {
    for camera in &cameras {
        commands
            .entity(camera)
            .insert(RenderLayers::from_layers(&[0, OVERLAY_LAYER]));
    }
}

fn keep_gizmos_on_overlay_layer(store: Option<ResMut<GizmoConfigStore>>) {
    let Some(mut store) = store else {
        return;
    };
    let overlay = RenderLayers::layer(OVERLAY_LAYER);
    if store.iter().all(|(_, config, _)| config.render_layers == overlay) {
        return;
    }
    for (_, config, _) in store.iter_mut() {
        config.render_layers = overlay.clone();
    }
}

fn drive_recorder(
    mut recorder: ResMut<Recorder>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut clock: ResMut<TimeUpdateStrategy>,
    cameras: Query<(Entity, &Camera, &RenderTarget), With<ChaseCamera>>,
) {
    let Ok((view, camera, target)) = cameras.single() else {
        return;
    };
    let recorder = recorder.as_mut();
    let mut next = None;
    match &mut recorder.mode {
        Mode::Idle => {
            let Some(size) = camera.physical_target_size() else {
                return;
            };
            if std::mem::take(&mut recorder.start_requested) {
                let path = capture_path(recorder.folder.as_deref(), "Videos", "mp4");
                let mut args = raw_input(size);
                args.extend(
                    [
                        "-vf", HDR10_FILTER, "-c:v", "hevc_nvenc", "-profile:v", "main10",
                        "-preset", "p5", "-rc", "vbr", "-cq", "16", "-b:v", "0",
                        "-color_primaries", "bt2020", "-color_trc", "smpte2084",
                        "-colorspace", "bt2020nc", "-color_range", "tv", "-tag:v", "hvc1",
                    ]
                    .map(String::from),
                );
                match Encoder::spawn(&args, &path) {
                    Ok(encoder) => {
                        let reader = begin_capture(&mut commands, &mut images, view, size);
                        *clock = TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                            1.0 / f64::from(FRAME_RATE),
                        ));
                        info!("gearbox-capture: recording {}", path.display());
                        next = Some(Mode::Recording { encoder, path, size, reader, frames: 0 });
                    }
                    Err(error) => warn!("gearbox-capture: cannot start ffmpeg: {error}"),
                }
            } else if std::mem::take(&mut recorder.still_requested) {
                commands.entity(view).insert(RenderLayers::layer(0));
                next = Some(Mode::Still {
                    stem: capture_path(recorder.folder.as_deref(), "Pictures", "png")
                        .with_extension(""),
                    size,
                    settle: SETTLE_FRAMES,
                    reader: None,
                    saved: false,
                });
            }
        }
        Mode::Still { stem, size, settle, reader, saved } => {
            if *settle > 0 {
                *settle -= 1;
            } else if let Some(reader) = *reader {
                if *saved {
                    end_capture(&mut commands, view, reader);
                    info!("gearbox-capture: still {}.png and .exr", stem.display());
                    recorder.last = Some(stem.with_extension("png"));
                    next = Some(Mode::Idle);
                }
            } else {
                commands
                    .spawn(Screenshot(target.clone()))
                    .observe(save_to_disk(stem.with_extension("png")));
                *reader = Some(begin_capture(&mut commands, &mut images, view, *size));
            }
        }
        Mode::Recording { reader, .. } => {
            if std::mem::take(&mut recorder.stop_requested) {
                end_capture(&mut commands, view, *reader);
                *clock = TimeUpdateStrategy::Automatic;
                next = Some(Mode::Idle);
            }
        }
    }
    if let Some(next) = next
        && let Mode::Recording { encoder, path, .. } = std::mem::replace(&mut recorder.mode, next)
    {
        encoder.finish();
        info!("gearbox-capture: finishing {}", path.display());
        recorder.last = Some(path);
    }
}

/// Points the camera at a fresh HDR image and reads it back every frame.
fn begin_capture(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    view: Entity,
    size: UVec2,
) -> Entity {
    let mut image = Image::new_uninit(
        Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
        TextureDimension::D2,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC | TextureUsages::COPY_DST;
    let handle = images.add(image);
    commands
        .entity(view)
        .insert((CaptureInto(handle.clone()), RenderLayers::layer(0)));
    commands.spawn(Readback::texture(handle)).observe(receive_frame).id()
}

fn end_capture(commands: &mut Commands, view: Entity, reader: Entity) {
    commands
        .entity(view)
        .remove::<CaptureInto>()
        .insert(RenderLayers::from_layers(&[0, OVERLAY_LAYER]));
    commands.entity(reader).despawn();
}

fn receive_frame(frame: On<ReadbackComplete>, mut recorder: ResMut<Recorder>) {
    match &mut recorder.mode {
        Mode::Recording { encoder, size, frames, .. } => {
            encoder.push(unpad(&frame.data, *size));
            *frames += 1;
        }
        Mode::Still { stem, size, saved, .. } if !*saved => {
            let path = stem.with_extension("exr");
            let mut args = raw_input(*size);
            args.extend(
                [
                    "-vf", "format=gbrpf32le", "-c:v", "exr", "-format", "half",
                    "-compression", "zip16", "-frames:v", "1",
                ]
                .map(String::from),
            );
            match Encoder::spawn(&args, &path) {
                Ok(encoder) => {
                    encoder.push(unpad(&frame.data, *size));
                    encoder.finish();
                }
                Err(error) => warn!("gearbox-capture: cannot start ffmpeg: {error}"),
            }
            *saved = true;
        }
        _ => {}
    }
}

/// Copies the HDR frame, bloom included, before tonemapping takes it.
fn copy_hdr_frame(
    view: ViewQuery<(&ViewTarget, &CaptureInto)>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, capture) = view.into_inner();
    let Some(image) = images.get(&capture.0) else {
        return;
    };
    let source = target.main_texture();
    if source.size() != image.texture.size()
        || source.format() != image.texture.format()
        || source.sample_count() != 1
    {
        return;
    }
    ctx.command_encoder().copy_texture_to_texture(
        source.as_image_copy(),
        image.texture.as_image_copy(),
        image.texture.size(),
    );
}

/// Readback rows are padded to 256 bytes; ffmpeg wants them packed.
fn unpad(data: &[u8], size: UVec2) -> Vec<u8> {
    let row = size.x as usize * 8;
    let stride = row.div_ceil(256) * 256;
    let mut packed = Vec::with_capacity(row * size.y as usize);
    for line in data.chunks(stride).take(size.y as usize) {
        packed.extend_from_slice(&line[..row]);
    }
    packed
}

fn raw_input(size: UVec2) -> Vec<String> {
    let mut args: Vec<String> = ["-f", "rawvideo", "-pix_fmt", "rgbaf16le", "-s"]
        .map(String::from)
        .to_vec();
    args.push(format!("{}x{}", size.x, size.y));
    args.extend(["-framerate", "60", "-i", "-"].map(String::from));
    args
}

/// `<folder>/gearbox-<local time>.<ext>`, the folder defaulting to
/// `~/<kind>/gearbox`.
fn capture_path(folder: Option<&Path>, kind: &str, ext: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    let dir = folder.map_or_else(|| home.join(kind).join("gearbox"), Path::to_path_buf);
    let _ = std::fs::create_dir_all(&dir);
    let stamp = Command::new("date")
        .arg("+%Y%m%d-%H%M%S")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|stamp| stamp.trim().to_string())
        .filter(|stamp| !stamp.is_empty())
        .unwrap_or_else(|| format!("{:?}", std::time::SystemTime::now()));
    dir.join(format!("gearbox-{stamp}.{ext}"))
}

/// `$XDG_CONFIG_HOME/gearbox/capture_folder`: the chosen folder on one line.
fn folder_file() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|config| config.join("gearbox/capture_folder"))
}

fn load_folder() -> Option<PathBuf> {
    let text = std::fs::read_to_string(folder_file()?).ok()?;
    let folder = PathBuf::from(text.trim());
    folder.is_dir().then_some(folder)
}

fn save_folder(folder: Option<&Path>) -> std::io::Result<()> {
    let file = folder_file().ok_or_else(|| std::io::Error::other("no config directory"))?;
    match folder {
        Some(folder) => {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(file, folder.to_string_lossy().as_bytes())
        }
        None => match std::fs::remove_file(file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        },
    }
}

/// An ffmpeg process fed raw frames from a worker thread.
struct Encoder {
    frames: SyncSender<Vec<u8>>,
}

impl Encoder {
    fn spawn(args: &[String], out: &Path) -> std::io::Result<Self> {
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(args)
            .arg(out)
            .stdin(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("ffmpeg stdin is piped");
        let (frames, queue) = sync_channel::<Vec<u8>>(QUEUED_FRAMES);
        let out = out.to_path_buf();
        std::thread::spawn(move || {
            for frame in queue {
                if stdin.write_all(&frame).is_err() {
                    break;
                }
            }
            drop(stdin);
            match child.wait() {
                Ok(status) if status.success() => info!("gearbox-capture: saved {}", out.display()),
                Ok(status) => warn!("gearbox-capture: ffmpeg {status} for {}", out.display()),
                Err(error) => warn!("gearbox-capture: ffmpeg failed: {error}"),
            }
        });
        Ok(Self { frames })
    }

    fn push(&self, frame: Vec<u8>) {
        let _ = self.frames.send(frame);
    }

    /// Closing the queue ends the stream; ffmpeg finalises on its own.
    fn finish(self) {}
}
