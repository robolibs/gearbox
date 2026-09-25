//! Live viewer for simulated camera links. Subscribes to
//! `/machines/<machine>/sensors/<link>` on the running gearbox instance and
//! shows every stream's color (or depth) image with its delivered frame rate
//! and bandwidth.
//!
//! ```text
//! cargo run --release -p gearbox-sim --example camera_view -- <machine> [link ...]
//! ```
//!
//! Links default to `camera_link`. `GEARBOX_INSTANCE` picks the instance when
//! several are running. The viewer connects as the `gearbox-camera-view`
//! identity; `camera_view --did` prints it for `gearbox run --allow <did>`.
//! `CAMERA_VIEW_SNAPSHOTS=<dir>` also writes each stream's latest colour
//! frame to `<dir>/<link>.png` every few seconds.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gearbox_api::topics::machine_sensor_topic;
use gearbox_api::{CAMERA_COLOR, CameraFrame, Client, IdentitySource, registry, unpack};
use mara::window::{AppRunner, CreationContext, MaraHostCtx, WindowApp};

/// Frame-rate and bandwidth window.
const RATE_WINDOW: Duration = Duration::from_secs(2);

const IDENTITY: &str = "gearbox-camera-view";

#[derive(Default)]
struct Stream {
    link: String,
    status: String,
    width: u32,
    height: u32,
    max_range: f32,
    sample: u32,
    sim_time: f64,
    color: Vec<u8>,
    depth: Vec<f32>,
    revision: u64,
    frames: VecDeque<Instant>,
    bytes: VecDeque<(Instant, usize)>,
    /// Colour and depth frames being put together, each by its own sample.
    assembling: [Option<Assembly>; 2],
}

/// One channel of one frame being put together from its row bands.
struct Assembly {
    sample: u32,
    data: Vec<u8>,
    rows: u32,
}

impl Stream {
    fn accept(&mut self, chunk: &CameraFrame, bytes: usize) {
        let now = Instant::now();
        self.bytes.push_back((now, bytes));
        if chunk.width != self.width || chunk.height != self.height {
            self.width = chunk.width;
            self.height = chunk.height;
            self.assembling = [None, None];
            self.color.clear();
            self.depth.clear();
        }
        let channel = usize::from(chunk.channel != CAMERA_COLOR);
        let size = (chunk.width * chunk.height) as usize * 4;
        let slot = &mut self.assembling[channel];
        if slot.as_ref().is_none_or(|a| a.sample != chunk.sample) {
            *slot = Some(Assembly {
                sample: chunk.sample,
                data: vec![0; size],
                rows: 0,
            });
        }
        let frame = slot.as_mut().expect("assembly just ensured");
        let start = (chunk.row_offset * chunk.width) as usize * 4;
        let end = (start + chunk.data.len()).min(size);
        frame.data[start..end].copy_from_slice(&chunk.data[..end - start]);
        frame.rows += chunk.rows;
        if frame.rows < chunk.height {
            return;
        }
        let frame = slot.take().expect("assembly present");
        if channel == 0 {
            self.color = frame.data;
            self.sample = frame.sample;
            self.sim_time = chunk.sim_time_s;
            self.frames.push_back(now);
        } else {
            self.depth = frame
                .data
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            if self.color.is_empty() {
                self.sample = frame.sample;
                self.sim_time = chunk.sim_time_s;
                self.frames.push_back(now);
            }
        }
        self.max_range = chunk.max_range_m as f32;
        self.revision += 1;
    }

    fn rates(&mut self) -> (f64, f64) {
        let cutoff = Instant::now() - RATE_WINDOW;
        while self.frames.front().is_some_and(|t| *t < cutoff) {
            self.frames.pop_front();
        }
        while self.bytes.front().is_some_and(|(t, _)| *t < cutoff) {
            self.bytes.pop_front();
        }
        let window = RATE_WINDOW.as_secs_f64();
        let bytes: usize = self.bytes.iter().map(|(_, b)| b).sum();
        (self.frames.len() as f64 / window, bytes as f64 / window / 1e6)
    }
}

