use std::{str::FromStr, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Body {
    #[default]
    Earth = 0,
    Mars = 1,
    Moon = 2,
}

impl FromStr for Body {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "earth" => Ok(Self::Earth),
            "mars" => Ok(Self::Mars),
            "moon" => Ok(Self::Moon),
            _ => Err(format!("unknown planet {value:?}; choose earth, mars or moon")),
        }
    }
}

impl Body {
    pub fn active() -> Self {
        static BODY: OnceLock<Body> = OnceLock::new();
        *BODY.get_or_init(|| Self::from_env().expect("valid GEARBOX_PLANET"))
    }

    pub fn from_env() -> Result<Self, String> {
        match std::env::var("GEARBOX_PLANET") {
            Ok(value) => value.parse(),
            Err(std::env::VarError::NotPresent) => Ok(Self::Earth),
            Err(error) => Err(error.to_string()),
        }
    }

    pub const fn name(self) -> &'static str {
        match self { Self::Earth => "earth", Self::Mars => "mars", Self::Moon => "moon" }
    }

    pub const fn radius_m(self) -> f64 {
        match self { Self::Earth => 6_371_000.0, Self::Mars => 3_389_500.0, Self::Moon => 1_737_400.0 }
    }

    pub const fn gravity_mps2(self) -> f64 {
        match self { Self::Earth => 9.81, Self::Mars => 3.711, Self::Moon => 1.62 }
    }

    pub const fn home(self) -> (f64, f64) {
        match self {
            Self::Earth => (52.370216, 4.895168),
            Self::Mars => (18.4447, 77.4508),
            Self::Moon => (0.67416, 23.47314),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_choices_and_default() {
        assert_eq!(Body::default(), Body::Earth);
        for body in [Body::Earth, Body::Mars, Body::Moon] {
            assert_eq!(body.name().parse::<Body>().unwrap(), body);
            assert!(body.radius_m() > 1_000_000.0);
        }
        assert!("marz".parse::<Body>().is_err());
        assert!(Body::Moon.gravity_mps2() < Body::Mars.gravity_mps2());
        assert!(Body::Mars.gravity_mps2() < Body::Earth.gravity_mps2());
    }
}
