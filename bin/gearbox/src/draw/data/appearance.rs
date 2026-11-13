use std::sync::Arc;

use bevy::math::{DQuat, DVec3};
use gearbox_api::draw_data::{Charge, CpuBudget, MemoryClass, StorageError};
use gearbox_api::wire::draw_data::*;

use super::cloud::{Geometry, Records, exact_u64, invalid, normalized, valid};

pub(super) const COLOR_VALID: u32 = 1;
pub(super) const VALUE_VALID: u32 = 2;
pub(super) const CATEGORY_VALID: u32 = 4;
pub(super) const RADIUS_VALID: u32 = 8;

#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct Sample {
    pub rgba: u32,
    pub value: f32,
    pub category: u32,
    pub radius_m: f32,
    pub flags: u32,
    pub acquisition_ns: [u32; 2],
    pub palette_indices: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Name {
    bytes: [u8; MAX_ATTRIBUTE_NAME_BYTES],
    len: u8,
}

impl Name {
    fn new(value: &str) -> Result<Self, StorageError> {
        if value.is_empty()
            || value.len() > MAX_ATTRIBUTE_NAME_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(invalid("invalid appearance attribute name"));
        }
        let mut name = Self {
            bytes: [0; MAX_ATTRIBUTE_NAME_BYTES],
            len: value.len() as u8,
        };
        name.bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(name)
    }
}

impl std::ops::Deref for Name {
    type Target = str;

