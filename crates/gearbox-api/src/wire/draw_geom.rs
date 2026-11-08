//! Geometry and colour helpers shared by the draw clients and the viewer:
//! colormaps, polygon triangulation with holes, covariance ellipses and
//! ellipsoids, point quantisation and camera fields of view.

use super::draw::{color, colormap, rgba, unpack_rgba};

/// A colour of ramp `map` at `t` in 0..1, RGBA8.
pub fn ramp(map: u32, t: f32) -> [u8; 4] {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let [r, g, b] = match map {
        colormap::TURBO => turbo(t),
        colormap::VIRIDIS => viridis(t),
        colormap::RDYLGN => {
            let stops = [unpack_rgba(color::RED), unpack_rgba(color::YELLOW), unpack_rgba(color::GREEN)];
            let (a, b, f) = if t < 0.5 { (stops[0], stops[1], t * 2.0) } else { (stops[1], stops[2], t * 2.0 - 1.0) };
            let m = |k: usize| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * f) / 255.0;
            [m(0), m(1), m(2)]
        }
        colormap::RAINBOW => hsv((1.0 - t) * 240.0, 1.0, 1.0),
        _ => [t, t, t],
    };
    [q(r), q(g), q(b), 255]
}

/// A colour of palette `map` for the raw value `v` (a cell byte, an index).
pub fn palette(map: u32, v: f32) -> [u8; 4] {
    let i = if v.is_finite() { v.round() as i64 } else { -1 };
    match map {
        colormap::COSTMAP => {
            let i = if i < 0 { 255 } else { i.min(255) };
            match i {
                0 => [0, 0, 0, 0],
                1..=98 => {
                    let c = (255 * i / 100) as u8;
                    [c, 0, 255 - c, 255]
                }
                99 => [0, 255, 255, 255],
                100 => [255, 0, 255, 255],
                101..=127 => [0, 255, 0, 255],
                128..=254 => [255, (255 * (i - 128) / (254 - 128)) as u8, 0, 255],
                _ => [0x70, 0x89, 0x86, 255],
            }
        }
        colormap::MAP => {
            let i = if i < 0 { 255 } else { i.min(255) };
            match i {
                0..=100 => {
                    let c = (255 - 255 * i / 100) as u8;
                    [c, c, c, 255]
                }
                101..=127 => [0, 255, 0, 255],
                128..=254 => [255, (255 * (i - 128) / (254 - 128)) as u8, 0, 255],
                _ => [0x70, 0x89, 0x86, 255],
            }
        }
        colormap::OKABE => unpack_rgba(color::OKABE[i.rem_euclid(color::OKABE.len() as i64) as usize]),
        _ => {
            let c = i.clamp(0, 255) as u8;
            [c, c, c, 255]
        }
    }
}

/// The colour `map` gives value `v` over `vmin..vmax` (palettes take it raw).
pub fn colormap_rgba(map: u32, v: f32, vmin: f32, vmax: f32) -> [u8; 4] {
    if colormap::is_palette(map) {
        return palette(map, v);
    }
    let span = vmax - vmin;
    let t = if span.abs() > 1e-12 { (v - vmin) / span } else { 0.5 };
    ramp(map, t)
}

/// Same as [`colormap_rgba`], packed.
pub fn colormap_color(map: u32, v: f32, vmin: f32, vmax: f32) -> u32 {
    let [r, g, b, a] = colormap_rgba(map, v, vmin, vmax);
    rgba(r, g, b, a)
}

/// The range values map over: `vmin..vmax`, or the data's own when equal.
pub fn value_range(values: impl IntoIterator<Item = f32>, vmin: f32, vmax: f32) -> (f32, f32) {
    if (vmax - vmin).abs() > 1e-12 {
        return (vmin, vmax);
    }
    let (lo, hi) = values.into_iter().filter(|v| v.is_finite()).fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if lo > hi { (0.0, 1.0) } else if hi - lo < 1e-9 { (lo - 0.5, hi + 0.5) } else { (lo, hi) }
}

fn turbo(t: f32) -> [f32; 3] {
    let p = |c: [f32; 6]| c[0] + t * (c[1] + t * (c[2] + t * (c[3] + t * (c[4] + t * c[5]))));
    [
        p([0.135_721_38, 4.615_392_6, -42.660_324, 132.131_08, -152.942_4, 59.286_38]),
        p([0.091_402_61, 2.194_188_4, 4.842_966_6, -14.185_033, 4.277_298_5, 2.829_566]),
        p([0.106_673_3, 12.641_946, -60.582_05, 110.362_77, -89.903_11, 27.348_25]),
    ]
}

