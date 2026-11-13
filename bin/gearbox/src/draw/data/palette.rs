use gearbox_api::{
    draw_data::StorageError,
    wire::draw_data::{Attribute, ColumnView, Normalization, ScalarType},
};

use super::cloud::exact_u64;

pub(super) fn indices(value: f64) -> u32 {
    let rounded = value.round();
    let raw = rounded.clamp(0.0, 255.0) as u32;
    let map = if rounded < 0.0 { 255 } else { raw };
    let okabe = rounded.rem_euclid(8.0) as u32;
    map | (raw << 8) | (okabe << 16)
}

pub(super) fn attribute_indices(
    column: ColumnView<'_>,
    field: &Attribute,
    index: usize,
    normalized: f64,
) -> Result<u32, StorageError> {
    if field.scalar == ScalarType::U64 && field.normalization == Normalization::None {
        let value = exact_u64(column, index)?.unwrap_or(0);
        let byte = value.min(255) as u32;
        Ok(byte | (byte << 8) | ((value % 8) as u32) << 16)
    } else {
        Ok(indices(normalized))
    }
}
