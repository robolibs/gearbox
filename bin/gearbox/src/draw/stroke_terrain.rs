//! Tessellated terrain patches for world-width flat strokes.

use super::{FLAG_FLAT, FLAG_FLAT_END, FLAG_FLAT_START, FLAG_TERRAIN, Ground, HUG_STEP, Strokes};
use bevy::prelude::*;

const VERTEX_BUDGET: usize = 1_000_000;

struct Patch {
    a: Vec3,
    b: Vec3,
    dir: Vec2,
    across: Vec2,
    radius: f32,
    length: f32,
    start: f32,
    end: f32,
}

impl Patch {
    fn of(s: &Strokes, k: usize) -> Option<Self> {
        let flags = (s.code[k] >> 2) & 0xfff;
        let radii = s.radii[k];
        if flags & FLAG_FLAT == 0 || radii.iter().any(|r| !r.is_finite() || *r <= 0.0) {
            return None;
        }
        let a = Vec3::from_array(s.a[k]);
        let b = Vec3::from_array(s.b[k]);
        if !a.is_finite() || !b.is_finite() {
            return None;
        }
        let span = (b - a).truncate();
        let length = span.length();
        let dir = span.normalize_or(Vec2::X);
        let across = Vec2::new(-dir.y, dir.x);
        let radius = radii[0].max(radii[1]);
        let reach = |neighbor: Vec2, flat: u32, r: f32| {
            if let Some(neighbor) = neighbor.try_normalize() {
                let sum = neighbor + dir;
                if neighbor.dot(dir) >= -0.85 && sum.length_squared() >= 1e-8 {
                    return radius * sum.dot(across).abs() / sum.dot(dir).max(1e-6);
                }
            }
            if flags & flat == 0 { r } else { 0.0 }
        };
        let start = reach(
            (a - Vec3::from_array(s.prev[k])).truncate(),
            FLAG_FLAT_START,
            radii[0],
        );
        let end = reach(
            (Vec3::from_array(s.next[k]) - b).truncate(),
            FLAG_FLAT_END,
            radii[1],
        );
        Some(Self {
            a,
            b,
            dir,
            across,
            radius,
            length,
            start,
            end,
        })
    }

    fn extent(&self) -> f32 {
        self.length + self.start + self.end
    }

    fn coordinate(&self, u: f32, v: f32) -> (Vec2, f32, Vec2) {
        let along = -self.start + v * self.extent();
        let xy = self.a.truncate() + self.dir * along + self.across * (u * self.radius);
        (xy, along, self.across * u + self.dir * (2.0 * v - 1.0))
    }

    fn sample(&self, u: f32, v: f32, ground: &dyn Fn(f32, f32) -> f32) -> [f32; 4] {
        let (xy, along, pad) = self.coordinate(u, v);
        let ga = ground(self.a.x, self.a.y);
        let gb = ground(self.b.x, self.b.y);
        let slope = if self.length > 1e-6 {
            (gb - ga) / self.length
        } else {
            0.0
        };
        let step = 0.01;
        let gradient = Vec2::new(
            ground(xy.x + step, xy.y) - ground(xy.x - step, xy.y),
            ground(xy.x, xy.y + step) - ground(xy.x, xy.y - step),
        ) / (2.0 * step);
        [
            u,
            v,
            ground(xy.x, xy.y) - ga - slope * along,
            gradient.dot(pad) - slope * (2.0 * v - 1.0),
        ]
    }
}

impl Strokes {
    pub(super) fn lay(&mut self, ground: Ground) {
        let Some(ground) = ground else { return };
        let count = (0..self.a.len())
            .step_by(4)
            .filter(|k| Patch::of(self, *k).is_some())
            .count();
        if count == 0 {
            return;
        }
        let per_patch = (VERTEX_BUDGET / count).max(4);
        let source = std::mem::take(self);
        self.pen = source.pen;
        self.phase = source.phase;
        self.strip_len = source.strip_len;
        for k in (0..source.a.len()).step_by(4) {
            let base = self.a.len() as u32;
            let Some(patch) = Patch::of(&source, k) else {
                for j in k..k + 4 {
                    self.copy_vertex(&source, j, [0.0; 4], false);
                }
                self.indices
                    .extend([base, base + 1, base + 3, base, base + 3, base + 2]);
                continue;
            };
            let mut nx = ((2.0 * patch.radius / HUG_STEP).ceil() as usize).clamp(1, 256);
            let mut ny = ((patch.extent() / HUG_STEP).ceil() as usize).clamp(1, 256);
            while (nx + 1) * (ny + 1) > per_patch {
                if nx >= ny && nx > 1 {
                    nx = nx.div_ceil(2);
                } else if ny > 1 {
                    ny = ny.div_ceil(2);
                } else {
                    break;
                }
            }
            for j in 0..=ny {
                for i in 0..=nx {
                    let uv = [2.0 * i as f32 / nx as f32 - 1.0, j as f32 / ny as f32];
                    self.copy_vertex(&source, k, patch.sample(uv[0], uv[1], ground), true);
                }
            }
            let row = nx as u32 + 1;
            for j in 0..ny as u32 {
                for i in 0..nx as u32 {
                    let a = base + j * row + i;
                    self.indices
                        .extend([a, a + row, a + row + 1, a, a + row + 1, a + 1]);
                }
            }
        }
    }

