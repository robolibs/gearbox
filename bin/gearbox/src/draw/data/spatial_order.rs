use gearbox_api::draw_data::StorageError;

use super::cloud::Geometry;

pub(super) fn order(
    geometry: &Geometry,
    ordinals: &mut [u32],
    check: &impl Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    check()?;
    if ordinals.len() < 2 {
        return Ok(());
    }
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for (index, ordinal) in ordinals.iter().copied().enumerate() {
        if index % 1024 == 0 {
            check()?;
        }
        for (axis, value) in geometry
            .source_position(ordinal as usize)?
            .into_iter()
            .enumerate()
        {
            bounds[0][axis] = bounds[0][axis].min(value);
            bounds[1][axis] = bounds[1][axis].max(value);
        }
    }
    let spans: [f64; 3] = std::array::from_fn(|axis| bounds[1][axis] - bounds[0][axis]);
    let axis = (1..3).fold(0, |axis, candidate| {
        if spans[candidate] > spans[axis] {
            candidate
        } else {
            axis
        }
    });
    if spans[axis] == 0.0 {
        ordinals.sort_unstable_by_key(|ordinal| geometry.source_id(*ordinal as usize).unwrap());
        return check();
    }
    let middle = ordinals.len() / 2;
    ordinals.select_nth_unstable_by(middle, |a, b| {
        let position = |ordinal: u32| geometry.source_position(ordinal as usize).unwrap()[axis];
        position(*a).total_cmp(&position(*b)).then_with(|| {
            geometry
                .source_id(*a as usize)
                .cmp(&geometry.source_id(*b as usize))
        })
    });
    let (left, right) = ordinals.split_at_mut(middle);
    order(geometry, left, check)?;
    order(geometry, right, check)
}
