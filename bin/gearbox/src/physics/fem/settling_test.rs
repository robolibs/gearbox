use molla_math::Vec3;

const REQUIRED_SECONDS: f64 = 0.050;
const MAX_SAMPLE_SECONDS: f64 = 1.0 / 300.0 + 1e-9;

#[derive(Clone, Copy)]
pub(super) struct Sample {
    pub time: f64,
    pub momentum: Vec3,
    pub contact_impulse: Vec3,
    pub max_particle_speed: f64,
    pub max_body_speed: f64,
    pub max_body_angular_speed: f64,
}

impl Sample {
    fn quiet(self, mass: f64) -> bool {
        self.time.is_finite()
            && self.momentum.is_finite()
            && self.contact_impulse.is_finite()
            && self.momentum.length() / mass <= 0.005
            && (0.0..=0.010).contains(&self.max_particle_speed)
            && (0.0..=0.010).contains(&self.max_body_speed)
            && (0.0..=0.050).contains(&self.max_body_angular_speed)
    }
}

#[derive(Debug, serde::Serialize)]
pub(super) struct Report {
    pub time: f64,
    pub com_speed: f64,
    pub max_particle_speed: f64,
    pub max_body_speed: f64,
    pub max_body_angular_speed: f64,
    pub force_residual_fraction: Option<f64>,
    pub quiet_load_balanced_seconds: f64,
    pub sustained: bool,
}

pub(super) struct Monitor {
    mass: f64,
    weight: Vec3,
    previous: Option<Sample>,
    quiet_seconds: f64,
}

impl Monitor {
    pub fn new(mass: f64, gravity: Vec3) -> Self {
        assert!(mass.is_finite() && mass > 0.0);
        assert!(gravity.is_finite() && gravity.length() > 0.0);
        Self {
            mass,
            weight: gravity * mass,
            previous: None,
            quiet_seconds: 0.0,
        }
    }

    pub fn observe(&mut self, sample: Sample) -> Report {
        let mut residual = None;
        let mut interval = 0.0;
        if let Some(previous) = self.previous {
            let dt = sample.time - previous.time;
            if dt.is_finite() && dt > 0.0 && dt <= MAX_SAMPLE_SECONDS {
                let force = (sample.contact_impulse - previous.contact_impulse) / dt;
                let fraction = (force + self.weight).length() / self.weight.length();
                if fraction.is_finite() {
                    residual = Some(fraction);
                    if fraction <= 0.05 && previous.quiet(self.mass) && sample.quiet(self.mass) {
                        interval = dt;
                    }
                }
            }
        }
        self.quiet_seconds = if interval > 0.0 {
            self.quiet_seconds + interval
        } else {
            0.0
        };
        self.previous = Some(sample);
        Report {
            time: sample.time,
            com_speed: sample.momentum.length() / self.mass,
            max_particle_speed: sample.max_particle_speed,
            max_body_speed: sample.max_body_speed,
            max_body_angular_speed: sample.max_body_angular_speed,
            force_residual_fraction: residual,
            quiet_load_balanced_seconds: self.quiet_seconds,
            sustained: self.quiet_seconds + 1e-12 >= REQUIRED_SECONDS,
        }
    }
}

fn static_sample(time: f64) -> Sample {
    Sample {
        time,
        momentum: Vec3::ZERO,
        contact_impulse: Vec3::Y * (750.0 * 9.81 * time),
        max_particle_speed: 0.0,
        max_body_speed: 0.0,
        max_body_angular_speed: 0.0,
    }
}

#[test]
fn settling_requires_a_complete_quiet_load_balanced_window() {
    let mut monitor = Monitor::new(750.0, Vec3::NEG_Y * 9.81);
    for i in 0..=15 {
        let report = monitor.observe(static_sample(i as f64 / 300.0));
        assert_eq!(report.sustained, i == 15);
    }
}

#[test]
fn settling_rejects_motion_despite_zero_net_momentum() {
    for field in 0..4 {
        let mut monitor = Monitor::new(750.0, Vec3::NEG_Y * 9.81);
        for i in 0..=20 {
            let mut sample = static_sample(i as f64 / 300.0);
            match field {
                0 => sample.max_particle_speed = 0.011,
                1 => sample.max_body_speed = 0.011,
                2 => sample.max_body_angular_speed = 0.051,
                _ => sample.momentum = Vec3::Y * (750.0 * 0.006),
            }
            assert!(!monitor.observe(sample).sustained);
        }
    }
}

#[test]
fn settling_rejects_freefall_and_oscillating_support() {
    for oscillating in [false, true] {
        let mut monitor = Monitor::new(750.0, Vec3::NEG_Y * 9.81);
        for i in 0..=20 {
            let mut sample = static_sample(i as f64 / 300.0);
            sample.contact_impulse = if oscillating {
                Vec3::Y * (750.0 * 9.81 * (2 * (i / 2)) as f64 / 300.0)
            } else {
                Vec3::ZERO
            };
            assert!(!monitor.observe(sample).sustained);
        }
    }
}

#[test]
fn settling_resets_on_missing_nonfinite_or_reversed_samples() {
    for failure in 0..5 {
        let mut monitor = Monitor::new(750.0, Vec3::NEG_Y * 9.81);
        for i in 0..=15 {
            monitor.observe(static_sample(i as f64 / 300.0));
        }
        let mut sample = static_sample(16.0 / 300.0);
        match failure {
            0 => sample.time = 18.0 / 300.0,
            1 => sample.time = 14.0 / 300.0,
            2 => sample.max_particle_speed = f64::NAN,
            3 => sample.contact_impulse = Vec3::splat(f64::NAN),
            _ => sample.time = 15.0 / 300.0,
        }
        let report = monitor.observe(sample);
        assert!(!report.sustained);
        assert_eq!(report.quiet_load_balanced_seconds, 0.0);
    }
}
