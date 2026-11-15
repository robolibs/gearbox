//! The wheel maps as last written, kept on the CPU for the ground wheels have
//! touched, so each stamp is merged with what the texel already holds instead
//! of replacing it.

use super::contacts::{ACROSS_SPAN, SCAR_SECONDS, TRAMPLE_CLOCK_S};
use bevy::prelude::*;
use bevy::platform::collections::HashMap;

const TILE: i32 = 32;
const TILE_TEXELS: usize = (TILE * TILE) as usize;
const SWEEP_S: f32 = 5.0;

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

/// Seconds since a stamp at `stamped` on the wrapped clock, as the shader
/// reads it: a stamp a moment ahead is fresh, not a period old.
fn age(now: f32, stamped: f32) -> f32 {
    let delta = now - stamped;
    if delta < -8.0 { (delta + TRAMPLE_CLOCK_S).max(0.0) } else { delta.max(0.0) }
}

fn stamp_age(now: f32, texel: [u16; 4]) -> f32 {
    age(now, f32::from(texel[0]) / 65535.0 * TRAMPLE_CLOCK_S)
}

/// Where across its tyre a texel lies, in half-widths.
fn across(texel: [u16; 4]) -> f32 {
    (f32::from((texel[1] >> 4) & 15) / 15.0 * 2.0 - 1.0) * ACROSS_SPAN
}

/// How long a mark is read for: the sward's recovery or the bare scar's.
pub fn horizon(recovery_seconds: f32) -> f32 {
    recovery_seconds.max(SCAR_SECONDS)
}

/// `new` laid over `old`: whichever lies deeper inside its tyre is kept, so
/// a stamp's margin past its tyre never lifts what another wheel pressed and
/// a turning wheel leaves the union of its poses. A wheel on the texel now
/// marks it pressed now, whichever reading is kept.
pub fn merged(old: [u16; 4], new: [u16; 4], now: f32, horizon: f32) -> [u16; 4] {
    if old[1] == 0 || stamp_age(now, old) >= horizon {
        return new;
    }
    let (kept, fresh) = (across(old).abs(), across(new).abs());
    if fresh < kept {
        return new;
    }
    if fresh < 1.0 - 1e-3 {
        return [new[0], old[1], old[2], old[3]];
    }
    old
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

    /// Every stamped texel in `[from, from + extent)` of `self`, moved to
    /// start at `to`, with its tile's newest stamp time.
    pub fn carried(&self, from: IVec2, to: IVec2, extent: IVec2) -> Vec<(IVec2, [u16; 4], f32)> {
        let mut out = Vec::new();
        for (&tile, held) in &self.tiles {
            for (index, &texel) in held.texels.iter().enumerate() {
                let at = tile * TILE + IVec2::new(index as i32 % TILE, index as i32 / TILE);
                if texel[1] != 0 && at.cmpge(from).all() && at.cmplt(from + extent).all() {
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
    use crate::contacts::trample_texel;

    fn texel(now: f32, across: f32) -> [u16; 4] {
        let [time, packed] = trample_texel(now, Vec2::X, across, 0.5);
        [time, packed, 0, 0]
    }

    #[test]
    fn a_margin_never_lifts_a_pressed_texel() {
        let front = texel(10.0, 0.2);
        let rear_margin = texel(10.5, 2.0);
        assert_eq!(merged(front, rear_margin, 10.5, 720.0), front);
    }

    #[test]
    fn a_wheel_pressing_again_refreshes_the_time_and_keeps_the_deeper_reading() {
        let front = texel(10.0, 0.2);
        let rear_edge = texel(12.0, 0.7);
        let kept = merged(front, rear_edge, 12.0, 720.0);
        assert_eq!(kept[0], rear_edge[0]);
        assert_eq!(kept[1], front[1]);
    }

    #[test]
    fn a_deeper_press_or_a_faded_mark_is_replaced() {
        let margin = texel(10.0, 1.8);
        let inside = texel(11.0, 0.0);
        assert_eq!(merged(margin, inside, 11.0, 720.0), inside);
        let old = texel(10.0, 0.0);
        let later_margin = texel(900.0, 2.0);
        assert_eq!(merged(old, later_margin, 900.0, 720.0), later_margin);
        assert_eq!(merged([0; 4], later_margin, 900.0, 720.0), later_margin);
    }

    #[test]
    fn the_shader_reads_the_same_span_and_scar_clock() {
        let shader = include_str!("shaders/interaction.wesl");
        assert!(shader.contains(&format!("const ACROSS_SPAN: f32 = {ACROSS_SPAN:?};")));
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
