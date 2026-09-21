//! Where the view stands, for anyone outside the simulator to read.
//!
//! The viewer owns the camera and says where it is; the host bus hands that on
//! in `/gearbox/info`, so `gearbox instance camera where` can answer without
//! reaching into the ECS. A place nobody has published yet is `None`, not a
//! zero — latitude zero, longitude zero is a real place in the Atlantic.

use std::sync::RwLock;

/// Latitude and longitude in degrees, the eye's height above the ground under
/// it, and how far it stands from what it is looking at, both in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewPlace {
    pub latitude: f64,
    pub longitude: f64,
    pub height_m: f64,
    pub distance_m: f64,
}

static PLACE: RwLock<Option<ViewPlace>> = RwLock::new(None);

/// Says where the view is now.
pub fn set_view_place(place: ViewPlace) {
    if let Ok(mut slot) = PLACE.write() {
        *slot = Some(place);
    }
}

/// Where the view was last said to be.
pub fn view_place() -> Option<ViewPlace> {
    PLACE.read().ok().and_then(|slot| *slot)
}

/// How the place travels in `/gearbox/info`: one property, four numbers, so no
/// wire type has to change to carry it.
pub fn view_place_prop() -> String {
    match view_place() {
        Some(place) => format!(
            "{:.6} {:.6} {:.1} {:.1}",
            place.latitude, place.longitude, place.height_m, place.distance_m
        ),
        None => String::new(),
    }
}

/// Reads back what [`view_place_prop`] wrote.
pub fn parse_view_place(text: &str) -> Option<ViewPlace> {
    let mut numbers = text.split_whitespace().map(str::parse::<f64>);
    let mut next = || numbers.next()?.ok();
    Some(ViewPlace {
        latitude: next()?,
        longitude: next()?,
        height_m: next()?,
        distance_m: next()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_survives_the_round_trip_through_a_property() {
        set_view_place(ViewPlace {
            latitude: 51.25,
            longitude: -0.5,
            height_m: 1234.5,
            distance_m: 2000.0,
        });
        let back = parse_view_place(&view_place_prop()).expect("a place was published");
        assert!((back.latitude - 51.25).abs() < 1e-6);
        assert!((back.longitude + 0.5).abs() < 1e-6);
        assert!((back.height_m - 1234.5).abs() < 0.1);
    }

    #[test]
    fn nothing_published_reads_back_as_nothing() {
        assert!(parse_view_place("").is_none());
        assert!(parse_view_place("51.2 -0.5").is_none());
    }
}