fn viridis(t: f32) -> [f32; 3] {
    const C: [[f32; 3]; 7] = [
        [0.277_727_33, 0.005_407_344_5, 0.334_099_8],
        [0.105_093_04, 1.404_613_5, 1.384_590_2],
        [-0.330_861_83, 0.214_847_56, 0.095_095_16],
        [-4.634_230_6, -5.799_101, -19.332_441],
        [6.228_27, 14.179_933, 56.690_55],
        [4.776_385, -13.745_145, -65.353_03],
        [-5.435_456, 4.645_852_6, 26.312_435],
    ];
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        *o = C.iter().rev().fold(0.0, |acc, c| acc * t + c[k]);
    }
    out
}

fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = (h.rem_euclid(360.0)) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

/// `points` as steps of `quant` metres, clamped to i16.
pub fn quantize(points: &[[f32; 3]], quant: f32) -> Vec<i16> {
    let q = if quant > 0.0 { quant } else { super::draw::DEFAULT_POINT_QUANT };
    points.iter().flatten().map(|v| (v / q).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16).collect()
}

/// Steps of `quant` metres back as metres.
pub fn dequantize(steps: &[i16], quant: f32) -> Vec<f32> {
    steps.iter().map(|s| *s as f32 * quant).collect()
}

/// Half-extents of a 2-D covariance `[[xx, xy], [xy, yy]]` at one sigma:
/// the major and minor standard deviations and the major axis' angle from
/// +x, radians.
pub fn covariance_ellipse(xx: f64, xy: f64, yy: f64) -> (f64, f64, f64) {
    let mid = 0.5 * (xx + yy);
    let half = (0.25 * (xx - yy).powi(2) + xy * xy).sqrt();
    let major = (mid + half).max(0.0).sqrt();
    let minor = (mid - half).max(0.0).sqrt();
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    (major, minor, angle)
}

/// Standard deviations along the principal axes of a symmetric 3×3
/// covariance (row-major) and the rotation (w, x, y, z) whose x, y, z are
/// those axes, largest first.
pub fn covariance_axes(c: [[f64; 3]; 3]) -> ([f64; 3], [f64; 4]) {
    let (values, vectors) = jacobi3(c);
    let mut order = [0usize, 1, 2];
    order.sort_by(|a, b| values[*b].total_cmp(&values[*a]));
    let col = |k: usize| [vectors[0][order[k]], vectors[1][order[k]], vectors[2][order[k]]];
    let (x, y) = (col(0), col(1));
    let z = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
    let sigmas = [values[order[0]].max(0.0).sqrt(), values[order[1]].max(0.0).sqrt(), values[order[2]].max(0.0).sqrt()];
    (sigmas, quat_from_axes(x, y, z))
}

/// Eigenvalues and eigenvectors (columns) of a symmetric 3×3 by Jacobi sweeps.
fn jacobi3(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        if off < 1e-15 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-18 {
                continue;
            }
            let theta = 0.5 * (a[q][q] - a[p][p]) / a[p][q];
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in 0..3 {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for row in v.iter_mut() {
                let (vp, vq) = (row[p], row[q]);
                row[p] = c * vp - s * vq;
                row[q] = s * vp + c * vq;
            }
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}

/// The rotation (w, x, y, z) taking the unit axes onto `x`, `y`, `z`.
pub fn quat_from_axes(x: [f64; 3], y: [f64; 3], z: [f64; 3]) -> [f64; 4] {
    let (m00, m11, m22) = (x[0], y[1], z[2]);
    let trace = m00 + m11 + m22;
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [0.25 * s, (y[2] - z[1]) / s, (z[0] - x[2]) / s, (x[1] - y[0]) / s]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        [(y[2] - z[1]) / s, 0.25 * s, (y[0] + x[1]) / s, (z[0] + x[2]) / s]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        [(z[0] - x[2]) / s, (y[0] + x[1]) / s, 0.25 * s, (z[1] + y[2]) / s]
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        [(x[1] - y[0]) / s, (z[0] + x[2]) / s, (z[1] + y[2]) / s, 0.25 * s]
    };
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-12);
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

