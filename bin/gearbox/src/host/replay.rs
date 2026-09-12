//! Scripted input for the window, so a shell can drive the panes the way a
//! pointer would: `gearbox instance ui "click 30 140"` drops the script in
//! `<registry dir>/<name>.ui`; an egui plugin feeds one line per frame into
//! the input stream. Lines: `move X Y`, `down X Y`, `up X Y`, `click X Y`,
//! `scroll DX DY`, `text …`, `key [ctrl+][shift+]NAME`, `wait MS`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers, PointerButton, RawInput};

use crate::viewer::screenshot::take_request;

pub fn install(ctx: &egui::Context) {
    ctx.add_plugin(Replay::default());
}

#[derive(Default)]
struct Replay {
    queue: VecDeque<Step>,
    last_poll: Option<Instant>,
    wait_until: Option<Instant>,
}

enum Step {
    Event(Event),
    Wait(Duration),
}

impl egui::Plugin for Replay {
    fn debug_name(&self) -> &'static str {
        "gearbox-ui-replay"
    }

    fn input_hook(&mut self, input: &mut RawInput) {
        let now = Instant::now();
        if self.queue.is_empty()
            && self
                .last_poll
                .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_millis(100))
        {
            self.last_poll = Some(now);
            if let Some(script) = take_request("ui") {
                match parse(&script.to_string_lossy()) {
                    Ok(steps) => self.queue = steps,
                    Err(error) => tracing::warn!(target: "gearbox", "ui script: {error}"),
                }
            }
        }
        if let Some(until) = self.wait_until {
            if now < until {
                return;
            }
            self.wait_until = None;
        }
        match self.queue.pop_front() {
            Some(Step::Event(event)) => input.events.push(event),
            Some(Step::Wait(delay)) => self.wait_until = Some(now + delay),
            None => {}
        }
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        if !self.queue.is_empty() || self.wait_until.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}

fn parse(script: &str) -> Result<VecDeque<Step>, String> {
    let mut steps = VecDeque::new();
    for (index, line) in script.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let error = || format!("line {}: `{line}` not understood", index + 1);
        let mut words = line.split_whitespace();
        let action = words.next().ok_or_else(error)?;
        let args: Vec<&str> = words.collect();
        let point = |i: usize| -> Result<f32, String> {
            args.get(i)
                .and_then(|w| w.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .ok_or_else(error)
        };
        match action {
            "move" => steps.push_back(Step::Event(Event::PointerMoved(egui::pos2(point(0)?, point(1)?)))),
            "down" | "up" => steps.push_back(Step::Event(button(point(0)?, point(1)?, action == "down"))),
            "click" => {
                let (x, y) = (point(0)?, point(1)?);
                steps.push_back(Step::Event(Event::PointerMoved(egui::pos2(x, y))));
                steps.push_back(Step::Event(button(x, y, true)));
                steps.push_back(Step::Event(button(x, y, false)));
            }
            "scroll" => steps.push_back(Step::Event(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(point(0)?, point(1)?),
                modifiers: Modifiers::NONE,
            })),
            "text" => steps.push_back(Step::Event(Event::Text(args.join(" ")))),
            "key" => {
                let spec = args.first().ok_or_else(error)?;
                let (modifiers, name) = split_modifiers(spec);
                let key = Key::from_name(name).ok_or_else(error)?;
                for pressed in [true, false] {
                    steps.push_back(Step::Event(Event::Key {
                        key,
                        physical_key: Some(key),
                        pressed,
                        repeat: false,
                        modifiers,
                    }));
                }
            }
            "wait" => {
                let millis = args
                    .first()
                    .and_then(|w| w.parse::<u64>().ok())
                    .ok_or_else(error)?;
                steps.push_back(Step::Wait(Duration::from_millis(millis)));
            }
            _ => return Err(error()),
        }
    }
    Ok(steps)
}

fn button(x: f32, y: f32, pressed: bool) -> Event {
    Event::PointerButton {
        pos: egui::pos2(x, y),
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

/// `ctrl+shift+k` → the modifiers and the key name.
fn split_modifiers(spec: &str) -> (Modifiers, &str) {
    let mut modifiers = Modifiers::NONE;
    let mut name = spec;
    while let Some((prefix, rest)) = name.split_once('+') {
        match prefix.to_ascii_lowercase().as_str() {
            "ctrl" => modifiers.ctrl = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            _ => break,
        }
        name = rest;
    }
    (modifiers, name)
}
