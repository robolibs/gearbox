//! Keyboard for the embedded app. mara forwards pointer input to the
//! viewport itself; keys are read from egui here and written to Bevy as
//! `KeyboardInput` messages, so `ButtonInput<KeyCode>` works as in a Bevy
//! window. Presses stop while a text field has the keyboard, releases
//! always go through so nothing sticks.

use std::collections::HashSet;

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyCode, KeyboardInput, NativeKey};
use bevy::prelude::*;

#[derive(Default)]
pub struct KeyBridge {
    held: HashSet<KeyCode>,
    shift: bool,
    ctrl: bool,
    alt: bool,
}

impl KeyBridge {
    pub fn forward(&mut self, egui: &egui::Context, world: &mut World) {
        let wants_text = egui.egui_wants_keyboard_input();
        let (events, modifiers) = egui.input(|i| (i.events.clone(), i.modifiers));
        let mut out: Vec<KeyboardInput> = Vec::new();
        for event in events {
            let egui::Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                ..
            } = event
            else {
                continue;
            };
            let Some(code) = key_code(physical_key.unwrap_or(key)) else {
                continue;
            };
            if pressed {
                if wants_text {
                    continue;
                }
                if !self.held.insert(code) && !repeat {
                    continue;
                }
            } else if !self.held.remove(&code) {
                continue;
            }
            out.push(message(code, pressed, repeat));
        }
        for (held, now, code) in [
            (&mut self.shift, modifiers.shift, KeyCode::ShiftLeft),
            (&mut self.ctrl, modifiers.ctrl, KeyCode::ControlLeft),
            (&mut self.alt, modifiers.alt, KeyCode::AltLeft),
        ] {
            if *held != now {
                *held = now;
                out.push(message(code, now, false));
            }
        }
        if wants_text && !self.held.is_empty() {
            for code in self.held.drain() {
                out.push(message(code, false, false));
            }
        }
        for msg in out {
            world.write_message(msg);
        }
    }
}

fn message(code: KeyCode, pressed: bool, repeat: bool) -> KeyboardInput {
    KeyboardInput {
        key_code: code,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state: if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        },
        text: None,
        repeat,
        window: Entity::PLACEHOLDER,
    }
}

fn key_code(key: egui::Key) -> Option<KeyCode> {
    use egui::Key as K;
    Some(match key {
        K::A => KeyCode::KeyA,
        K::B => KeyCode::KeyB,
        K::C => KeyCode::KeyC,
        K::D => KeyCode::KeyD,
        K::E => KeyCode::KeyE,
        K::F => KeyCode::KeyF,
        K::G => KeyCode::KeyG,
        K::H => KeyCode::KeyH,
        K::I => KeyCode::KeyI,
        K::J => KeyCode::KeyJ,
        K::K => KeyCode::KeyK,
        K::L => KeyCode::KeyL,
        K::M => KeyCode::KeyM,
        K::N => KeyCode::KeyN,
        K::O => KeyCode::KeyO,
        K::P => KeyCode::KeyP,
        K::Q => KeyCode::KeyQ,
        K::R => KeyCode::KeyR,
        K::S => KeyCode::KeyS,
        K::T => KeyCode::KeyT,
        K::U => KeyCode::KeyU,
        K::V => KeyCode::KeyV,
        K::W => KeyCode::KeyW,
        K::X => KeyCode::KeyX,
        K::Y => KeyCode::KeyY,
        K::Z => KeyCode::KeyZ,
        K::Num0 => KeyCode::Digit0,
        K::Num1 => KeyCode::Digit1,
        K::Num2 => KeyCode::Digit2,
        K::Num3 => KeyCode::Digit3,
        K::Num4 => KeyCode::Digit4,
        K::Num5 => KeyCode::Digit5,
        K::Num6 => KeyCode::Digit6,
        K::Num7 => KeyCode::Digit7,
        K::Num8 => KeyCode::Digit8,
        K::Num9 => KeyCode::Digit9,
        K::Escape => KeyCode::Escape,
        K::Space => KeyCode::Space,
        K::Enter => KeyCode::Enter,
        K::Tab => KeyCode::Tab,
        K::Backspace => KeyCode::Backspace,
        K::Delete => KeyCode::Delete,
        K::ArrowUp => KeyCode::ArrowUp,
        K::ArrowDown => KeyCode::ArrowDown,
        K::ArrowLeft => KeyCode::ArrowLeft,
        K::ArrowRight => KeyCode::ArrowRight,
        K::Home => KeyCode::Home,
        K::End => KeyCode::End,
        K::PageUp => KeyCode::PageUp,
        K::PageDown => KeyCode::PageDown,
        K::Minus => KeyCode::Minus,
        K::Plus | K::Equals => KeyCode::Equal,
        K::Comma => KeyCode::Comma,
        K::Period => KeyCode::Period,
        K::Slash | K::Questionmark => KeyCode::Slash,
        K::F1 => KeyCode::F1,
        K::F2 => KeyCode::F2,
        K::F3 => KeyCode::F3,
        K::F4 => KeyCode::F4,
        K::F5 => KeyCode::F5,
        K::F6 => KeyCode::F6,
        K::F7 => KeyCode::F7,
        K::F8 => KeyCode::F8,
        K::F9 => KeyCode::F9,
        K::F10 => KeyCode::F10,
        K::F11 => KeyCode::F11,
        K::F12 => KeyCode::F12,
        _ => return None,
    })
}