/// The rotation (w, x, y, z) of `yaw` radians about z.
pub fn yaw_quat(yaw: f64) -> [f64; 4] {
    [(0.5 * yaw).cos(), 0.0, 0.0, (0.5 * yaw).sin()]
}

/// `v` turned by the rotation `q` (w, x, y, z).
pub fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let [w, x, y, z] = q;
    let u = [x, y, z];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let ut = cross(u, t);
    [v[0] + w * t[0] + ut[0], v[1] + w * t[1] + ut[1], v[2] + w * t[2] + ut[2]]
}

/// Horizontal and vertical field of view, radians, of a pinhole camera
/// with focal lengths `fx`, `fy` pixels over a `width` by `height` image.
pub fn pinhole_fov(fx: f64, fy: f64, width: f64, height: f64) -> (f64, f64) {
    (2.0 * (0.5 * width / fx.max(1e-9)).atan(), 2.0 * (0.5 * height / fy.max(1e-9)).atan())
}

/// Triangles (vertex index triples) filling a polygon whose rings are
/// `rings`: the outline first, then its holes, each closed implicitly.
/// Vertices are numbered across the rings in order. An ear-clipping
/// triangulator in the manner of mapbox earcut, holes bridged in.
pub fn triangulate(rings: &[&[[f32; 2]]]) -> Vec<[u32; 3]> {
    let mut data: Vec<[f64; 2]> = Vec::new();
    let mut starts = Vec::new();
    for ring in rings {
        starts.push(data.len());
        data.extend(ring.iter().map(|p| [p[0] as f64, p[1] as f64]));
    }
    let mut ear = Earcut { nodes: Vec::with_capacity(data.len() * 2), triangles: Vec::new() };
    let outer_end = starts.get(1).copied().unwrap_or(data.len());
    let Some(mut outer) = ear.linked_list(&data, 0, outer_end, true) else {
        return Vec::new();
    };
    if ear.nodes[outer].next == ear.nodes[outer].prev {
        return Vec::new();
    }
    if rings.len() > 1 {
        let mut queue = Vec::new();
        for k in 1..starts.len() {
            let end = starts.get(k + 1).copied().unwrap_or(data.len());
            if let Some(list) = ear.linked_list(&data, starts[k], end, false) {
                if ear.nodes[list].next == list {
                    ear.nodes[list].steiner = true;
                }
                queue.push(ear.leftmost(list));
            }
        }
        queue.sort_by(|a, b| ear.nodes[*a].x.total_cmp(&ear.nodes[*b].x));
        for hole in queue {
            outer = ear.eliminate_hole(hole, outer);
        }
    }
    ear.earcut_linked(Some(outer), 0);
    ear.triangles
}

#[derive(Clone, Copy)]
struct Node {
    i: u32,
    x: f64,
    y: f64,
    prev: usize,
    next: usize,
    steiner: bool,
}

struct Earcut {
    nodes: Vec<Node>,
    triangles: Vec<[u32; 3]>,
}

impl Earcut {
    fn insert(&mut self, i: usize, p: [f64; 2], last: Option<usize>) -> usize {
        let n = self.nodes.len();
        let mut node = Node { i: i as u32, x: p[0], y: p[1], prev: n, next: n, steiner: false };
        if let Some(last) = last {
            node.next = self.nodes[last].next;
            node.prev = last;
            let after = self.nodes[last].next;
            self.nodes.push(node);
            self.nodes[after].prev = n;
            self.nodes[last].next = n;
        } else {
            self.nodes.push(node);
        }
        n
    }

    fn remove(&mut self, p: usize) {
        let (prev, next) = (self.nodes[p].prev, self.nodes[p].next);
        self.nodes[next].prev = prev;
        self.nodes[prev].next = next;
    }

    fn linked_list(&mut self, data: &[[f64; 2]], start: usize, end: usize, clockwise: bool) -> Option<usize> {
        if end <= start {
            return None;
        }
        let mut sum = 0.0;
        let mut j = end - 1;
        for i in start..end {
            sum += (data[j][0] - data[i][0]) * (data[i][1] + data[j][1]);
            j = i;
        }
        let mut last = None;
        if clockwise == (sum > 0.0) {
            for (i, p) in data.iter().enumerate().take(end).skip(start) {
                last = Some(self.insert(i, *p, last));
            }
        } else {
            for i in (start..end).rev() {
                last = Some(self.insert(i, data[i], last));
            }
        }
        let last = last?;
        if self.equals(last, self.nodes[last].next) {
            let next = self.nodes[last].next;
            self.remove(last);
            return Some(next);
        }
        Some(last)
    }

