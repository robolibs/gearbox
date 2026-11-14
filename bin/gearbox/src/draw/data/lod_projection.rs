use bevy::math::{DMat4, DVec3};

#[cfg(test)]
#[path = "lod_projection_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::draw::data) struct View {
    pub clip_from_world: DMat4,
    pub viewport: [u32; 2],
    pub error_px: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::draw::data) struct Glyph {
    pub metres: f64,
    pub pixels: f64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Projection {
    pub visible: bool,
    pub rect: [f64; 4],
}

impl Projection {
    pub fn empty() -> Self {
        Self {
            visible: false,
            rect: [0.0; 4],
        }
    }

    fn unbounded() -> Self {
        Self {
            visible: true,
            rect: [
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::INFINITY,
            ],
        }
    }

    pub fn error(self) -> f64 {
        if self.visible {
            (self.rect[2] - self.rect[0]).hypot(self.rect[3] - self.rect[1])
        } else {
            0.0
        }
    }

}

impl View {
    pub(super) fn sampling_error(
        self,
        world_from_local: DMat4,
        rebase_m: [f64; 3],
        bounds: [[f64; 3]; 2],
        position_error_m: f64,
        sample_error_m: f64,
    ) -> f64 {
        let slope = self.sampling_slope(self.clip_from_world * world_from_local, rebase_m, bounds, position_error_m);
        sampled(slope, position_error_m, sample_error_m)
    }

    /// Screen pixels per metre of sample displacement inside `bounds`, for `clip_from_local`.
    pub(super) fn sampling_slope(
        self,
        matrix: DMat4,
        rebase_m: [f64; 3],
        bounds: [[f64; 3]; 2],
        position_error_m: f64,
    ) -> f64 {
        let mut minimum_w = f64::INFINITY;
        let mut maximum_xy = [0.0f64; 2];
        for corner in 0..8 {
            let local = std::array::from_fn(|axis| {
                let side = (corner >> axis) & 1;
                bounds[side][axis] - rebase_m[axis]
                    + if side == 0 {
                        -position_error_m
                    } else {
                        position_error_m
                    }
            });
            let clip = matrix * DVec3::from_array(local).extend(1.0);
            if !clip.is_finite() || clip.w <= 0.0 || clip.z < 0.0 || clip.z > clip.w {
                return f64::INFINITY;
            }
            minimum_w = minimum_w.min(clip.w.next_down());
            maximum_xy[0] = maximum_xy[0].max(clip.x.abs().next_up());
            maximum_xy[1] = maximum_xy[1].max(clip.y.abs().next_up());
        }
        if minimum_w <= 0.0 {
            return f64::INFINITY;
        }
        let norm_bound = |row: usize| {
            let values = matrix.row(row).truncate().abs().to_array();
            let scale = values.into_iter().fold(0.0f64, f64::max);
            if scale == 0.0 {
                return 0.0;
            }
            let squared = values.into_iter().fold(0.0f64, |sum, value| {
                let normalized = (value / scale).next_up();
                (sum + (normalized * normalized).next_up()).next_up()
            });
            (squared.sqrt().next_up() * scale).next_up()
        };
        let w_gradient = norm_bound(3);
        let slope: [f64; 2] = std::array::from_fn(|axis| {
            let direct = (norm_bound(axis) / minimum_w).next_up();
            let perspective = (((maximum_xy[axis] / minimum_w).next_up() * w_gradient).next_up()
                / minimum_w)
                .next_up();
            ((direct + perspective).next_up() * f64::from(self.viewport[axis]) * 0.5).next_up()
        });
        slope[0].hypot(slope[1]).next_up()
    }

    pub(super) fn project(
        self,
        world_from_local: DMat4,
        rebase_m: [f64; 3],
        bounds: [[f64; 3]; 2],
        error_m: f64,
        glyph: Glyph,
    ) -> Projection {
        self.project_local(self.clip_from_world * world_from_local, world_from_local, rebase_m, bounds, error_m, glyph)
    }

    /// [`Self::project`] with `clip_from_world * world_from_local` already formed.
    pub(super) fn project_local(
        self,
        matrix: DMat4,
        world_from_local: DMat4,
        rebase_m: [f64; 3],
        bounds: [[f64; 3]; 2],
        error_m: f64,
        glyph: Glyph,
    ) -> Projection {
        let half = [
            f64::from(self.viewport[0]) * 0.5,
            f64::from(self.viewport[1]) * 0.5,
        ];
        let margin = [
            1.0 + (glyph.pixels + 1.0) / half[0],
            1.0 + (glyph.pixels + 1.0) / half[1],
        ];
        let mut world_bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
        let mut depth_outside = 7u8;
        for corner in 0..8 {
            let local = std::array::from_fn(|axis| {
                let side = (corner >> axis) & 1;
                bounds[side][axis] - rebase_m[axis] + if side == 0 { -error_m } else { error_m }
            });
            let world = world_from_local * DVec3::from_array(local).extend(1.0);
            let center = self.clip_from_world * world;
            if !center.is_finite() {
                return Projection::unbounded();
            }
            depth_outside &= u8::from(center.z < 0.0)
                | (u8::from(center.z > center.w) << 1)
                | (u8::from(center.w <= 0.0) << 2);
            if !world.is_finite() {
                return Projection::unbounded();
            }
            for axis in 0..3 {
                world_bounds[0][axis] = world_bounds[0][axis].min(world[axis].next_down());
                world_bounds[1][axis] = world_bounds[1][axis].max(world[axis].next_up());
            }
        }
        if depth_outside != 0 {
            return Projection::empty();
        }
        let mut common = 15u8;
        for corner in 0..8 {
            let world = std::array::from_fn(|axis| {
                let side = (corner >> axis) & 1;
                world_bounds[side][axis]
                    + if side == 0 {
                        -glyph.metres
                    } else {
                        glyph.metres
                    }
            });
            let clip = self.clip_from_world * DVec3::from_array(world).extend(1.0);
            if !clip.is_finite() {
                return Projection::unbounded();
            }
            let outside = u8::from(clip.x < -clip.w * margin[0])
                | (u8::from(clip.x > clip.w * margin[0]) << 1)
                | (u8::from(clip.y < -clip.w * margin[1]) << 2)
                | (u8::from(clip.y > clip.w * margin[1]) << 3);
            common &= outside;
        }
        if common != 0 {
            return Projection::empty();
        }
        let mut rect = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for corner in 0..8 {
            let local = std::array::from_fn(|axis| {
                let side = (corner >> axis) & 1;
                bounds[side][axis] - rebase_m[axis] + if side == 0 { -error_m } else { error_m }
            });
            let clip = matrix * DVec3::from_array(local).extend(1.0);
            if !clip.is_finite() || clip.w <= 0.0 || clip.z < 0.0 || clip.z > clip.w {
                return Projection::unbounded();
            }
            let xy = [clip.x / clip.w * half[0], clip.y / clip.w * half[1]];
            if xy.iter().any(|v| !v.is_finite()) {
                return Projection::unbounded();
            }
            for axis in 0..2 {
                rect[axis] = rect[axis].min(xy[axis].next_down());
                rect[axis + 2] = rect[axis + 2].max(xy[axis].next_up());
            }
        }
        Projection {
            visible: true,
            rect,
        }
    }
}

/// The screen error of samples `sample_error_m` apart under a sampling `slope`.
pub(super) fn sampled(slope: f64, position_error_m: f64, sample_error_m: f64) -> f64 {
    let distance = (sample_error_m + 2.0 * position_error_m).next_up();
    let error = (slope * distance).next_up();
    if error.is_finite() {
        error
    } else {
        f64::INFINITY
    }
}
