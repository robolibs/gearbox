use egui::{Pos2, Rect, Vec2, pos2, vec2};

pub(super) fn place(bounds: Rect, viewport: Rect, occupied: &[Rect], gap: f32) -> Rect {
    let inset = viewport.shrink(8.0);
    let min = bounds.min.clamp(inset.min, (inset.max - bounds.size()).max(inset.min));
    let base = Rect::from_min_size(min, bounds.size());
    let overlap = |r: Rect| occupied.iter().filter_map(|other| {
        let padded = r.expand(gap * 0.5);
        padded.intersects(*other).then(|| padded.intersect(*other).area())
    }).sum::<f32>();
    if inset.contains_rect(base) && overlap(base) == 0.0 { return base; }
    if !inset.contains_rect(base) { return base; }
    let limits = Rect::from_min_max(inset.min, inset.max - bounds.size());
    let padding = gap * 0.5 + 0.01;
    let axes = |origin: f32, low: f32, high: f32, size: f32, horizontal: bool| {
        let mut positions = vec![origin, low, high];
        for other in occupied {
            let (start, end) = if horizontal { (other.left(), other.right()) } else { (other.top(), other.bottom()) };
            positions.extend([(start - size - padding).clamp(low, high), (end + padding).clamp(low, high)]);
        }
        positions.sort_by(|a, b| (a - origin).abs().total_cmp(&(b - origin).abs()).then_with(|| a.total_cmp(b)));
        positions.dedup();
        positions
    };
    let xs = axes(base.left(), limits.left(), limits.right(), base.width(), true);
    let ys = axes(base.top(), limits.top(), limits.bottom(), base.height(), false);
    let distance = |(x, y): (usize, usize)| (xs[x] as f64 - base.left() as f64).powi(2) + (ys[y] as f64 - base.top() as f64).powi(2);
    let mut frontier = vec![(0, 0)];
    let mut seen = std::collections::HashSet::from([(0, 0)]);
    let mut best = (base, overlap(base), 0.0);
    for _ in 0..169 {
        let Some(index) = (0..frontier.len()).min_by(|&a, &b| distance(frontier[a]).total_cmp(&distance(frontier[b]))
            .then_with(|| frontier[a].cmp(&frontier[b]))) else { break; };
        let (x, y) = frontier.swap_remove(index);
        let candidate = Rect::from_min_size(pos2(xs[x], ys[y]), bounds.size());
        let area = overlap(candidate);
        let distance = distance((x, y));
        if area == 0.0 { return candidate; }
        if area < best.1 || (area == best.1 && distance < best.2) { best = (candidate, area, distance); }
        for next in [(x + 1, y), (x, y + 1)] {
            if next.0 < xs.len() && next.1 < ys.len() && seen.insert(next) { frontier.push(next); }
        }
    }
    let step = bounds.size() + Vec2::splat(gap);
    for y in -6..=6 {
        for x in -6..=6 {
            let delta = vec2(x as f32 * step.x, y as f32 * step.y);
            let candidate = base.translate(delta);
            if !inset.contains_rect(candidate) { continue; }
            let area = overlap(candidate);
            let distance = delta.x as f64 * delta.x as f64 + delta.y as f64 * delta.y as f64;
            if area < best.1 || (area == best.1 && distance < best.2) { best = (candidate, area, distance); }
        }
    }
    best.0
}

pub(super) fn leader(at: Pos2, bounds: Rect) -> Option<[Pos2; 3]> {
    if bounds.contains(at) { return None; }
    let inset = vec2(8.0_f32.min(bounds.width() * 0.25), 8.0_f32.min(bounds.height() * 0.25));
    let x = at.x.clamp(bounds.left() + inset.x, bounds.right() - inset.x);
    let y = at.y.clamp(bounds.top() + inset.y, bounds.bottom() - inset.y);
    let candidates = [
        (at.x < bounds.left(), pos2(bounds.left(), y), vec2(-8.0, 0.0)),
        (at.x > bounds.right(), pos2(bounds.right(), y), vec2(8.0, 0.0)),
        (at.y < bounds.top(), pos2(x, bounds.top()), vec2(0.0, -8.0)),
        (at.y > bounds.bottom(), pos2(x, bounds.bottom()), vec2(0.0, 8.0)),
    ];
    let (_, end, offset) = candidates.into_iter().filter(|(outside, _, _)| *outside)
        .min_by(|a, b| at.distance_sq(a.1).total_cmp(&at.distance_sq(b.1)))?;
    Some([at, end + offset, end])
}

