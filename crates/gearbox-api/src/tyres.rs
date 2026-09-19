use crate::Props;

/// Gauge-bar pressure telemetry for one registered tyre.
#[derive(Clone, Debug, PartialEq)]
pub struct TyrePressure {
    pub link: String,
    pub axle: u16,
    pub applied: f64,
    pub target: f64,
    pub min: f64,
    pub max: f64,
}

impl TyrePressure {
    pub fn read(link: &str, mut get: impl FnMut(&str) -> Option<f64>) -> Option<Self> {
        let axle = get("tyre_axle")?;
        let tyre = Self {
            link: link.into(),
            axle: axle as u16,
            applied: get("tyre_pressure_bar")?,
            target: get("tyre_target_pressure_bar")?,
            min: get("tyre_min_pressure_bar")?,
            max: get("tyre_max_pressure_bar")?,
        };
        (axle >= 1.0
            && axle <= u16::MAX as f64
            && axle.fract() == 0.0
            && [tyre.applied, tyre.target, tyre.min, tyre.max]
                .iter()
                .all(|v| v.is_finite())
            && tyre.min > 0.0
            && tyre.max >= tyre.min)
            .then_some(tyre)
    }
}

pub fn from_props(props: &Props) -> Result<Vec<TyrePressure>, String> {
    let mut tyres = Vec::new();
    for (key, _) in props.iter() {
        let Some(link) = key
            .strip_prefix("link.")
            .and_then(|k| k.strip_suffix(".tyre_target_pressure_bar"))
        else {
            continue;
        };
        tyres.push(
            TyrePressure::read(link, |name| {
                props.get(&format!("link.{link}.{name}"))?.parse().ok()
            })
            .ok_or_else(|| format!("incomplete tyre pressure telemetry for {link}"))?,
        );
    }
    tyres.sort_by(|a, b| a.link.cmp(&b.link));
    Ok(tyres)
}

/// Selects all tyres, `axle:N` (one-based) or `wheel:LINK`, then validates the entire edit.
pub fn targets(tyres: &[TyrePressure], scope: &str, bar: f64) -> Result<Vec<String>, String> {
    if !bar.is_finite() || bar <= 0.0 {
        return Err("pressure must be finite positive gauge bar".into());
    }
    let axle = scope.strip_prefix("axle:").map(|n| n.parse::<u16>());
    if axle
        .as_ref()
        .is_some_and(|n| !n.as_ref().is_ok_and(|n| *n > 0))
    {
        return Err("axle must be an integer from 1 to 65535".into());
    }
    if scope != "all" && axle.is_none() && !scope.starts_with("wheel:") {
        return Err("pressure scope must be all, axle:N or wheel:LINK".into());
    }
    let selected: Vec<_> = tyres
        .iter()
        .filter(|t| {
            scope == "all"
                || axle
                    .as_ref()
                    .is_some_and(|n| n.as_ref().is_ok_and(|n| *n == t.axle))
                || scope.strip_prefix("wheel:") == Some(t.link.as_str())
        })
        .collect();
    if selected.is_empty() {
        return Err(format!("no registered pressure tyres in {scope}"));
    }
    for tyre in &selected {
        if !(tyre.min..=tyre.max).contains(&bar) {
            return Err(format!(
                "{} requires {}..={} gauge bar",
                tyre.link, tyre.min, tyre.max
            ));
        }
    }
    Ok(selected.into_iter().map(|t| t.link.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_validate_all_members_without_partial_selection() {
        let tyres: Vec<_> = [("fl", 1, 3.0), ("fr", 1, 2.0), ("rear", 2, 4.0)]
            .into_iter()
            .map(|(name, axle, max)| TyrePressure {
                link: name.into(),
                axle,
                min: 0.5,
                max,
                applied: 1.8,
                target: 1.8,
            })
            .collect();
        assert_eq!(targets(&tyres, "axle:1", 1.0).unwrap(), ["fl", "fr"]);
        assert_eq!(targets(&tyres, "wheel:rear", 4.0).unwrap(), ["rear"]);
        assert_eq!(targets(&tyres, "all", 2.0).unwrap().len(), 3);
        for scope in ["all", "axle:1"] {
            assert!(targets(&tyres, scope, 2.5).is_err());
        }
        for scope in ["axle:0", "axle:1.5", "axle:65536", "wheel:no", "front"] {
            assert!(targets(&tyres, scope, 1.0).is_err());
        }
        for pressure in [f64::NAN, f64::INFINITY, -1.0, 0.0, 0.49] {
            assert!(targets(&tyres, "all", pressure).is_err());
        }
    }

    #[test]
    fn missing_group_telemetry_is_an_error_not_a_smaller_group() {
        let props = Props::from_pairs(&[("link.left.tyre_target_pressure_bar", "1.8")]);
        assert!(from_props(&props).is_err());
    }
}