    fn copy_vertex(&mut self, src: &Self, k: usize, terrain: [f32; 4], baked: bool) {
        self.a.push(src.a[k]);
        self.b.push(src.b[k]);
        self.color_a.push(src.color_a[k]);
        self.color_b.push(src.color_b[k]);
        self.radii.push(src.radii[k]);
        self.dist.push(src.dist[k]);
        self.code
            .push(src.code[k] | if baked { FLAG_TERRAIN << 2 } else { 0 });
        self.fx.push(src.fx[k]);
        self.fx2.push(src.fx2[k]);
        self.prev.push(src.prev[k]);
        self.next.push(src.next[k]);
        self.terrain.push(terrain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(ground: &dyn Fn(f32, f32) -> f32) -> Strokes {
        let mut s = Strokes::default();
        let points = [
            Vec3::new(0.0, 0.0, ground(0.0, 0.0) + 0.1),
            Vec3::new(2.0, 0.0, ground(2.0, 0.0) + 0.3),
        ];
        s.polyline(
            &points,
            &[[20, 100, 240, 120]; 2],
            &[1.5; 2],
            false,
            FLAG_FLAT,
            false,
        );
        s
    }

    #[test]
    fn ribbon_samples_the_whole_width_and_keeps_height_and_color() {
        let hill = |x: f32, y: f32| 0.2 * x + 0.4 * y + 0.15 * (y * 2.0).sin();
        let mut s = strip(&hill);
        let patch = Patch::of(&s, 0).unwrap();
        s.lay(Some(&hill));
        assert!(s.a.len() > 4);
        for (k, t) in s.terrain.iter().enumerate() {
            let (xy, along, _) = patch.coordinate(t[0], t[1]);
            let height = patch.a.z + (patch.b.z - patch.a.z) * along / patch.length + t[2];
            assert!((height - hill(xy.x, xy.y) - (0.1 + 0.1 * along)).abs() < 1e-5);
            assert_eq!(s.color_a[k], [20, 100, 240, 120]);
            assert_eq!(s.fx2[k][1], s.fx2[0][1]);
            assert_ne!(s.code[k] & (FLAG_TERRAIN << 2), 0);
        }
        for tri in s.indices.chunks_exact(3) {
            for pair in [[tri[0], tri[1]], [tri[1], tri[2]], [tri[2], tri[0]]] {
                let xy = |k: u32| {
                    let t = s.terrain[k as usize];
                    patch.coordinate(t[0], t[1]).0
                };
                assert!(xy(pair[0]).distance(xy(pair[1])) <= HUG_STEP * 2.0f32.sqrt() + 1e-5);
            }
        }
    }

    #[test]
    fn screen_space_strokes_keep_their_compact_quad_layout() {
        let mut s = strip(&|_, _| 0.0);
        s.radii.fill([-1.0; 2]);
        s.lay(Some(&|_, _| 1.0));
        assert_eq!(s.a.len(), 4);
        assert!(s.terrain.is_empty());
    }

    #[test]
    fn halo_padding_tracks_cross_slope_without_a_height_step() {
        let hill = |x: f32, y: f32| 0.2 * x + 0.7 * y;
        let s = strip(&hill);
        let p = Patch::of(&s, 0).unwrap();
        for (u, v) in [(-1.0, 0.0), (1.0, 0.3), (0.2, 1.0)] {
            let t = p.sample(u, v, &hill);
            let (xy, along, pad) = p.coordinate(u, v);
            let margin = 0.12;
            let at = xy + pad * margin;
            let reach = along + (2.0 * v - 1.0) * margin;
            let height = p.a.z + (p.b.z - p.a.z) * reach / p.length + t[2] + t[3] * margin;
            assert!((height - hill(at.x, at.y) - (0.1 + 0.1 * reach)).abs() < 1e-5);
        }
    }
}
