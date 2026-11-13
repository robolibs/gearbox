use serde_json::Value;

fn triple(value: &Value) -> Result<[f64; 3], String> {
    let values = value.as_array().filter(|v| v.len() == 3).ok_or("focus needs three coordinates")?;
    let mut result = [0.0; 3];
    for (out, value) in result.iter_mut().zip(values) {
        *out = value.as_f64().filter(|v| v.is_finite()).ok_or("focus coordinates must be finite")?;
    }
    Ok(result)
}

fn axes(bearing: f64, pitch: f64) -> [[f64; 3]; 3] {
    let (b, p) = (bearing.to_radians(), pitch.to_radians());
    [[b.cos(), -b.sin(), 0.0], [-b.sin() * p.sin(), -b.cos() * p.sin(), p.cos()],
        [b.sin() * p.cos(), b.cos() * p.cos(), p.sin()]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 { a.into_iter().zip(b).map(|(a, b)| a * b).sum() }

pub(super) fn camera(manifest: &Value, target: &str, width: f64, height: f64, fov: f64) -> Result<Value, String> {
    if ![width, height, fov].into_iter().all(|v| v.is_finite() && v > 0.0) || fov >= 180.0 {
        return Err("focus viewport and field of view must be finite and positive".into());
    }
    let mut camera = if target == "overview" { manifest.get("overview") } else {
        ["sections", "slots"].into_iter().filter_map(|key| manifest[key].as_array())
            .flatten().find(|entry| entry["id"] == target)
    }.cloned().ok_or_else(|| format!("unknown exhibition focus: {target}"))?;
    if manifest["scene"] != "exhibition" && camera.get("bounds").is_none() { return Ok(camera); }
    let bounds = camera.get("bounds").or_else(|| (target == "overview").then(|| &manifest["bounds"]))
        .ok_or("exhibition focus has no bounds")?;
    let (lo, hi, focus) = (triple(&bounds["min"])?, triple(&bounds["max"])?, triple(&camera["focus"])?);
    if (0..3).any(|axis| lo[axis] > hi[axis]) { return Err("focus bounds are reversed".into()); }
    let bearing = camera["bearing"].as_f64().ok_or("focus bearing required")?;
    let pitch = camera["pitch"].as_f64().ok_or("focus pitch required")?;
    let [right, up, eye] = axes(bearing, pitch);
    let vertical = (fov.to_radians() * 0.5).tan() * 0.90;
    let horizontal = vertical * width / height;
    let mut distance = camera["distance"].as_f64().ok_or("focus distance required")?;
    for corner in 0..8 {
        let p = std::array::from_fn(|axis| (if corner & (1 << axis) == 0 { lo[axis] } else { hi[axis] }) - focus[axis]);
        let lateral = (dot(p, right).abs() / horizontal).max(dot(p, up).abs() / vertical).max(0.1);
        distance = distance.max(dot(p, eye) + lateral);
    }
    camera["distance"] = serde_json::json!(distance);
    Ok(camera)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_bounds_fit_wide_square_and_portrait_views() {
        let manifest = super::super::exhibition::manifest("fixture");
        for (width, height) in [(1600.0, 1000.0), (1000.0, 1000.0), (912.0, 1000.0), (600.0, 1200.0)] {
            for (target, bounds) in std::iter::once(("overview", &manifest["bounds"]))
                .chain(["sections", "slots"].into_iter().flat_map(|key| manifest[key].as_array().unwrap())
                    .map(|entry| (entry["id"].as_str().unwrap(), &entry["bounds"]))) {
                let camera = camera(&manifest, target, width, height, 58.0).unwrap();
                let focus = triple(&camera["focus"]).unwrap();
                let lo = triple(&bounds["min"]).unwrap();
                let hi = triple(&bounds["max"]).unwrap();
                let [right, up, eye] = axes(camera["bearing"].as_f64().unwrap(), camera["pitch"].as_f64().unwrap());
                for corner in 0..8 {
                    let p = std::array::from_fn(|axis| (if corner & (1 << axis) == 0 { lo[axis] } else { hi[axis] }) - focus[axis]);
                    let depth = camera["distance"].as_f64().unwrap() - dot(p, eye);
                    let half = depth * 29f64.to_radians().tan();
                    assert!(depth > 0.0);
                    assert!(dot(p, right).abs() / (half * width / height) <= 0.900001, "{target}: horizontal clipping");
                    assert!(dot(p, up).abs() / half <= 0.900001, "{target}: vertical clipping");
                }
            }
        }
        let narrow = camera(&manifest, "styles", 912.0, 1000.0, 58.0).unwrap();
        assert!(narrow["distance"].as_f64().unwrap() > 30.0);
    }

    #[test]
    fn focus_rejects_invalid_inputs_and_preserves_non_catalogue_views() {
        let manifest = super::super::exhibition::manifest("fixture");
        for (width, height, fov) in [(0.0, 100.0, 58.0), (100.0, f64::NAN, 58.0), (100.0, 100.0, 180.0)] {
            assert!(camera(&manifest, "overview", width, height, fov).is_err());
        }
        assert!(camera(&manifest, "missing", 1600.0, 1000.0, 58.0).is_err());
        let legacy = serde_json::json!({"overview": {"focus":[1,2,3], "distance":7, "pitch":30, "bearing":180}});
        assert_eq!(camera(&legacy, "overview", 600.0, 1000.0, 58.0).unwrap(), legacy["overview"]);
    }

    #[test]
    fn bounded_review_scenes_fit_portrait_windows() {
        for scene in super::super::review::manifest()["scenes"].as_array().unwrap() {
            let manifest = serde_json::json!({"overview": scene});
            let wide = camera(&manifest, "overview", 1600.0, 1000.0, 58.0).unwrap();
            let portrait = camera(&manifest, "overview", 828.0, 1348.0, 58.0).unwrap();
            assert!(portrait["distance"].as_f64().unwrap() > wide["distance"].as_f64().unwrap());
            assert_eq!(portrait["focus"], scene["focus"]);
            let (lo, hi, focus) = (triple(&scene["bounds"]["min"]).unwrap(), triple(&scene["bounds"]["max"]).unwrap(), triple(&scene["focus"]).unwrap());
            let [right, up, eye] = axes(scene["bearing"].as_f64().unwrap(), scene["pitch"].as_f64().unwrap());
            for corner in 0..8 {
                let p = std::array::from_fn(|axis| (if corner & (1 << axis) == 0 { lo[axis] } else { hi[axis] }) - focus[axis]);
                let half = (portrait["distance"].as_f64().unwrap() - dot(p, eye)) * 29f64.to_radians().tan();
                assert!(dot(p, right).abs() / (half * 828.0 / 1348.0) <= 0.900001);
                assert!(dot(p, up).abs() / half <= 0.900001);
            }
        }
    }
}
