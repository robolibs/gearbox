use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;

use super::visual;

type Key = [u32; 3];

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Accent {
    pub phase: [f32; 2],
    pub mask: u8,
}

impl Accent {
    pub fn spans(self) -> Vec<[f32; 3]> {
        let [a, b] = self.phase;
        let mut cuts = vec![0.0, 1.0];
        if self.mask != 0 && (b - a).abs() > 1e-8 {
            for period in 0..if self.mask & 8 != 0 { 4 } else { 1 } {
                for (flag, shoulder) in [
                    (2, visual::BRACKET_FRACTION),
                    (4, 1.0 - visual::BRACKET_FRACTION),
                ] {
                    let t = (period as f32 + shoulder - a) / (b - a);
                    if self.mask & flag != 0 && t > 1e-6 && t < 1.0 - 1e-6 {
                        cuts.push(t);
                    }
                }
            }
        }
        cuts.sort_by(f32::total_cmp);
        cuts.windows(2)
            .map(|span| {
                let mut phase = a + (b - a) * (span[0] + span[1]) * 0.5;
                if self.mask & 8 != 0 {
                    phase = phase.fract();
                }
                let bold = (self.mask & 2 != 0 && phase < visual::BRACKET_FRACTION)
                    || (self.mask & 4 != 0 && phase > 1.0 - visual::BRACKET_FRACTION);
                [
                    span[0],
                    span[1],
                    if bold {
                        visual::STRUCTURE_CORNER_GAIN
                    } else {
                        1.0
                    },
                ]
            })
            .collect()
    }
}

fn key(p: Vec3) -> Key {
    p.to_array().map(|v| if v == 0.0 { 0 } else { v.to_bits() })
}

struct Path {
    edges: Vec<(usize, bool)>,
    ends: [Key; 2],
    length: f32,
    closed: bool,
}