    fn area(&self, p: usize, q: usize, r: usize) -> f64 {
        let (p, q, r) = (&self.nodes[p], &self.nodes[q], &self.nodes[r]);
        (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
    }

    fn equals(&self, a: usize, b: usize) -> bool {
        self.nodes[a].x == self.nodes[b].x && self.nodes[a].y == self.nodes[b].y
    }

    fn filter_points(&mut self, start: usize, end: Option<usize>) -> usize {
        let mut end = end.unwrap_or(start);
        let mut p = start;
        loop {
            let next = self.nodes[p].next;
            let prev = self.nodes[p].prev;
            if !self.nodes[p].steiner && (self.equals(p, next) || self.area(prev, p, next) == 0.0) {
                self.remove(p);
                p = prev;
                end = prev;
                if p == self.nodes[p].next {
                    break;
                }
                continue;
            }
            p = next;
            if p == end {
                break;
            }
        }
        end
    }

    fn earcut_linked(&mut self, ear: Option<usize>, pass: u8) {
        let Some(mut ear) = ear else {
            return;
        };
        let mut stop = ear;
        let mut guard = 0usize;
        while self.nodes[ear].prev != self.nodes[ear].next {
            guard += 1;
            if guard > 4 * self.nodes.len() * self.nodes.len().max(16) {
                return;
            }
            let prev = self.nodes[ear].prev;
            let next = self.nodes[ear].next;
            if self.is_ear(ear) {
                self.triangles.push([self.nodes[prev].i, self.nodes[ear].i, self.nodes[next].i]);
                self.remove(ear);
                ear = self.nodes[next].next;
                stop = ear;
                continue;
            }
            ear = next;
            if ear == stop {
                match pass {
                    0 => {
                        let f = self.filter_points(ear, None);
                        self.earcut_linked(Some(f), 1);
                    }
                    1 => {
                        let f = self.filter_points(ear, None);
                        let cured = self.cure_local_intersections(f);
                        self.earcut_linked(Some(cured), 2);
                    }
                    _ => self.split_earcut(ear),
                }
                break;
            }
        }
    }

    fn is_ear(&self, ear: usize) -> bool {
        let (a, b, c) = (self.nodes[ear].prev, ear, self.nodes[ear].next);
        if self.area(a, b, c) >= 0.0 {
            return false;
        }
        let (na, nb, nc) = (self.nodes[a], self.nodes[b], self.nodes[c]);
        let (x0, x1) = (na.x.min(nb.x).min(nc.x), na.x.max(nb.x).max(nc.x));
        let (y0, y1) = (na.y.min(nb.y).min(nc.y), na.y.max(nb.y).max(nc.y));
        let mut p = nc.next;
        while p != a {
            let np = self.nodes[p];
            if np.x >= x0
                && np.x <= x1
                && np.y >= y0
                && np.y <= y1
                && in_triangle(na.x, na.y, nb.x, nb.y, nc.x, nc.y, np.x, np.y)
                && self.area(np.prev, p, np.next) >= 0.0
            {
                return false;
            }
            p = np.next;
        }
        true
    }

    fn cure_local_intersections(&mut self, start: usize) -> usize {
        let mut start = start;
        let mut p = start;
        loop {
            let a = self.nodes[p].prev;
            let pn = self.nodes[p].next;
            let b = self.nodes[pn].next;
            if !self.equals(a, b) && self.intersects(a, p, pn, b) && self.locally_inside(a, b) && self.locally_inside(b, a) {
                self.triangles.push([self.nodes[a].i, self.nodes[p].i, self.nodes[b].i]);
                self.remove(p);
                self.remove(pn);
                p = b;
                start = b;
            }
            p = self.nodes[p].next;
            if p == start {
                break;
            }
        }
        self.filter_points(p, None)
    }

    fn split_earcut(&mut self, start: usize) {
        let mut a = start;
        loop {
            let mut b = self.nodes[self.nodes[a].next].next;
            while b != self.nodes[a].prev {
                if self.nodes[a].i != self.nodes[b].i && self.is_valid_diagonal(a, b) {
                    let c = self.split_polygon(a, b);
                    let an = self.nodes[a].next;
                    let a = self.filter_points(a, Some(an));
                    let cn = self.nodes[c].next;
                    let c = self.filter_points(c, Some(cn));
                    self.earcut_linked(Some(a), 0);
                    self.earcut_linked(Some(c), 0);
                    return;
                }
                b = self.nodes[b].next;
            }
            a = self.nodes[a].next;
            if a == start {
                break;
            }
        }
    }

    fn leftmost(&self, start: usize) -> usize {
        let mut p = start;
        let mut left = start;
        loop {
            let (np, nl) = (self.nodes[p], self.nodes[left]);
            if np.x < nl.x || (np.x == nl.x && np.y < nl.y) {
                left = p;
            }
            p = np.next;
            if p == start {
                break;
            }
        }
        left
    }

    fn eliminate_hole(&mut self, hole: usize, outer: usize) -> usize {
        let Some(bridge) = self.hole_bridge(hole, outer) else {
            return outer;
        };
        let reverse = self.split_polygon(bridge, hole);
        let rn = self.nodes[reverse].next;
        self.filter_points(reverse, Some(rn));
        let bn = self.nodes[bridge].next;
        self.filter_points(bridge, Some(bn))
    }

    fn hole_bridge(&self, hole: usize, outer: usize) -> Option<usize> {
        let (hx, hy) = (self.nodes[hole].x, self.nodes[hole].y);
        let mut qx = f64::NEG_INFINITY;
        let mut m = None;
        let mut p = outer;
        loop {
            let (np, nn) = (self.nodes[p], self.nodes[self.nodes[p].next]);
            if hy <= np.y && hy >= nn.y && nn.y != np.y {
                let x = np.x + (hy - np.y) * (nn.x - np.x) / (nn.y - np.y);
                if x <= hx && x > qx {
                    qx = x;
                    m = Some(if np.x < nn.x { p } else { np.next });
                    if x == hx {
                        return m;
                    }
                }
            }
            p = np.next;
            if p == outer {
                break;
            }
        }
        let mut m = m?;
        let stop = m;
        let (mx, my) = (self.nodes[m].x, self.nodes[m].y);
        let mut tan_min = f64::INFINITY;
        p = m;
        loop {
            let np = self.nodes[p];
            if hx >= np.x
                && np.x >= mx
                && hx != np.x
                && in_triangle(if hy < my { hx } else { qx }, hy, mx, my, if hy < my { qx } else { hx }, hy, np.x, np.y)
            {
                let tan = (hy - np.y).abs() / (hx - np.x);
                let nm = self.nodes[m];
                if self.locally_inside(p, hole)
                    && (tan < tan_min || (tan == tan_min && (np.x > nm.x || (np.x == nm.x && self.sector_contains_sector(m, p)))))
                {
                    m = p;
                    tan_min = tan;
                }
            }
            p = np.next;
            if p == stop {
                break;
            }
        }
        Some(m)
    }

    fn sector_contains_sector(&self, m: usize, p: usize) -> bool {
        self.area(self.nodes[m].prev, m, self.nodes[p].prev) < 0.0 && self.area(self.nodes[p].next, m, self.nodes[m].next) < 0.0
    }

    fn is_valid_diagonal(&self, a: usize, b: usize) -> bool {
        let (na, nb) = (self.nodes[a], self.nodes[b]);
        self.nodes[na.next].i != nb.i
            && self.nodes[na.prev].i != nb.i
            && !self.intersects_polygon(a, b)
            && ((self.locally_inside(a, b)
                && self.locally_inside(b, a)
                && self.middle_inside(a, b)
                && (self.area(na.prev, a, nb.prev) != 0.0 || self.area(a, nb.prev, b) != 0.0))
                || (self.equals(a, b) && self.area(na.prev, a, na.next) > 0.0 && self.area(nb.prev, b, nb.next) > 0.0))
    }

    fn intersects(&self, p1: usize, q1: usize, p2: usize, q2: usize) -> bool {
        let sign = |v: f64| (v > 0.0) as i32 - (v < 0.0) as i32;
        let o1 = sign(self.area(p1, q1, p2));
        let o2 = sign(self.area(p1, q1, q2));
        let o3 = sign(self.area(p2, q2, p1));
        let o4 = sign(self.area(p2, q2, q1));
        if o1 != o2 && o3 != o4 {
            return true;
        }
        (o1 == 0 && self.on_segment(p1, p2, q1))
            || (o2 == 0 && self.on_segment(p1, q2, q1))
            || (o3 == 0 && self.on_segment(p2, p1, q2))
            || (o4 == 0 && self.on_segment(p2, q1, q2))
    }

    fn on_segment(&self, p: usize, q: usize, r: usize) -> bool {
        let (p, q, r) = (&self.nodes[p], &self.nodes[q], &self.nodes[r]);
        q.x <= p.x.max(r.x) && q.x >= p.x.min(r.x) && q.y <= p.y.max(r.y) && q.y >= p.y.min(r.y)
    }

    fn intersects_polygon(&self, a: usize, b: usize) -> bool {
        let (ia, ib) = (self.nodes[a].i, self.nodes[b].i);
        let mut p = a;
        loop {
            let np = self.nodes[p];
            let ni = self.nodes[np.next].i;
            if np.i != ia && ni != ia && np.i != ib && ni != ib && self.intersects(p, np.next, a, b) {
                return true;
            }
            p = np.next;
            if p == a {
                break;
            }
        }
        false
    }

    fn locally_inside(&self, a: usize, b: usize) -> bool {
        let na = self.nodes[a];
        if self.area(na.prev, a, na.next) < 0.0 {
            self.area(a, b, na.next) >= 0.0 && self.area(a, na.prev, b) >= 0.0
        } else {
            self.area(a, b, na.prev) < 0.0 || self.area(a, na.next, b) < 0.0
        }
    }

    fn middle_inside(&self, a: usize, b: usize) -> bool {
        let (na, nb) = (self.nodes[a], self.nodes[b]);
        let (px, py) = (0.5 * (na.x + nb.x), 0.5 * (na.y + nb.y));
        let mut inside = false;
        let mut p = a;
        loop {
            let np = self.nodes[p];
            let nn = self.nodes[np.next];
            if ((np.y > py) != (nn.y > py)) && nn.y != np.y && px < (nn.x - np.x) * (py - np.y) / (nn.y - np.y) + np.x {
                inside = !inside;
            }
            p = np.next;
            if p == a {
                break;
            }
        }
        inside
    }

    fn split_polygon(&mut self, a: usize, b: usize) -> usize {
        let (na, nb) = (self.nodes[a], self.nodes[b]);
        let a2 = self.nodes.len();
        self.nodes.push(Node { prev: a2, next: a2, steiner: false, ..na });
        let b2 = self.nodes.len();
        self.nodes.push(Node { prev: b2, next: b2, steiner: false, ..nb });
        let (an, bp) = (na.next, nb.prev);
        self.nodes[a].next = b;
        self.nodes[b].prev = a;
        self.nodes[a2].next = an;
        self.nodes[an].prev = a2;
        self.nodes[b2].next = a2;
        self.nodes[a2].prev = b2;
        self.nodes[bp].next = b2;
        self.nodes[b2].prev = bp;
        b2
    }
}

#[allow(clippy::too_many_arguments)]
fn in_triangle(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64, px: f64, py: f64) -> bool {
    (cx - px) * (ay - py) >= (ax - px) * (cy - py)
        && (ax - px) * (by - py) >= (bx - px) * (ay - py)
        && (bx - px) * (cy - py) >= (cx - px) * (by - py)
}

/// Signed area of a ring, positive when it turns counter-clockwise.
pub fn ring_area(ring: &[[f32; 2]]) -> f32 {
    let n = ring.len();
    (0..n).map(|i| {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        a[0] * b[1] - b[0] * a[1]
    })
    .sum::<f32>()
        * 0.5
}

/// Whether `p` lies inside the ring (even-odd).
pub fn in_ring(ring: &[[f32; 2]], p: [f32; 2]) -> bool {
    let n = ring.len();
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Distance from `p` to the nearest edge of any of `rings`.
pub fn edge_distance(rings: &[&[[f32; 2]]], p: [f32; 2]) -> f32 {
    let mut best = f32::MAX;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let ab = [b[0] - a[0], b[1] - a[1]];
            let ap = [p[0] - a[0], p[1] - a[1]];
            let len2 = ab[0] * ab[0] + ab[1] * ab[1];
            let t = if len2 > 1e-12 { ((ap[0] * ab[0] + ap[1] * ab[1]) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let d = [ap[0] - ab[0] * t, ap[1] - ab[1] * t];
            best = best.min((d[0] * d[0] + d[1] * d[1]).sqrt());
        }
    }
    best
}