    fn deref(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).expect("validated UTF-8 name")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Selector {
    Constant,
    Attribute(Name),
    Scalar(Name, [f64; 2]),
    Category(Name),
    Axis(u8, [f64; 2]),
    Range([f64; 3], [f64; 2]),
    Age,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct AppearanceKey {
    pub selector: Selector,
    pub radius_attribute: Option<Name>,
    pub source_pose: Option<([f64; 3], RigidPose)>,
    pub acquisition_origin_ns: Option<u64>,
}

impl AppearanceKey {
    pub fn new(presentation: &Presentation, frame: &DataFrame) -> Result<Self, StorageError> {
        let selector = match &presentation.color {
            ColorMode::Constant { .. } => Selector::Constant,
            ColorMode::Attribute { name, .. } => Selector::Attribute(Name::new(name)?),
            ColorMode::Scalar { name, range, .. } => Selector::Scalar(Name::new(name)?, *range),
            ColorMode::Category { name, .. } => Selector::Category(Name::new(name)?),
            ColorMode::Axis { axis, range, .. } => Selector::Axis(*axis, *range),
            ColorMode::Range {
                origin_m, range, ..
            } => Selector::Range(*origin_m, *range),
            ColorMode::Age { .. } => Selector::Age,
            _ => return Err(invalid("occupancy presentation cannot shade cloud samples")),
        };
        let source_pose = matches!(selector, Selector::Axis(..) | Selector::Range(..))
            .then_some((frame.origin_m, frame.pose));
        let acquisition_origin_ns = matches!(selector, Selector::Age)
            .then(|| {
                frame
                    .capture
                    .as_ref()
                    .and_then(|capture| capture.source_time_ns)
            })
            .flatten();
        Ok(Self {
            selector,
            radius_attribute: presentation
                .radius_attribute
                .as_deref()
                .map(Name::new)
                .transpose()?,
            source_pose,
            acquisition_origin_ns,
        })
    }
}

pub(super) struct Appearance {
    pub key: AppearanceKey,
    pub samples: Records<Sample>,
    pub coarse: Arc<super::coarse::Samples>,
    pub geometry: Arc<Geometry>,
    pub alpha: super::alpha::Summary,
    pub max_radius_m: f32,
    _charge: Charge,
}

impl Appearance {
    /// Keeps style identity and coarse samples for a hollow page, without full samples.
    pub fn hollow(
        &self,
        budget: &CpuBudget,
        geometry: Arc<Geometry>,
    ) -> Result<Arc<Self>, StorageError> {
        let bytes = (size_of::<Self>() + 2 * size_of::<usize>()) as u64;
        Ok(Arc::new(Self {
            key: self.key.clone(),
            samples: Records::new(budget, 0)?,
            coarse: self.coarse.clone(),
            geometry,
            alpha: self.alpha,
            max_radius_m: self.max_radius_m,
            _charge: budget
                .reserve(bytes)?
                .charge(bytes, MemoryClass::Metadata)?,
        }))
    }

    pub fn prepare(
        budget: &CpuBudget,
        schema: &CloudSchema,
        geometry: Arc<Geometry>,
        presentation: &Presentation,
        frame: &DataFrame,
    ) -> Result<Arc<Self>, StorageError> {
        presentation
            .validate(Some(schema), None, frame.capture.is_some())
            .map_err(invalid)?;
        presentation
            .color
            .validate_capture(frame.capture.as_ref())
            .map_err(invalid)?;
        frame.validate().map_err(invalid)?;
        let key = AppearanceKey::new(presentation, frame)?;
        let field = |name: &str| -> Result<(&Attribute, ColumnView<'_>), StorageError> {
            let index = schema
                .attributes
                .iter()
                .position(|a| a.name == name)
                .ok_or_else(|| invalid("selected attribute missing"))?;
            let column = geometry
                .source()
                .ok_or_else(|| invalid("page payload is not resident"))?
                .column(index)
                .ok_or_else(|| invalid("selected column missing"))?;
            Ok((&schema.attributes[index], column))
        };
        let selected = match &key.selector {
            Selector::Attribute(name) | Selector::Scalar(name, _) | Selector::Category(name) => {
                Some(field(name)?)
            }
            _ => None,
        };
        let radius = key.radius_attribute.as_deref().map(field).transpose()?;
        let time = if matches!(key.selector, Selector::Age) {
            let mut fields = schema
                .attributes
                .iter()
                .filter(|a| a.semantic == AttributeSemantic::AcquisitionOffset);
            let first = fields.next();
            if fields.next().is_some() {
                return Err(invalid(
                    "age display requires an unambiguous acquisition offset",
                ));
            }
            first.map(|attribute| field(&attribute.name)).transpose()?
        } else {
            None
        };
        let bytes = (size_of::<Self>() + 2 * size_of::<usize>()) as u64;
        let charge = budget
            .reserve(bytes)?
            .charge(bytes, MemoryClass::Metadata)?;
        let mut samples = Records::<Sample>::new(budget, geometry.positions.values.len())?;
        let rotation = frame.pose.rotation_wxyz;
        let rotation = DQuat::from_xyzw(rotation[1], rotation[2], rotation[3], rotation[0]);
        let translation = DVec3::from_array(frame.pose.translation_m);
        let origin =
            DVec3::from_array(frame.origin_m) + DVec3::from_array(geometry.descriptor().origin_m);
        for (render_index, sample) in samples.values.iter_mut().enumerate() {
            let index = geometry.positions.values[render_index].source_index as usize;
            if let Some((attribute, column)) = radius
                && let Some(value) = normalized(column, attribute, index)?
            {
                sample.radius_m = value as f32;
                if !sample.radius_m.is_finite() || sample.radius_m <= 0.0 {
                    return Err(invalid("metric radius cannot be represented on GPU"));
                }
                sample.flags |= RADIUS_VALID;
            }
            match &key.selector {
                Selector::Constant => {}
                Selector::Attribute(_) => {
                    let (_, column) = selected.unwrap();
                    if valid(column, index) {
                        let raw = column
                            .values
                            .get(index * 4..index * 4 + 4)
                            .ok_or_else(|| invalid("short RGBA column"))?;
                        sample.rgba = u32::from_le_bytes(raw.try_into().unwrap());
                        sample.flags |= COLOR_VALID;
                    }
                }
                Selector::Scalar(_, range) => {
                    let (attribute, column) = selected.unwrap();
                    if let Some(value) = normalized(column, attribute, index)? {
                        sample.value = display_coordinate(value, *range);
                        sample.palette_indices =
                            super::palette::attribute_indices(column, attribute, index, value)?;
                        sample.flags |= VALUE_VALID;
                    }
                }
                Selector::Category(_) => {
                    let (attribute, column) = selected.unwrap();
                    if let Some(value) = normalized(column, attribute, index)? {
                        sample.category = value as u32;
                        sample.flags |= CATEGORY_VALID;
                    }
                }
                Selector::Axis(_, range) | Selector::Range(_, range) => {
                    let point = rotation
                        * (origin + DVec3::from_array(geometry.source_position(render_index)?))
                        + translation;
                    if !point.is_finite() {
                        return Err(invalid("nonfinite source-frame color position"));
                    }
                    let value = match &key.selector {
                        Selector::Axis(axis, _) => point[*axis as usize],
                        Selector::Range(origin_m, _) => {
                            let delta = point - DVec3::from_array(*origin_m);
                            delta.x.hypot(delta.y).hypot(delta.z)
                        }
                        _ => unreachable!(),
                    };
                    if !value.is_finite() {
                        return Err(invalid("nonfinite source-frame range"));
                    }
                    sample.value = display_coordinate(value, *range);
                    sample.palette_indices = super::palette::indices(value);
                    sample.flags |= VALUE_VALID;
                }
                Selector::Age => {
                    let offset_ns = match time {
                        Some((_, column)) => match exact_u64(column, index)? {
                            Some(value) => {
                                if let Some(capture) = &frame.capture
                                    && capture.source_time_ns.is_some()
                                {
                                    capture.source_sample_ns(value).map_err(invalid)?;
                                }
                                value
                            }
                            None => continue,
                        },
                        None => 0,
                    };
                    sample.acquisition_ns = [offset_ns as u32, (offset_ns >> 32) as u32];
                    sample.flags |= VALUE_VALID;
                }
            }
        }
        let alpha = super::alpha::Summary::new(&samples.values);
        let max_radius_m = samples
            .values
            .iter()
            .filter(|sample| sample.flags & RADIUS_VALID != 0)
            .map(|sample| sample.radius_m)
            .fold(0.0, f32::max);
        Ok(Arc::new(Self {
            key,
            coarse: super::coarse::Samples::build(budget, &geometry.coarse, &samples.values)?,
            samples,
            geometry,
            alpha,
            max_radius_m,
            _charge: charge,
        }))
    }
}

pub(super) fn display_coordinate(value: f64, range: [f64; 2]) -> f32 {
    if range[0] == range[1] {
        return 0.5;
    }
    if value <= range[0] {
        return 0.0;
    }
    if value >= range[1] {
        return 1.0;
    }
    let width = range[1] - range[0];
    let t = if width.is_finite() {
        (value - range[0]) / width
    } else {
        (value * 0.5 - range[0] * 0.5) / (range[1] * 0.5 - range[0] * 0.5)
    };
    t.clamp(0.0, 1.0) as f32
}

#[cfg(test)]
#[path = "appearance_tests.rs"]
mod tests;