pub(super) fn accents(edges: &[[Vec3; 2]]) -> Vec<Accent> {
    let mut result = vec![Accent::default(); edges.len()];
    if edges.len() >= visual::FEATURE_EDGE_LIMIT {
        return result;
    }
    let keys: Vec<_> = edges.iter().map(|edge| edge.map(key)).collect();
    let mut vertices = BTreeMap::<Key, Vec<usize>>::new();
    for (i, pair) in keys.iter().enumerate() {
        for p in pair {
            vertices.entry(*p).or_default().push(i);
        }
    }
    let smooth: BTreeSet<_> = vertices
        .iter()
        .filter_map(|(p, incident)| {
            if incident.len() != 2 {
                return None;
            }
            let direction = |i: usize| {
                let at = usize::from(keys[i][1] == *p);
                (edges[i][1 - at] - edges[i][at]).normalize_or_zero()
            };
            (direction(incident[0]).dot(direction(incident[1])) < -0.819_152).then_some(*p)
        })
        .collect();
    let mut visited = vec![false; edges.len()];
    let mut paths = Vec::new();
    let starts = vertices
        .iter()
        .filter(|(p, _)| !smooth.contains(*p))
        .flat_map(|(p, incident)| incident.iter().map(move |i| (*p, *i)))
        .chain(keys.iter().enumerate().map(|(i, pair)| (pair[0], i)));
    for (start, first) in starts {
        if visited[first] {
            continue;
        }
        let (mut at, mut next) = (start, first);
        let mut path = Path {
            edges: Vec::new(),
            ends: [start; 2],
            length: 0.0,
            closed: false,
        };
        loop {
            visited[next] = true;
            let forward = keys[next][0] == at;
            path.edges.push((next, forward));
            path.length += edges[next][0].distance(edges[next][1]);
            at = keys[next][usize::from(forward)];
            if at == start || !smooth.contains(&at) {
                break;
            }
            let Some(i) = vertices[&at].iter().find(|i| !visited[**i]) else {
                break;
            };
            next = *i;
        }
        path.ends[1] = at;
        path.closed = at == start && smooth.contains(&at);
        paths.push(path);
    }
    let mut corners = BTreeMap::<Key, f32>::new();
    let mut candidates = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        if path.closed {
            candidates.push((path.length / 4.0, None, i));
        } else {
            for p in path.ends {
                corners
                    .entry(p)
                    .and_modify(|v| *v = v.max(path.length))
                    .or_insert(path.length);
            }
        }
    }
    candidates.extend(corners.into_iter().map(|(p, length)| (length, Some(p), 0)));
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (mut junctions, mut loops) = (BTreeSet::new(), BTreeSet::new());
    let mut remaining = visual::STRUCTURE_LIMIT;
    for (_, point, path) in candidates {
        let cost = if point.is_some() { 1 } else { 4 };
        if remaining < cost {
            continue;
        }
        remaining -= cost;
        if let Some(p) = point {
            junctions.insert(p);
        } else {
            loops.insert(path);
        }
    }
    for (i, path) in paths.iter().enumerate() {
        if path.length <= 1e-8 {
            continue;
        }
        let mask = if path.closed {
            if loops.contains(&i) { 14 } else { 0 }
        } else {
            u8::from(junctions.contains(&path.ends[0])) * 2
                + u8::from(junctions.contains(&path.ends[1])) * 4
        };
        let scale = if path.closed { 4.0 } else { 1.0 } / path.length;
        let mut distance = 0.0;
        for &(edge, forward) in &path.edges {
            let end = distance + edges[edge][0].distance(edges[edge][1]);
            let phase = [distance * scale, end * scale];
            result[edge] = Accent {
                phase: if forward { phase } else { [phase[1], phase[0]] },
                mask,
            };
            distance = end;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subdivisions_do_not_become_junctions() {
        let edges = [
            [Vec3::ZERO, Vec3::X],
            [Vec3::X, 3.0 * Vec3::X],
            [3.0 * Vec3::X, 3.0 * Vec3::X + Vec3::Y],
        ];
        let data = accents(&edges);
        assert_eq!(data[0].mask, 6);
        assert_eq!(data[1].mask, 6);
        assert_eq!(data[0].phase, [0.0, 1.0 / 3.0]);
        assert_eq!(data[1].phase, [1.0 / 3.0, 1.0]);
        assert_eq!(data[2].phase, [0.0, 1.0]);
    }

    #[test]
    fn smooth_closed_rings_share_four_arclength_stations() {
        for count in [16, 64, 256] {
            let point = |i: usize| {
                let a = std::f32::consts::TAU * (i % count) as f32 / count as f32;
                Vec3::new(a.cos(), a.sin(), 0.0)
            };
            let edges: Vec<_> = (0..count).map(|i| [point(i), point(i + 1)]).collect();
            let data = accents(&edges);
            assert!(data.iter().all(|a| a.mask == 14));
            let periods: f32 = data.iter().map(|a| (a.phase[1] - a.phase[0]).abs()).sum();
            assert!((periods - 4.0).abs() < 1e-5);
            let bold: f32 = data
                .iter()
                .flat_map(|a| a.spans())
                .filter(|s| s[2] > 1.0)
                .map(|s| s[1] - s[0])
                .sum();
            assert!((bold / count as f32 - 2.0 * visual::BRACKET_FRACTION).abs() < 1e-5);
        }
    }

    #[test]
    fn accent_budget_and_truncated_topology_stay_quiet() {
        let edges: Vec<_> = (0..100)
            .map(|i| [Vec3::new(i as f32, 0.0, 0.0), Vec3::new(i as f32, 1.0, 0.0)])
            .collect();
        let data = accents(&edges);
        assert_eq!(
            data.iter()
                .map(|a| a.mask.count_ones() as usize)
                .sum::<usize>(),
            visual::STRUCTURE_LIMIT
        );
        let dense = vec![[Vec3::ZERO, Vec3::X]; visual::FEATURE_EDGE_LIMIT];
        assert!(accents(&dense).iter().all(|a| a.mask == 0));
    }
}
