//! Wheel tracks as distance fields. A texel of a wheel map holds how far
//! inside a track's edge it lies: half the tyre's width less its distance
//! from the line the tyre's middle drove. A distance carries no direction, so
//! a curve lies as cleanly as a straight, and blended between texels its zero
//! falls on the tyre's own edge. The maps are mirrored on the CPU for the
//! ground wheels have touched, so a stamp is merged with what a texel holds.

use super::contacts::{EDGE_REACH_TEXELS, SCAR_SECONDS, TRAMPLE_CLOCK_S, texel_inside, texel_stamped, texel_time};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

const TILE: i32 = 32;
const TILE_TEXELS: usize = (TILE * TILE) as usize;
const SWEEP_S: f32 = 5.0;

/// How far `at` lies inside a tyre `2 * half_width` wide whose middle ran
/// from `from` to `to`, all in texels; negative outside it.
pub fn inside(at: Vec2, from: Vec2, to: Vec2, half_width: f32) -> f32 {
    let run = to - from;
    let t = if run.length_squared() > 1e-9 {
        ((at - from).dot(run) / run.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    half_width - at.distance(from + run * t)
}

/// The texels within `EDGE_REACH_TEXELS` of that tyre's edge, row by row as
/// (row, first, last), clipped to `low..=high`.
pub fn rows(from: Vec2, to: Vec2, half_width: f32, low: IVec2, high: IVec2) -> Vec<(i32, i32, i32)> {
    let reach = half_width + EDGE_REACH_TEXELS;
    let lo = (from.min(to) - reach).floor().as_ivec2().max(low);
    let hi = (from.max(to) + reach).ceil().as_ivec2().min(high);
    let mut out = Vec::new();
    for z in lo.y..=hi.y {
        let near = |x: i32| inside(Vec2::new(x as f32, z as f32), from, to, half_width) > -EDGE_REACH_TEXELS;
        if let Some(x0) = (lo.x..=hi.x).find(|&x| near(x)) {
            let x1 = (x0..=hi.x).rev().find(|&x| near(x)).unwrap_or(x0);
            out.push((z, x0, x1));
        }
    }
    out
}

pub fn age(now: f32, stamped: f32) -> f32 {
    let delta = now - stamped;
    if delta < -8.0 { (delta + TRAMPLE_CLOCK_S).max(0.0) } else { delta.max(0.0) }
}

/// The stamp a texel keeps. Tracks join rather than replace: the texel keeps
/// the deeper of the two distances; a tyre over it now lays its roll and
/// tread, and where both pressed it the harder press stands.
pub fn merged(old: [u16; 4], new: [u16; 4], now: f32, horizon: f32) -> [u16; 4] {
    if !texel_stamped(old) || age(now, texel_time(old)) >= horizon {
        return new;
    }
    let (was, is) = (texel_inside(old), texel_inside(new));
    let harder = if age(now, texel_time(old)) < age(now, texel_time(new)) { old[0] } else { new[0] };
    if is >= was {
        return if was > 0.0 { [harder, new[1], new[2], new[3]] } else { new };
    }
    if is > 0.0 {
        return [harder, (new[1] & 0xff00) | (old[1] & 0x00ff), new[2], new[3]];
    }
    old
}

/// How long a mark is read for: the sward's recovery or the bare scar's.
pub fn horizon(recovery_seconds: f32) -> f32 {
    recovery_seconds.max(SCAR_SECONDS)
}

/// One mirror per wheel map, by the map's image.
#[derive(Resource, Default)]
pub struct TrampleMirrors {
    pub maps: HashMap<AssetId<Image>, Mirror>,
    swept: f32,
}

#[derive(Default)]
pub struct Mirror {
    tiles: HashMap<IVec2, Tile>,
}

struct Tile {
    texels: Box<[[u16; 4]; TILE_TEXELS]>,
    newest: f32,
}

impl Mirror {
    pub fn get(&self, at: IVec2) -> [u16; 4] {
        let (tile, index) = split(at);
        self.tiles.get(&tile).map_or([0; 4], |tile| tile.texels[index])
    }

    pub fn set(&mut self, at: IVec2, texel: [u16; 4], now: f32) {
        let (tile, index) = split(at);
        let tile = self.tiles.entry(tile).or_insert_with(|| Tile {
            texels: Box::new([[0; 4]; TILE_TEXELS]),
            newest: now,
        });
        tile.texels[index] = texel;
        tile.newest = now;
    }

    /// Every stamped texel in `[from, from + extent)`, moved to start at
    /// `to`, with its tile's newest stamp time.
    pub fn carried(&self, from: IVec2, to: IVec2, extent: IVec2) -> Vec<(IVec2, [u16; 4], f32)> {
        let mut out = Vec::new();
        for (&tile, held) in &self.tiles {
            for (index, &texel) in held.texels.iter().enumerate() {
                let at = tile * TILE + IVec2::new(index as i32 % TILE, index as i32 / TILE);
                if texel_stamped(texel) && at.cmpge(from).all() && at.cmplt(from + extent).all() {
                    out.push((at - from + to, texel, held.newest));
                }
            }
        }
        out
    }
}

impl TrampleMirrors {
    /// Drops maps no longer drawn, and every few seconds the tiles whose
    /// newest stamp has outlived their map's horizon.
    pub fn sweep(&mut self, horizon_of: impl Fn(&AssetId<Image>) -> Option<f32>, now: f32) {
        self.maps.retain(|id, _| horizon_of(id).is_some());
        if age(now, self.swept) < SWEEP_S {
            return;
        }
        self.swept = now;
        for (id, mirror) in &mut self.maps {
            let oldest = horizon_of(id).unwrap_or(0.0);
            mirror.tiles.retain(|_, tile| age(now, tile.newest) < oldest);
        }
    }
}

fn split(at: IVec2) -> (IVec2, usize) {
    let tile = IVec2::new(at.x.div_euclid(TILE), at.y.div_euclid(TILE));
    let local = at - tile * TILE;
    (tile, (local.y * TILE + local.x) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::{EDGE_REACH_TEXELS, trample_texel};

    const WIDE: IVec2 = IVec2::splat(4096);

    /// Lays a wheel driven through `path` (texels, one point a frame) the way
    /// the render world does.
    fn drive(mirror: &mut Mirror, path: &[Vec2], half_width: f32, now: f32) {
        for leg in path.windows(2) {
            let roll = (leg[1] - leg[0]).normalize_or(Vec2::X);
            for (z, x0, x1) in rows(leg[0], leg[1], half_width, IVec2::ZERO, WIDE) {
                for x in x0..=x1 {
                    let at = IVec2::new(x, z);
                    let depth = inside(at.as_vec2(), leg[0], leg[1], half_width);
                    let [time, packed] = trample_texel(now, roll, depth, 0.5);
                    let kept = merged(mirror.get(at), [time, packed, 0, 0], now, 720.0);
                    mirror.set(at, kept, now);
                }
            }
        }
    }

    /// The distance the shader reads at `at`: blended between the four
    /// texels round it, unstamped ones lying a full reach outside.
    fn read(mirror: &Mirror, at: Vec2) -> f32 {
        let corner = at.floor();
        let f = at - corner;
        let v = |dx: i32, dz: i32| {
            let texel = mirror.get(corner.as_ivec2() + IVec2::new(dx, dz));
            if texel_stamped(texel) { texel_inside(texel) } else { -EDGE_REACH_TEXELS }
        };
        let top = v(0, 0) + (v(1, 0) - v(0, 0)) * f.x;
        let bottom = v(0, 1) + (v(1, 1) - v(0, 1)) * f.x;
        top + (bottom - top) * f.y
    }

    /// Width of the pressed band across `at`, measured along `normal`.
    fn width_across(mirror: &Mirror, at: Vec2, normal: Vec2, half_width: f32) -> f32 {
        let step = 0.005;
        let span = half_width + 1.0;
        let pressed = (0..=((2.0 * span / step) as i32))
            .map(|k| -span + k as f32 * step)
            .filter(|&s| read(mirror, at + normal * s) > 0.0)
            .count();
        pressed as f32 * step
    }

    #[test]
    fn a_straight_track_is_the_tyre_wide_at_any_heading() {
        for half_width in [0.8, 1.3, 2.6] {
            for degrees in [0.0_f32, 7.0, 22.5, 30.0, 45.0, 61.0, 90.0, 133.0] {
                let mut mirror = Mirror::default();
                let dir = Vec2::from_angle(degrees.to_radians());
                let start = Vec2::splat(200.0);
                let path: Vec<Vec2> = (0..400).map(|k| start + dir * (k as f32 * 0.15)).collect();
                drive(&mut mirror, &path, half_width, 10.0);
                for k in [80, 150, 233, 310] {
                    let width = width_across(&mirror, path[k], dir.perp(), half_width);
                    assert!((width - 2.0 * half_width).abs() < 0.06,
                        "half {half_width} at {degrees}°: {width} wide");
                }
            }
        }
    }

    #[test]
    fn a_curved_track_is_the_tyre_wide_all_the_way_round() {
        for half_width in [0.8, 2.6] {
            for radius in [12.0_f32, 30.0, 80.0] {
                let mut mirror = Mirror::default();
                let centre = Vec2::splat(300.0);
                let at = |a: f32| centre + Vec2::from_angle(a) * radius;
                let path: Vec<Vec2> = (0..((3.0 * radius / 0.15) as i32)).map(|k| at(k as f32 * 0.15 / radius)).collect();
                drive(&mut mirror, &path, half_width, 10.0);
                for a in [0.4_f32, 0.9, 1.37, 2.1, 2.6] {
                    let width = width_across(&mirror, at(a), Vec2::from_angle(a), half_width);
                    assert!((width - 2.0 * half_width).abs() < 0.08,
                        "half {half_width} radius {radius} at {a}: {width} wide");
                }
            }
        }
    }

    #[test]
    fn a_track_crossed_by_another_stays_pressed_and_neither_spreads() {
        let mut mirror = Mirror::default();
        let first: Vec<Vec2> = (0..300).map(|k| Vec2::new(100.0 + k as f32 * 0.15, 140.0)).collect();
        let dir = Vec2::from_angle(1.1);
        let second: Vec<Vec2> = (0..300).map(|k| Vec2::new(110.0, 120.0) + dir * (k as f32 * 0.15)).collect();
        drive(&mut mirror, &first, 1.3, 10.0);
        drive(&mut mirror, &second, 0.8, 12.0);
        for k in (0..300).step_by(7) {
            let p = first[k];
            assert!(read(&mirror, p) > 0.0, "first track lifted at {p}");
            if (p - second[0]).perp_dot(dir).abs() > 3.0 {
                let width = width_across(&mirror, p, Vec2::Y, 1.3);
                assert!((width - 2.6).abs() < 0.06, "first track {width} wide at {p}");
            }
        }
        for k in (0..300).step_by(7) {
            assert!(read(&mirror, second[k]) > 0.0);
        }
        for x in 0..400 {
            for z in [137.0, 143.0] {
                let p = Vec2::new(100.0 + x as f32 * 0.1, z);
                let off_second = (p - second[0]).perp_dot(dir).abs();
                if off_second > 1.0 {
                    assert!(read(&mirror, p) <= 0.0, "pressed outside both at {p}");
                }
            }
        }
    }

    #[test]
    fn a_fresh_press_keeps_the_deeper_reach_and_a_faded_one_gives_way() {
        let deep = { let [t, g] = trample_texel(10.0, Vec2::X, 1.2, 0.5); [t, g, 0, 0] };
        let shallow = { let [t, g] = trample_texel(20.0, Vec2::Y, 0.4, 0.5); [t, g, 7, 7] };
        let kept = merged(deep, shallow, 20.0, 720.0);
        assert!((texel_inside(kept) - texel_inside(deep)).abs() < 1e-6);
        assert_eq!(kept[0], shallow[0]);
        assert_eq!(kept[2], 7);
        let light = { let [t, g] = trample_texel(20.0 - 200.0, Vec2::Y, 1.4, 0.5); [t, g, 5, 5] };
        let lifted = merged(deep, light, 20.0, 720.0);
        assert_eq!(lifted[0], deep[0], "a wheel riding high lightened a full press");
        assert!((texel_inside(lifted) - texel_inside(light)).abs() < 1e-6);
        let outside = { let [t, g] = trample_texel(20.0, Vec2::Y, -0.6, 0.5); [t, g, 0, 0] };
        assert_eq!(merged(deep, outside, 20.0, 720.0), deep);
        assert_eq!(merged(deep, outside, 900.0, 720.0), outside);
        assert_eq!(merged([0; 4], outside, 20.0, 720.0), outside);
    }

    #[test]
    fn the_shader_reads_the_same_reach_and_scar_clock() {
        let shader = include_str!("shaders/interaction.wesl");
        assert!(shader.contains(&format!("const EDGE_REACH_TEXELS: f32 = {EDGE_REACH_TEXELS:?};")));
        assert!(shader.contains(&format!("const SCAR_SECONDS: f32 = {SCAR_SECONDS:?};")));
    }

    #[test]
    fn the_mirror_reads_back_what_was_set_on_either_side_of_the_origin() {
        let mut mirror = Mirror::default();
        for at in [IVec2::new(-1, -1), IVec2::new(0, 0), IVec2::new(31, 32), IVec2::new(-33, 5)] {
            mirror.set(at, [1, 2, 3, at.x as u16], 0.0);
            assert_eq!(mirror.get(at), [1, 2, 3, at.x as u16]);
        }
        assert_eq!(mirror.get(IVec2::new(7, 7)), [0; 4]);
        let moved = mirror.carried(IVec2::new(0, 0), IVec2::new(100, 100), IVec2::new(40, 40));
        assert_eq!(moved.len(), 2);
        assert!(moved.contains(&(IVec2::new(131, 132), [1, 2, 3, 31], 0.0)));
    }
}
