use bevy::render::render_resource::{
    BlendState, CompareFunction, DepthStencilState, MultisampleState, TextureFormat,
};

use super::{
    appearance::{CATEGORY_VALID, COLOR_VALID, Sample, VALUE_VALID},
    style::Style,
};

pub(super) const OPAQUE: u8 = 1;
pub(super) const TRANSMITTING: u8 = 2;

fn mode(alpha: f32) -> u8 {
    if alpha <= 0.0 {
        0
    } else if alpha >= 1.0 {
        OPAQUE
    } else {
        TRANSMITTING
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Summary {
    pub rgba: u8,
    pub valid_value: bool,
    pub missing_value: bool,
    pub valid_category: bool,
    pub missing_category: bool,
}

impl Summary {
    #[cfg(test)]
    pub fn new(samples: &[Sample]) -> Self {
        let mut result = Self::default();
        for sample in samples {
            result.add(sample);
        }
        result
    }

    pub fn add(&mut self, sample: &Sample) {
        if sample.flags & COLOR_VALID != 0 {
            self.rgba |= mode((sample.rgba >> 24) as f32 / 255.0);
        }
        self.valid_value |= sample.flags & VALUE_VALID != 0;
        self.missing_value |= sample.flags & VALUE_VALID == 0;
        self.valid_category |= sample.flags & CATEGORY_VALID != 0;
        self.missing_category |= sample.flags & CATEGORY_VALID == 0;
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct AlphaStyle {
    kind: u32,
    valid: u8,
    missing: u8,
    opacity: f32,
}

impl AlphaStyle {
    pub fn new(style: &Style) -> Self {
        let missing = mode(style.missing.w);
        let valid = match style.controls.x {
            0 => mode(style.constant.w),
            1 => 0,
            3 => style.categories[..style.controls.z as usize]
                .iter()
                .fold(missing, |modes, entry| {
                    modes | mode((entry.y >> 24) as f32 / 255.0)
                }),
            _ => {
                let mut modes = style
                    .ramp
                    .iter()
                    .fold(0, |modes, color| modes | mode(color.w));
                if style.controls.y == 0 && style.ramp.windows(2).any(|pair| pair[0].w != pair[1].w)
                {
                    modes |= TRANSMITTING;
                }
                modes
            }
        };
        Self {
            kind: style.controls.x,
            valid,
            missing,
            opacity: style.effects.x,
        }
    }

    pub fn required(self, summary: Summary, samples: u32) -> u8 {
        let mut modes = match self.kind {
            0 => self.valid,
            1 => summary.rgba,
            3 => {
                (if summary.valid_category {
                    self.valid
                } else {
                    0
                }) | (if summary.missing_category {
                    self.missing
                } else {
                    0
                })
            }
            _ => {
                (if summary.valid_value { self.valid } else { 0 })
                    | (if summary.missing_value {
                        self.missing
                    } else {
                        0
                    })
            }
        };
        if self.opacity == 0.0 {
            modes = 0;
        } else if self.opacity < 1.0 && modes != 0 {
            modes = TRANSMITTING;
        }
        if samples == 1 && modes & OPAQUE != 0 {
            modes |= TRANSMITTING;
        }
        if modes == 0 { OPAQUE } else { modes }
    }
}

#[derive(Clone, Copy)]
pub(super) struct RasterState {
    pub opaque: bool,
    pub samples: u32,
}

impl RasterState {
    pub fn blend(self) -> Option<BlendState> {
        if self.opaque {
            None
        } else {
            Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING)
        }
    }
    pub fn multisample(self) -> MultisampleState {
        MultisampleState {
            count: self.samples,
            mask: !0,
            alpha_to_coverage_enabled: self.opaque && self.samples > 1,
        }
    }
    pub fn depth(self, format: TextureFormat) -> DepthStencilState {
        DepthStencilState {
            format,
            depth_write_enabled: Some(self.opaque),
            depth_compare: Some(CompareFunction::GreaterEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }
    }
}

#[cfg(test)]
#[path = "alpha_tests.rs"]
mod tests;
