//! Layered gamepad routing, independent of rendering and vehicle physics.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Layer {
    #[default]
    Camera,
    Vehicle,
    Machine,
    Inactive,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    pub connected: bool,
    pub focused: bool,
    pub r1: bool,
    pub l1: bool,
    pub r2: f32,
    pub l2: f32,
    pub stop: bool,
    pub left: [f32; 2],
    pub right: [f32; 2],
    pub previous_pressed: bool,
    pub next_pressed: bool,
    pub follow_pressed: bool,
    pub cinematic_pressed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CameraInput {
    pub orbit: [f32; 2],
    pub pan: f32,
    pub forward: f32,
    pub lift: f32,
}

impl CameraInput {
    pub fn for_view(mut self, invert_y: bool) -> Self {
        if invert_y {
            self.orbit[1] = -self.orbit[1];
        }
        self
    }

    pub fn active(self) -> bool {
        self.orbit != [0.0; 2] || self.pan != 0.0 || self.forward != 0.0 || self.lift != 0.0
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Frame {
    pub layer: Layer,
    pub drive_enabled: bool,
    pub throttle: f32,
    pub steering: f32,
    pub camera: CameraInput,
    pub select_step: i8,
    pub toggle_follow: bool,
    pub toggle_cinematic: bool,
}

#[derive(Debug, Default)]
pub struct Router {
    armed: bool,
}

impl Router {
    pub fn require_release(&mut self) {
        self.armed = false;
    }

    pub fn update(&mut self, input: Input) -> Frame {
        if !input.connected || !input.focused {
            self.require_release();
            return Frame {
                layer: Layer::Inactive,
                ..Frame::default()
            };
        }
        if !input.r1 {
            self.armed = true;
        }
        if input.l1 || input.stop {
            self.require_release();
            return Frame {
                layer: if input.l1 {
                    Layer::Machine
                } else {
                    Layer::Inactive
                },
                ..Frame::default()
            };
        }
        if input.r1 {
            return Frame {
                layer: Layer::Vehicle,
                drive_enabled: self.armed,
                throttle: if self.armed {
                    deadzone(input.left[1])
                } else {
                    0.0
                },
                steering: if self.armed {
                    -deadzone(input.left[0])
                } else {
                    0.0
                },
                ..Frame::default()
            };
        }
        Frame {
            camera: CameraInput {
                orbit: input.right.map(deadzone),
                pan: deadzone(input.left[0]),
                forward: deadzone(input.left[1]),
                lift: deadzone(input.r2).max(0.0) - deadzone(input.l2).max(0.0),
            },
            select_step: i8::from(input.next_pressed) - i8::from(input.previous_pressed),
            toggle_follow: input.follow_pressed,
            toggle_cinematic: input.cinematic_pressed,
            ..Frame::default()
        }
    }
}

fn deadzone(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    value.signum() * ((value.abs() - 0.12) / 0.88).clamp(0.0, 1.0)
}

pub fn cycle_index(current: Option<usize>, len: usize, step: i8) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match current.filter(|index| *index < len) {
        Some(index) => Some((index as isize + step as isize).rem_euclid(len as isize) as usize),
        None => Some(if step < 0 { len - 1 } else { 0 }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_stick_moves_camera_and_drives_machine() {
        let mut router = Router::default();
        let sticks = Input {
            left: [-1.0, 1.0],
            right: [1.0, -1.0],
            ..input()
        };
        let camera = router.update(sticks).camera;
        assert_eq!(camera.orbit, [1.0, -1.0]);
        assert_eq!(camera.pan, -1.0);
        assert_eq!(camera.forward, 1.0);
        let inverted = camera.for_view(true);
        assert_eq!(inverted.orbit, [1.0, 1.0]);
        assert_eq!(inverted.pan, -1.0);
        assert_eq!(inverted.forward, 1.0);
        let drive = router.update(Input { r1: true, ..sticks });
        assert!(drive.drive_enabled);
        assert_eq!(drive.throttle, 1.0);
        assert_eq!(drive.steering, 1.0);
        assert!(!drive.camera.active());
    }

    #[test]
    fn inverting_turns_only_vertical_look() {
        let input = CameraInput {
            orbit: [0.4, -0.8],
            pan: 0.5,
            forward: 1.0,
            lift: 0.7,
        };
        assert_eq!(input.for_view(false), input);
        let inverted = input.for_view(true);
        assert_eq!(inverted.orbit, [0.4, 0.8]);
        assert_eq!(inverted.pan, 0.5);
        assert_eq!(inverted.forward, 1.0);
        assert_eq!(inverted.lift, 0.7);
    }

    #[test]
    fn follow_and_cinematic_toggles_are_independent() {
        let mut router = Router::default();
        let up = router.update(Input {
            follow_pressed: true,
            ..input()
        });
        assert!(up.toggle_follow);
        assert!(!up.toggle_cinematic);
        let down = router.update(Input {
            cinematic_pressed: true,
            ..input()
        });
        assert!(!down.toggle_follow);
        assert!(down.toggle_cinematic);
        let both = Input {
            follow_pressed: true,
            cinematic_pressed: true,
            ..input()
        };
        let frame = router.update(both);
        assert!(frame.toggle_follow && frame.toggle_cinematic);
        for input in [
            Input { r1: true, ..both },
            Input { l1: true, ..both },
            Input {
                focused: false,
                ..both
            },
            input(),
        ] {
            let frame = router.update(input);
            assert!(!frame.toggle_follow && !frame.toggle_cinematic);
        }
    }

    fn input() -> Input {
        Input {
            connected: true,
            focused: true,
            left: [0.5, 1.0],
            right: [1.0, 1.0],
            ..Input::default()
        }
    }

    #[test]
    fn layers_are_exclusive_and_release_stops_drive() {
        let mut router = Router::default();
        let normal = router.update(input());
        assert!(normal.camera.active());
        assert!(!normal.drive_enabled);
        let drive = router.update(Input {
            r1: true,
            ..input()
        });
        assert!(drive.drive_enabled);
        assert_eq!(drive.throttle, 1.0);
        assert!(!drive.camera.active());
        let reserved = router.update(Input {
            r1: true,
            l1: true,
            ..input()
        });
        assert_eq!(reserved.layer, Layer::Machine);
        assert!(!reserved.drive_enabled);
        assert!(
            !router
                .update(Input {
                    r1: true,
                    ..input()
                })
                .drive_enabled
        );
        assert!(!router.update(input()).drive_enabled);
        assert!(
            router
                .update(Input {
                    r1: true,
                    ..input()
                })
                .drive_enabled
        );
    }

    #[test]
    fn reconnect_focus_and_selection_require_deadman_release() {
        let mut router = Router::default();
        assert!(
            !router
                .update(Input {
                    r1: true,
                    ..input()
                })
                .drive_enabled
        );
        router.update(input());
        assert!(
            router
                .update(Input {
                    r1: true,
                    ..input()
                })
                .drive_enabled
        );
        router.update(Input {
            connected: false,
            ..input()
        });
        assert!(
            !router
                .update(Input {
                    r1: true,
                    ..input()
                })
                .drive_enabled
        );
        router.update(input());
        router.require_release();
        assert!(
            !router
                .update(Input {
                    r1: true,
                    ..input()
                })
                .drive_enabled
        );
        router.update(Input {
            focused: false,
            ..input()
        });
        assert!(
            !router
                .update(Input {
                    r1: true,
                    ..input()
                })
                .drive_enabled
        );
    }

    #[test]
    fn navigation_is_normal_only_and_wraps() {
        let mut router = Router::default();
        let nav = Input {
            next_pressed: true,
            follow_pressed: true,
            ..input()
        };
        assert_eq!(router.update(nav).select_step, 1);
        assert!(router.update(nav).toggle_follow);
        assert_eq!(router.update(Input { r1: true, ..nav }).select_step, 0);
        assert_eq!(cycle_index(Some(2), 3, 1), Some(0));
        assert_eq!(cycle_index(Some(0), 3, -1), Some(2));
        assert_eq!(cycle_index(None, 0, 1), None);
    }

    #[test]
    fn triggers_lift_only_in_normal_layer() {
        let mut router = Router::default();
        let neutral = Input {
            connected: true,
            focused: true,
            ..Input::default()
        };
        let up = router.update(Input { r2: 1.0, ..neutral });
        assert_eq!(up.layer, Layer::Camera);
        assert_eq!(up.camera.lift, 1.0);
        assert!(up.camera.active());
        assert_eq!(up.camera.for_view(true).lift, 1.0);
        assert_eq!(
            router.update(Input { l2: 1.0, ..neutral }).camera.lift,
            -1.0
        );
        assert_eq!(
            router
                .update(Input {
                    r2: 0.56,
                    ..neutral
                })
                .camera
                .lift,
            0.5
        );
        assert_eq!(
            router
                .update(Input {
                    r2: 1.0,
                    l2: 1.0,
                    ..neutral
                })
                .camera
                .lift,
            0.0
        );
        assert_eq!(
            router
                .update(Input {
                    r2: f32::NAN,
                    l2: -1.0,
                    ..neutral
                })
                .camera
                .lift,
            0.0
        );
        let drive = router.update(Input {
            r1: true,
            r2: 1.0,
            ..neutral
        });
        assert_eq!(drive.layer, Layer::Vehicle);
        assert!(drive.drive_enabled);
        assert!(!drive.camera.active());
        let machine = router.update(Input {
            l1: true,
            r2: 1.0,
            ..neutral
        });
        assert_eq!(machine.layer, Layer::Machine);
        assert!(!machine.camera.active());
    }
}