/// Owns the client and every subscriber; drains them into the shared streams.
fn receive(machine: String, streams: Vec<Arc<Mutex<Stream>>>) {
    let set_status = |text: String| {
        for stream in &streams {
            stream.lock().unwrap().status = text.clone();
        }
    };
    let entry = loop {
        let wanted = std::env::var("GEARBOX_INSTANCE").ok();
        let entries = registry::list();
        let found = match wanted {
            Some(name) => entries.into_iter().find(|e| e.name == name || e.did == name),
            None => entries.into_iter().next(),
        };
        match found {
            Some(entry) => break entry,
            None => set_status("waiting for a gearbox instance".into()),
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    let client = loop {
        match Client::connect(&entry.did, IdentitySource::Name(IDENTITY.into()), IDENTITY) {
            Ok(client) => break client,
            Err(error) => set_status(format!("connecting to `{}`: {error}", entry.name)),
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    let mut subscribers: Vec<_> = streams.iter().map(|_| None).collect();
    loop {
        let mut idle = true;
        for (stream, subscriber) in streams.iter().zip(subscribers.iter_mut()) {
            if subscriber.is_none() {
                let link = stream.lock().unwrap().link.clone();
                let topic = machine_sensor_topic(&machine, &link);
                match client.subscribe_env(&topic) {
                    Ok(sub) => {
                        stream.lock().unwrap().status = format!("subscribed to {topic}");
                        *subscriber = Some(sub);
                    }
                    Err(error) => {
                        stream.lock().unwrap().status = format!("waiting for {topic}: {error}");
                        continue;
                    }
                }
            }
            let sub = subscriber.as_mut().expect("subscribed above");
            loop {
                match sub.take() {
                    Ok(Some(sample)) => {
                        idle = false;
                        let payload = sample.payload();
                        match unpack::<CameraFrame>(sample.header().type_hash, payload) {
                            Ok(chunk) => stream.lock().unwrap().accept(&chunk, payload.len()),
                            Err(error) => stream.lock().unwrap().status = format!("decode: {error}"),
                        }
                    }
                    Ok(None) => break,
                    Err(gearbox_api::peerbus::Error::Lagged { .. }) => continue,
                    Err(error) => {
                        stream.lock().unwrap().status = format!("receive: {error}");
                        *subscriber = None;
                        break;
                    }
                }
            }
        }
        if idle {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

struct CameraView {
    machine: String,
    streams: Vec<Arc<Mutex<Stream>>>,
    textures: Vec<Option<(egui::TextureHandle, u64, bool)>>,
    show_depth: bool,
    snapshots: Option<(std::path::PathBuf, Instant)>,
}

/// Seconds between snapshot dumps.
const SNAPSHOT_PERIOD: Duration = Duration::from_secs(3);

fn write_snapshot(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, image.size[0] as u32, image.size[1] as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&bytes))
        .map_err(std::io::Error::other)
}

impl WindowApp for CameraView {
    fn new(_: CreationContext<'_>) -> Self {
        let mut args = std::env::args().skip(1);
        let machine = args.next().unwrap_or_else(|| "sy".to_string());
        let mut links: Vec<String> = args.collect();
        if links.is_empty() {
            links.push("camera_link".to_string());
        }
        let streams: Vec<_> = links
            .into_iter()
            .map(|link| Arc::new(Mutex::new(Stream { link, ..Default::default() })))
            .collect();
        let shared = streams.clone();
        let target = machine.clone();
        std::thread::spawn(move || receive(target, shared));
        Self {
            machine,
            textures: streams.iter().map(|_| None).collect(),
            streams,
            show_depth: false,
            snapshots: std::env::var_os("CAMERA_VIEW_SNAPSHOTS")
                .map(|dir| (dir.into(), Instant::now())),
        }
    }

    #[allow(deprecated)]
    fn update(&mut self, host: &mut MaraHostCtx<'_>) {
        let ctx = host.__internal_egui();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(36.0);
            ui.horizontal(|ui| {
                ui.heading(format!("machine `{}`", self.machine));
                ui.checkbox(&mut self.show_depth, "depth");
            });
            let columns = (self.streams.len() as f32).sqrt().ceil().max(1.0) as usize;
            let rows = self.streams.len().div_ceil(columns);
            let cell = egui::vec2(
                ui.available_width() / columns as f32 - 8.0,
                ui.available_height() / rows as f32 - 8.0,
            );
            let mut total = 0.0;
            egui::Grid::new("cameras").spacing([8.0, 8.0]).show(ui, |ui| {
                for (index, stream) in self.streams.iter().enumerate() {
                    let mut s = stream.lock().unwrap();
                    let (fps, mbps) = s.rates();
                    total += mbps;
                    ui.vertical(|ui| {
                        ui.set_width(cell.x);
                        ui.set_height(cell.y);
                        ui.label(format!(
                            "{}  {}x{}  {fps:.1} fps  {mbps:.1} MB/s  sample {}  t={:.2}s",
                            s.link, s.width, s.height, s.sample, s.sim_time
                        ));
                        if s.revision == 0 {
                            ui.label(&s.status);
                            return;
                        }
                        let stale = self.textures[index]
                            .as_ref()
                            .is_none_or(|(_, rev, depth)| *rev != s.revision || *depth != self.show_depth);
                        if stale {
                            let size = [s.width as usize, s.height as usize];
                            let pixels = size[0] * size[1];
                            let has_depth = s.depth.len() == pixels;
                            let has_color = s.color.len() == pixels * 4;
                            let image = if (self.show_depth && has_depth) || !has_color {
                                if !has_depth {
                                    return;
                                }
                                depth_image(size, &s.depth, s.max_range)
                            } else {
                                color_image(size, &s.color)
                            };
                            match &mut self.textures[index] {
                                Some((texture, rev, depth)) => {
                                    texture.set(image, egui::TextureOptions::NEAREST);
                                    (*rev, *depth) = (s.revision, self.show_depth);
                                }
                                slot => {
                                    let texture = ui.ctx().load_texture(
                                        format!("camera-{index}"),
                                        image,
                                        egui::TextureOptions::NEAREST,
                                    );
                                    *slot = Some((texture, s.revision, self.show_depth));
                                }
                            }
                        }
                        let (texture, _, _) = self.textures[index].as_ref().expect("texture set");
                        let aspect = s.width as f32 / s.height.max(1) as f32;
                        let height = (cell.y - 24.0).min(cell.x / aspect);
                        ui.image((texture.id(), egui::vec2(height * aspect, height)));
                    });
                    if (index + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });
            ui.label(format!("received {total:.1} MB/s in total"));
        });
        if let Some((dir, last)) = &mut self.snapshots
            && last.elapsed() >= SNAPSHOT_PERIOD
        {
            *last = Instant::now();
            for stream in &self.streams {
                let s = stream.lock().unwrap();
                if s.revision > 0 {
                    let image = color_image([s.width as usize, s.height as usize], &s.color);
                    if let Err(error) = write_snapshot(&dir.join(format!("{}.png", s.link)), &image) {
                        eprintln!("snapshot {}: {error}", s.link);
                    }
                }
            }
        }
        ctx.request_repaint();
    }
}

/// Hits keep their colour; a miss (alpha 0) is drawn as sky.
fn color_image(size: [usize; 2], rgba: &[u8]) -> egui::ColorImage {
    let pixels = rgba
        .chunks_exact(4)
        .map(|p| {
            if p[3] == 0 {
                egui::Color32::from_rgb(150, 190, 230)
            } else {
                egui::Color32::from_rgb(p[0], p[1], p[2])
            }
        })
        .collect();
    egui::ColorImage::new(size, pixels)
}

/// Near is bright, far is dark, a miss is dark blue.
fn depth_image(size: [usize; 2], depth: &[f32], max_range: f32) -> egui::ColorImage {
    let pixels = depth
        .iter()
        .map(|d| {
            if d.is_finite() {
                let v = (255.0 * (1.0 - (d / max_range.max(1e-3)).clamp(0.0, 1.0)).powf(2.0)) as u8;
                egui::Color32::from_gray(v)
            } else {
                egui::Color32::from_rgb(10, 14, 40)
            }
        })
        .collect();
    egui::ColorImage::new(size, pixels)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--did") {
        let key = gearbox_api::agentio::resolve_identity(&IdentitySource::Name(IDENTITY.into()))?;
        println!("{}", gearbox_api::agentio::did_key::endpoint_to_did_key(&key.public())?);
        return Ok(());
    }
    AppRunner::new()
        .title("gearbox camera view")
        .size(1280.0, 860.0)
        .run::<CameraView>()
}