fn covered_interval(a: Pos2, b: Pos2, rect: Rect) -> Option<(f32, f32)> {
    let delta = b - a;
    let (mut enter, mut leave) = (0.0_f32, 1.0_f32);
    for (origin, direction, lo, hi) in [(a.x, delta.x, rect.left(), rect.right()), (a.y, delta.y, rect.top(), rect.bottom())] {
        if direction.abs() < 1e-6 {
            if origin < lo || origin > hi { return None; }
        } else {
            let (first, last) = ((lo - origin) / direction, (hi - origin) / direction);
            enter = enter.max(first.min(last));
            leave = leave.min(first.max(last));
        }
    }
    (leave > enter).then_some((enter, leave))
}

pub(super) fn visible_leader(at: Pos2, bounds: Rect, occupied: &[Rect]) -> Vec<Vec<Pos2>> {
    let Some(points) = leader(at, bounds) else { return Vec::new(); };
    let mut paths: Vec<Vec<Pos2>> = Vec::new();
    for pair in points.windows(2) {
        let mut covered: Vec<_> = occupied.iter().filter(|r| **r != bounds)
            .filter_map(|r| covered_interval(pair[0], pair[1], r.expand(4.0))).collect();
        covered.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut start = 0.0_f32;
        for (enter, leave) in covered.into_iter().chain(std::iter::once((1.0, 1.0))) {
            if enter > start {
                let (a, b) = (pair[0].lerp(pair[1], start), pair[0].lerp(pair[1], enter));
                if let Some(path) = paths.last_mut().filter(|p| p.last() == Some(&a)) { path.push(b); }
                else { paths.push(vec![a, b]); }
            }
            start = start.max(leave);
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_finds_side_slots_between_lattice_steps() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(640.0, 400.0));
        let size = vec2(256.0, 288.0);
        for mirrored in [false, true] {
            let mirror = |r: Rect| if mirrored {
                Rect::from_min_size(pos2(viewport.right() - r.right(), r.top()), r.size())
            } else { r };
            let occupied = mirror(Rect::from_min_size(pos2(350.0, 8.0), size));
            let initial = mirror(Rect::from_min_size(pos2(224.0, 8.0), size));
            let placed = place(initial, viewport, &[occupied], 16.0);
            assert!(viewport.shrink(8.0).contains_rect(placed));
            assert_eq!(placed.size(), size);
            assert!(!placed.expand(7.99).intersects(occupied), "{placed:?} overlaps {occupied:?}");
            assert!(placed.min.distance(initial.min) < 140.0);
        }
    }

    #[test]
    fn placement_combines_boundaries_from_multiple_neighbors() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(640.0, 400.0));
        let initial = Rect::from_min_size(pos2(210.0, 55.0), vec2(256.0, 180.0));
        let occupied = [
            Rect::from_min_max(pos2(350.0, 8.0), pos2(632.0, 392.0)),
            Rect::from_min_max(pos2(8.0, 8.0), pos2(350.0, 130.0)),
        ];
        let placed = place(initial, viewport, &occupied, 16.0);
        assert!((placed.size() - initial.size()).length() < 0.001);
        assert!(viewport.shrink(8.0).contains_rect(placed));
        assert!(occupied.iter().all(|other| !placed.expand(7.99).intersects(*other)), "{placed:?}");
        assert_eq!(placed, place(initial, viewport, &occupied.into_iter().rev().collect::<Vec<_>>(), 16.0));
    }

    #[test]
    fn boundary_slots_keep_fractional_bounds_inside_offset_viewports() {
        let viewport = Rect::from_min_size(pos2(100.25, 50.75), vec2(650.5, 430.25));
        for i in 0..128 {
            let size = vec2(140.0 + i as f32 * 0.37, 30.0 + i as f32 * 0.21);
            let initial = Rect::from_min_size(viewport.min + vec2(193.13, 126.17), size);
            let placed = place(initial, viewport, &[initial], 8.0);
            assert!(viewport.shrink(8.0).contains_rect(placed), "{i}: {placed:?}");
            assert!((placed.size() - size).length() < 0.001);
            assert!(!placed.expand(3.99).intersects(initial));
            assert_eq!(placed, place(initial, viewport, &[initial], 8.0));
        }
    }

    #[test]
    fn leaders_stay_outside_content_in_all_directions() {
        let bounds = Rect::from_min_size(pos2(100.0, 150.0), vec2(240.0, 100.0));
        for x in [50.0, 99.9, 220.0, 340.1, 400.0] {
            for y in [50.0, 149.9, 200.0, 250.1, 350.0] {
                let at = pos2(x, y);
                if bounds.contains(at) { assert!(leader(at, bounds).is_none()); continue; }
                let path = leader(at, bounds).unwrap();
                assert!(bounds.contains(path[2]));
                assert!(!bounds.shrink(0.01).contains(path[2]));
                for pair in path.windows(2) {
                    for i in 0..100 { assert!(!bounds.shrink(0.01).contains(pair[0].lerp(pair[1], i as f32 / 100.0))); }
                }
            }
        }
    }

    #[test]
    fn leaders_are_occluded_by_neighboring_annotation_bounds() {
        let target = Rect::from_min_size(pos2(100.0, 300.0), vec2(200.0, 50.0));
        let blockers = [Rect::from_min_size(pos2(170.0, 45.0), vec2(60.0, 30.0)),
            Rect::from_min_size(pos2(150.0, 120.0), vec2(100.0, 60.0)),
            Rect::from_min_size(pos2(150.0, 170.0), vec2(100.0, 70.0)), target];
        let paths = visible_leader(pos2(200.0, 50.0), target, &blockers);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths.last().unwrap().last(), Some(&pos2(200.0, 300.0)));
        for pair in paths.iter().flat_map(|p| p.windows(2)) {
            for i in 0..=100 {
                let at = pair[0].lerp(pair[1], i as f32 / 100.0);
                assert!(blockers[..3].iter().all(|r| !r.expand(3.9).contains(at)));
            }
        }
        assert_eq!(visible_leader(pos2(50.0, 250.0), target, &[]).len(), 1);
        assert_eq!(covered_interval(pos2(0.0, 0.0), pos2(10.0, 10.0), target), None);
        assert!(visible_leader(pos2(200.0, 50.0), target, &[target.expand(500.0)]).is_empty());
    }

    #[test]
    fn crowded_placement_is_stable_and_does_not_resize_content() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0));
        let mut occupied = Vec::new();
        for size in [vec2(256.0, 158.0), vec2(256.0, 58.0), vec2(160.0, 42.0), vec2(120.0, 20.0)] {
            let initial = Rect::from_min_size(pos2(990.0, 690.0), size);
            let placed = place(initial, viewport, &occupied, 8.0);
            assert_eq!(placed, place(initial, viewport, &occupied, 8.0));
            assert_eq!(placed.size(), size);
            assert!(viewport.shrink(8.0).contains_rect(placed));
            assert!(occupied.iter().all(|other| !placed.expand(4.0).intersects(*other)));
            occupied.push(placed);
        }
        let oversized = Rect::from_min_size(pos2(-10.0, -20.0), vec2(1200.0, 800.0));
        assert_eq!(place(oversized, viewport, &occupied, 8.0).size(), oversized.size());
    }
}
