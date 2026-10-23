//! Stage unit factors and the dimensional conversions of every physical
//! quantity the compiler normalizes. L is metres per stage unit, M kilograms
//! per stage mass unit; angles are authored in degrees.

use openusd::sdf::Value;
use openusd::usd::Stage;

use super::diagnostics::{Code, Diagnostic};
use super::model::{DriveType, Provenance, StageUnits, UpAxis, Valued};

/// UsdGeom's fallback when a stage authors no `metersPerUnit`: centimetres.
pub const SCHEMA_METERS_PER_UNIT: f64 = 0.01;
pub const SCHEMA_KILOGRAMS_PER_UNIT: f64 = 1.0;

/// Factor from an authored angular acceleration-drive gain to SI. The gain
/// is authored per degree of error (or degree/s), so it scales by 180/π;
/// mass and length do not enter.
pub const ACCELERATION_ANGULAR_GAIN_FACTOR: f64 = 180.0 / std::f64::consts::PI;

fn scalar(value: Option<Value>) -> Option<f64> {
    match value? {
        Value::Double(d) => Some(d),
        Value::Float(f) => Some(f as f64),
        Value::Int(i) => Some(i as f64),
        Value::Int64(i) => Some(i as f64),
        _ => None,
    }
}

pub fn read_stage_units(stage: &Stage, meters_override: Option<f64>, diagnostics: &mut Vec<Diagnostic>) -> StageUnits {
    let mpu = match (meters_override, scalar(stage.stage_metadata("metersPerUnit").ok().flatten())) {
        (Some(v), _) => Valued::new(v, Provenance::ProfileDefault),
        (None, Some(v)) => Valued::authored(v),
        (None, None) => Valued::schema(SCHEMA_METERS_PER_UNIT),
    };
    let kpu = scalar(stage.stage_metadata("kilogramsPerUnit").ok().flatten())
        .map(Valued::authored)
        .unwrap_or_else(|| Valued::schema(SCHEMA_KILOGRAMS_PER_UNIT));
    let up = match stage.stage_metadata("upAxis").ok().flatten() {
        Some(Value::Token(t)) => Some(t.as_str().to_string()),
        Some(Value::String(s)) => Some(s),
        _ => None,
    };
    let up_axis = match up.as_deref() {
        Some("Z") => Valued::authored(UpAxis::Z),
        Some("Y") => Valued::authored(UpAxis::Y),
        Some(other) => {
            diagnostics.push(Diagnostic::error(Code::InvalidUnits, None, format!("upAxis `{other}` is neither Y nor Z")));
            Valued::authored(UpAxis::Y)
        }
        None => Valued::schema(UpAxis::Y),
    };
    for (name, value) in [("metersPerUnit", mpu.value), ("kilogramsPerUnit", kpu.value)] {
        if !(value.is_finite() && value > 0.0) {
            diagnostics.push(Diagnostic::error(Code::InvalidUnits, None, format!("{name} = {value} must be finite and positive")));
        }
    }
    StageUnits { meters_per_unit: mpu, kilograms_per_unit: kpu, up_axis }
}

impl StageUnits {
    pub fn si() -> Self {
        Self {
            meters_per_unit: Valued::authored(1.0),
            kilograms_per_unit: Valued::authored(1.0),
            up_axis: Valued::authored(UpAxis::Z),
        }
    }

    pub fn l(&self) -> f64 {
        self.meters_per_unit.value
    }

    pub fn m(&self) -> f64 {
        self.kilograms_per_unit.value
    }

    pub fn length(&self, v: f64) -> f64 {
        v * self.l()
    }

    pub fn mass(&self, v: f64) -> f64 {
        v * self.m()
    }

    pub fn inertia(&self, v: f64) -> f64 {
        v * self.m() * self.l() * self.l()
    }

    pub fn density(&self, v: f64) -> f64 {
        v * self.m() / self.l().powi(3)
    }

    /// Joint coordinate, target, limit or joint state.
    pub fn coordinate(&self, rotational: bool, v: f64) -> f64 {
        if rotational { v.to_radians() } else { self.length(v) }
    }

    /// Force (linear) or torque (angular): drive caps and break limits.
    pub fn effort(&self, rotational: bool, v: f64) -> f64 {
        if rotational { v * self.m() * self.l() * self.l() } else { v * self.m() * self.l() }
    }

    /// Drive stiffness or damping. Force drives: linear gains are mass per
    /// s² (or s), so × M; angular gains are torque per degree, so
    /// × M·L²·180/π. Acceleration drives act per unit of joint inertia, so
    /// mass and length cancel; angular ones are still per degree.
    pub fn drive_gain(&self, rotational: bool, drive_type: DriveType, v: f64) -> f64 {
        match (drive_type, rotational) {
            (DriveType::Force, false) => v * self.m(),
            (DriveType::Force, true) => v * self.m() * self.l() * self.l() * 180.0 / std::f64::consts::PI,
            (DriveType::Acceleration, false) => v,
            (DriveType::Acceleration, true) => v * ACCELERATION_ANGULAR_GAIN_FACTOR,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(l: f64, m: f64) -> StageUnits {
        StageUnits {
            meters_per_unit: Valued::authored(l),
            kilograms_per_unit: Valued::authored(m),
            up_axis: Valued::authored(UpAxis::Z),
        }
    }

    /// A 2 N·m/rad spring authored in cm and grams gives the same SI gain
    /// as the same spring authored in m and kg.
    #[test]
    fn force_drive_gains_follow_their_dimensions() {
        let si = units(1.0, 1.0);
        let cm_g = units(0.01, 0.001);
        let per_degree_si = 2.0 * std::f64::consts::PI / 180.0;
        let per_degree_cm_g = per_degree_si / (0.001 * 0.01 * 0.01);
        assert!((si.drive_gain(true, DriveType::Force, per_degree_si) - 2.0).abs() < 1e-12);
        assert!((cm_g.drive_gain(true, DriveType::Force, per_degree_cm_g) - 2.0).abs() < 1e-9);
        assert!((cm_g.drive_gain(false, DriveType::Force, 500.0 / 0.001) - 500.0).abs() < 1e-9);
        assert!((cm_g.effort(false, 10.0 / (0.001 * 0.01)) - 10.0).abs() < 1e-9);
        assert!((cm_g.effort(true, 10.0 / (0.001 * 0.01 * 0.01)) - 10.0).abs() < 1e-9);
        assert!((cm_g.inertia(1.0 / (0.001 * 0.01 * 0.01)) - 1.0).abs() < 1e-9);
        assert!((cm_g.coordinate(false, 150.0) - 1.5).abs() < 1e-12);
        assert!((cm_g.coordinate(true, 180.0) - std::f64::consts::PI).abs() < 1e-12);
    }

    #[test]
    fn acceleration_drive_gains_ignore_mass_and_length() {
        let cm_g = units(0.01, 0.001);
        assert_eq!(cm_g.drive_gain(false, DriveType::Acceleration, 40.0), 40.0);
        assert!((cm_g.drive_gain(true, DriveType::Acceleration, 1.0) - 57.29577951308232).abs() < 1e-9);
        assert_eq!(cm_g.drive_gain(true, DriveType::Acceleration, 40.0), 40.0 * ACCELERATION_ANGULAR_GAIN_FACTOR);
    }
}
