//! Regular height samples of the ground a field covers.

use bevy::prelude::*;

/// Regular height samples over a square centred on the origin.
pub struct HeightGrid {
    pub min_x: f32,
    pub min_z: f32,
    pub cell: f32,
    pub cols: usize,
    pub rows: usize,
    heights: Vec<f32>,
}

impl HeightGrid {
    pub fn sample(size: f32, cell: f32, height: impl Fn(f32, f32) -> f32) -> Self {
        Self::sample_at(Vec2::ZERO, size, cell, height)
    }

    /// A square grid `size` metres wide centred on `center`.
    pub fn sample_at(center: Vec2, size: f32, cell: f32, height: impl Fn(f32, f32) -> f32) -> Self {
        let cols = (size / cell).ceil().max(1.0) as usize + 1;
        let rows = cols;
        let cell = size / (cols - 1) as f32;
        let min_x = center.x - size * 0.5;
        let min_z = center.y - size * 0.5;
        let mut heights = Vec::with_capacity(cols * rows);
        for i in 0..rows {
            let z = min_z + i as f32 * cell;
            for j in 0..cols {
                let x = min_x + j as f32 * cell;
                heights.push(height(x, z));
            }
        }
        Self {
            min_x,
            min_z,
            cell,
            cols,
            rows,
            heights,
        }
    }

    pub fn at(&self, i: usize, j: usize) -> f32 {
        self.heights[i * self.cols + j]
    }

    /// Bilinear height, `None` outside the grid.
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let fx = (x - self.min_x) / self.cell;
        let fz = (z - self.min_z) / self.cell;
        if fx < 0.0 || fz < 0.0 {
            return None;
        }
        let j = (fx.floor() as usize).min(self.cols - 2);
        let i = (fz.floor() as usize).min(self.rows - 2);
        if fx > (self.cols - 1) as f32 || fz > (self.rows - 1) as f32 {
            return None;
        }
        let tx = (fx - j as f32).clamp(0.0, 1.0);
        let tz = (fz - i as f32).clamp(0.0, 1.0);
        let h00 = self.at(i, j);
        let h10 = self.at(i, j + 1);
        let h01 = self.at(i + 1, j);
        let h11 = self.at(i + 1, j + 1);
        let h0 = h00 + (h10 - h00) * tx;
        let h1 = h01 + (h11 - h01) * tx;
        Some(h0 + (h1 - h0) * tz)
    }

    pub fn normal_at(&self, x: f32, z: f32) -> Vec3 {
        let d = self.cell;
        let max_x = self.min_x + (self.cols - 1) as f32 * d;
        let max_z = self.min_z + (self.rows - 1) as f32 * d;
        let x = x.clamp(self.min_x, max_x);
        let z = z.clamp(self.min_z, max_z);
        let xl = (x - d).max(self.min_x);
        let xr = (x + d).min(max_x);
        let zl = (z - d).max(self.min_z);
        let zr = (z + d).min(max_z);
        let h = |x: f32, z: f32| {
            self.height_at(x, z)
                .expect("normal sample inside height grid")
        };
        let dx = (h(xr, z) - h(xl, z)) / (xr - xl);
        let dz = (h(x, zr) - h(x, zl)) / (zr - zl);
        Vec3::new(-dx, 1.0, -dz).normalize()
    }

    pub fn normal_at_index(&self, i: usize, j: usize) -> Vec3 {
        let jl = j.saturating_sub(1);
        let jr = (j + 1).min(self.cols - 1);
        let iu = i.saturating_sub(1);
        let id = (i + 1).min(self.rows - 1);
        let dx = (self.at(i, jr) - self.at(i, jl)) / ((jr - jl) as f32 * self.cell);
        let dz = (self.at(id, j) - self.at(iu, j)) / ((id - iu) as f32 * self.cell);
        Vec3::new(-dx, 1.0, -dz).normalize()
    }

    pub fn half_size(&self) -> f32 {
        (self.cols - 1) as f32 * self.cell * 0.5
    }
}

impl HeightGrid {
    pub fn center(&self) -> Vec2 {
        Vec2::new(self.min_x, self.min_z) + Vec2::splat(self.half_size())
    }
}
