use bevy::{prelude::*, render::render_resource::ShaderType};
use gearbox_api::wire::draw_data::{AgeClock, CaptureStamp, ColorMode};

const SECOND: i128 = 1_000_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct AgeUniform {
    pub anchor: UVec4,
    pub mapping: Vec4,
    pub palette: UVec4,
}

#[derive(Clone, Copy, Debug, Default)]
struct Pair {
    hi: f64,
    lo: f64,
}

impl Pair {
    fn difference(a: f64, b: f64) -> Self {
        let hi = a - b;
        let virtual_b = a - hi;
        Self {
            hi,
            lo: (a - (hi + virtual_b)) + (virtual_b - b),
        }
    }

    fn add(self, rhs: Self) -> Self {
        let sum = Self::difference(self.hi, -rhs.hi);
        let low = self.lo + rhs.lo + sum.lo;
        Self::difference(sum.hi, -low)
    }

    fn negative(self) -> bool {
        self.hi < 0.0 || self.hi == 0.0 && self.lo < 0.0
    }
    fn zero(self) -> bool {
        self.hi == 0.0 && self.lo == 0.0
    }
    fn value(self) -> f64 {
        self.hi + self.lo
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Nanoseconds {
    whole: i128,
    fraction: Pair,
}

impl Nanoseconds {
    fn seconds(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        let bits = value.to_bits();
        let biased = ((bits >> 52) & 2047) as i32;
        let mantissa = (bits & ((1 << 52) - 1)) | if biased == 0 { 0 } else { 1 << 52 };
        let exponent = if biased == 0 {
            -1074
        } else {
            biased - 1023 - 52
        };
        let mantissa = u128::from(mantissa) * SECOND as u128;
        let (integer, remainder) = if exponent >= 0 {
            (
                mantissa.checked_mul(1u128.checked_shl(exponent as u32)?)?,
                0,
            )
        } else if exponent <= -128 {
            (0, mantissa)
        } else {
            (mantissa >> -exponent, mantissa & ((1u128 << -exponent) - 1))
        };
        let sign = if value.is_sign_negative() { -1.0 } else { 1.0 };
        let half = exponent.div_euclid(2);
        let fraction = if remainder == 0 {
            0.0
        } else {
            ((remainder as f64 * 2.0f64.powi(half)) * 2.0f64.powi(exponent - half))
                .min(f64::from_bits(1.0f64.to_bits() - 1))
                * sign
        };
        let whole = i128::try_from(integer).ok()? * if sign < 0.0 { -1 } else { 1 };
        Some(Self {
            whole,
            fraction: Pair {
                hi: fraction,
                lo: 0.0,
            },
        })
    }

    fn pair(value: Pair) -> Option<Self> {
        Self::seconds(value.hi)?.add(Self::seconds(value.lo)?)
    }

    fn add(self, rhs: Self) -> Option<Self> {
        let fraction = self.fraction.add(rhs.fraction);
        let integer = fraction.hi.trunc();
        Some(Self {
            whole: self
                .whole
                .checked_add(rhs.whole)?
                .checked_add(integer as i128)?,
            fraction: Pair {
                hi: fraction.hi - integer,
                lo: fraction.lo,
            },
        })
    }

    fn negate(self) -> Self {
        Self {
            whole: -self.whole,
            fraction: Pair {
                hi: -self.fraction.hi,
                lo: -self.fraction.lo,
            },
        }
    }

    fn palette(self) -> UVec4 {
        let shifted = self.whole.rem_euclid(SECOND) + SECOND / 2;
        let below = i128::from(self.fraction.negative() && shifted.rem_euclid(SECOND) == 0);
        let quotient = self.whole.div_euclid(SECOND) + shifted.div_euclid(SECOND) - below;
        let remainder =
            (shifted.rem_euclid(SECOND) - i128::from(self.fraction.negative())).rem_euclid(SECOND);
        let signed = quotient.clamp(-(1 << 60), 1 << 60) as i64 as u64;
        UVec4::new(
            signed as u32,
            (signed >> 32) as u32,
            remainder as u32,
            quotient.rem_euclid(8) as u32 | (u32::from(self.fraction.zero()) << 3),
        )
    }
}

impl AgeUniform {
    pub fn new(
        color: &ColorMode,
        capture: Option<&CaptureStamp>,
        now_s: f64,
        source_ns: u64,
    ) -> Self {
        let ColorMode::Age { clock, range, .. } = color else {
            return Self::default();
        };
        let Some(capture) = capture else {
            return Self::default();
        };
        let (approximate, difference, palette) = match clock {
            AgeClock::Source => {
                let reference = Nanoseconds {
                    whole: i128::from(source_ns)
                        - i128::from(capture.source_time_ns.unwrap_or(source_ns)),
                    ..Default::default()
                };
                let difference =
                    Nanoseconds::seconds(range[0]).and_then(|low| reference.add(low.negate()));
                (
                    reference.whole as f64 / SECOND as f64,
                    difference,
                    reference.palette(),
                )
            }
            AgeClock::Simulation => {
                let elapsed = Pair::difference(now_s, capture.simulation_time_s);
                let reference = Nanoseconds::pair(elapsed);
                let difference = Nanoseconds::pair(elapsed.add(Pair {
                    hi: -range[0],
                    lo: 0.0,
                }));
                let palette = if let Some(reference) = reference {
                    reference.palette()
                } else {
                    let residue = Pair::difference(
                        now_s.rem_euclid(8.0),
                        capture.simulation_time_s.rem_euclid(8.0),
                    );
                    let mut palette = Nanoseconds::pair(residue).unwrap_or_default().palette();
                    let signed = if elapsed.negative() {
                        -(1i64 << 60)
                    } else {
                        1 << 60
                    } as u64;
                    palette.x = signed as u32;
                    palette.y = (signed >> 32) as u32;
                    palette
                };
                (elapsed.value(), difference, palette)
            }
        };
        let mut result = Self {
            palette,
            ..Default::default()
        };
        if range[0] == range[1] {
            result.mapping.x = 0.5;
            return result;
        }
        let width = (range[1] - range[0]) * SECOND as f64;
        let Some(difference) = difference.filter(|_| width.is_finite()) else {
            result.mapping.x = super::appearance::display_coordinate(approximate, *range);
            return result;
        };
        if difference.whole < 0 || difference.whole == 0 && difference.fraction.negative() {
            return result;
        }
        let floor = difference.whole - i128::from(difference.fraction.negative());
        let anchor = floor.clamp(0, i128::from(u64::MAX)) as u64;
        let residual = (difference.whole - i128::from(anchor)) as f64 + difference.fraction.value();
        let value = residual / width;
        if anchor == u64::MAX && value >= 1.0 {
            result.mapping.x = 1.0;
            return result;
        }
        result.anchor = UVec4::new(
            anchor as u32,
            (anchor >> 32) as u32,
            if width < 1.0 { 2 } else { 1 },
            0,
        );
        result.mapping = Vec4::new(
            value.clamp(0.0, 1.0) as f32,
            if width < 1.0 {
                0.0
            } else {
                (1.0 / width) as f32
            },
            0.0,
            0.0,
        );
        result
    }
}

#[cfg(test)]
#[path = "age_tests.rs"]
mod tests;
