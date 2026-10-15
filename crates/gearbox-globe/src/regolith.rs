use bevy::math::DVec3;
use crate::{Body, regolith_noise as noise};

pub fn crater_profile(distance_in_radii: f64) -> f64 {
    let q = distance_in_radii;
    if q >= 1.6 { return 0.0; }
    let bowl = if q < 1.0 { -0.19 * (1.0 - q * q).powi(2) } else { 0.0 };
    let rim = 0.048 * (-((q - 1.0) / 0.14).powi(2)).exp();
    let taper = ((1.6 - q) / 0.25).clamp(0.0, 1.0);
    bowl + rim * taper * taper * (3.0 - 2.0 * taper)
}

fn craters(p: DVec3, cell_m: f64, density: f32) -> f32 {
    let cell = (p / cell_m).floor();
    let radius = p.length();
    let mut height = 0.0;
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let c = cell + DVec3::new(x as f64, y as f64, z as f64);
                let (i, j, k) = (c.x as i64, c.y as i64, c.z as i64);
                let seed = noise::hash(i, j, k);
                if seed > density { continue; }
                let jitter = DVec3::new(seed as f64, noise::hash(i + 913, j, k) as f64, noise::hash(i, j + 719, k) as f64);
                let centre = (c + jitter * 0.7 + DVec3::splat(0.15)) * cell_m;
                if (centre.length() - radius).abs() > cell_m * 0.45 { continue; }
                let centre = centre.normalize() * radius;
                let r = cell_m * (0.12 + 0.20 * noise::hash(i, j, k + 331) as f64);
                height += crater_profile(p.distance(centre) / r) * r;
            }
        }
    }
    height as f32
}

pub fn height(body: Body, p: DVec3) -> f32 {
    let broad = (noise::fbm(p * 0.000055, 4) - 0.5) * 420.0;
    let hills = (noise::fbm(p * 0.0013 + DVec3::splat(71.0), 4) - 0.5) * 28.0;
    let grit = (noise::fbm(p * 0.06, 3) - 0.5) * 0.18;
    match body {
        Body::Earth => 0.0,
        Body::Mars => {
            let wind = p.dot(DVec3::new(0.53, 0.21, 0.82));
            let dunes = (wind * 0.30 + 2.0 * noise::value(p * 0.02) as f64).sin() as f32 * 0.32;
            broad + hills + grit + dunes + craters(p, 95.0, 0.35) + craters(p, 700.0, 0.55) + craters(p, 9_000.0, 0.6)
                + craters(p, 180_000.0, 0.55) * 0.25 + craters(p, 900_000.0, 0.4) * 0.1
                + (noise::fbm(p * 0.000001, 4) - 0.5) * 3_600.0
        }
        Body::Moon => broad * 0.75 + hills * 0.55 + grit
            + craters(p, 65.0, 0.8) + craters(p, 450.0, 0.9) + craters(p, 6_500.0, 0.95)
            + craters(p, 120_000.0, 0.85) * 0.7 + craters(p, 550_000.0, 0.75) * 0.3
            + (noise::fbm(p * 0.0000016, 4) - 0.5) * 1_200.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_has_bowl_raised_rim_and_finite_ejecta_extent() {
        assert!(crater_profile(0.0) < -0.18);
        assert!(crater_profile(1.0) > 0.04);
        assert_eq!(crater_profile(1.6), 0.0);
        assert_eq!(crater_profile(2.0), 0.0);
    }

    #[test]
    fn surfaces_are_deterministic_distinct_and_not_flat() {
        let mut ranges = Vec::new();
        for body in [Body::Mars, Body::Moon] {
            let (lat, lon) = body.home();
            let datum = crate::Datum::on(body, lat, lon);
            let mut low = f32::INFINITY;
            let mut high = f32::NEG_INFINITY;
            for x in -20..=20 {
                for z in -20..=20 {
                    let point = datum.to_ecef(DVec3::new(x as f64 * 25.0, 0.0, z as f64 * 25.0)).normalize() * body.radius_m();
                    let h = height(body, point);
                    assert!(h.is_finite());
                    assert_eq!(h, height(body, point));
                    low = low.min(h); high = high.max(h);
                }
            }
            assert!(high - low > 4.0, "{body:?}: {low}..{high}");
            ranges.push((low, high));
        }
        assert_ne!(ranges[0], ranges[1]);
    }
}
