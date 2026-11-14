use std::sync::Arc;

use bevy::math::{DQuat, DVec3, UVec4};
use gearbox_api::draw_data::{Charge, CpuBudget, MemoryClass, StorageError};
use gearbox_api::wire::draw_data::*;

use super::cloud::{Geometry, Records, exact_u64, invalid, normalized, valid};

pub(super) const COLOR_VALID: u32 = 1;
pub(super) const VALUE_VALID: u32 = 2;
pub(super) const CATEGORY_VALID: u32 = 4;
pub(super) const RADIUS_VALID: u32 = 8;

/// One point's attributes before packing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Sample {
    pub rgba: u32,
    pub value: f32,
    pub category: u32,
    pub radius_m: f32,
    pub flags: u32,
    pub acquisition_ns: [u32; 2],
    pub palette_indices: u32,
}

/// Colour kinds, numbered as the style's `controls.x`.
pub(super) const KIND_RGBA: u32 = 1;
pub(super) const KIND_VALUE: u32 = 2;
pub(super) const KIND_CATEGORY: u32 = 3;
pub(super) const KIND_AGE: u32 = 4;

/// Ramp positions in 8.12 fixed point: 4096 steps between neighbouring ramp entries.
const RAMP_STEPS: f64 = 4096.0;
const RAMP_POSITION_BITS: u32 = 0xf_ffff;

/// Packs a display coordinate in [0, 1] and its palette indices into one value word:
/// ramp position (bits 0-19), raw palette byte (20-27), Okabe slot (28-30), below zero (31).
pub(super) fn value_word(value: f32, indices: u32) -> u32 {
    let position = (f64::from(value.clamp(0.0, 1.0)) * 255.0 * RAMP_STEPS).round() as u32;
    let raw = (indices >> 8) & 255;
    let below = u32::from(indices & 255 != raw);
    position | raw << 20 | ((indices >> 16) & 7) << 28 | below << 31
}

/// The display coordinate and `map | raw << 8 | okabe << 16` indices of a value word.
pub(super) fn value_parts(word: u32) -> (f32, u32) {
    let raw = (word >> 20) & 255;
    let map = if word >> 31 != 0 { 255 } else { raw };
    let value = (f64::from(word & RAMP_POSITION_BITS) / (255.0 * RAMP_STEPS)) as f32;
    (value, map | raw << 8 | ((word >> 28) & 7) << 16)
}

/// Where one appearance's points keep their attribute words.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Layout {
    pub count: usize,
    pub kind: u32,
    /// Colour words per point; the radius word follows them when stored.
    pub colors: u32,
    pub radius: bool,
    /// One validity bit per point after the point words.
    pub mask: bool,
}

impl Layout {
    pub fn new(selector: &Selector, radius: bool, count: usize) -> Self {
        let (kind, colors) = match selector {
            Selector::Constant => (0, 0),
            Selector::Attribute(_) => (KIND_RGBA, 1),
            Selector::Scalar(..) | Selector::Axis(..) | Selector::Range(..) => (KIND_VALUE, 1),
            Selector::Category(_) => (KIND_CATEGORY, 1),
            Selector::Age => (KIND_AGE, 2),
        };
        Self { count, kind, colors, radius, mask: colors > 0 && kind != KIND_RGBA }
    }

    pub fn stride(self) -> usize {
        (self.colors + u32::from(self.radius)) as usize
    }

    fn mask_start(self) -> Option<usize> {
        self.mask.then(|| self.count * self.stride())
    }

    pub fn words(self) -> usize {
        self.count * self.stride() + if self.mask { self.count.div_ceil(32) } else { 0 }
    }

    pub fn bytes(self) -> u64 {
        self.words() as u64 * 4
    }

    /// x: words per point, y: colour kind, z: 1 when a radius word ends each point,
    /// w: the first validity word, or `u32::MAX` when every point is valid.
    pub fn uniform(self) -> UVec4 {
        let mask = self.mask_start().map_or(u32::MAX, |start| start as u32);
        UVec4::new(self.stride() as u32, self.kind, u32::from(self.radius), mask)
    }
}

/// Packed per-point attribute words, uploaded to the cloud shader as they are.
#[derive(Debug)]
pub(super) struct Attributes {
    pub layout: Layout,
    pub words: Records<u32>,
}

impl Attributes {
    pub fn new(budget: &CpuBudget, layout: Layout) -> Result<Self, StorageError> {
        Ok(Self { layout, words: Records::new(budget, layout.words())? })
    }

    /// Keeps the layout of a hollow page without its words.
    pub fn hollow(&self, budget: &CpuBudget) -> Result<Self, StorageError> {
        Ok(Self { layout: self.layout, words: Records::new(budget, 0)? })
    }

    pub fn bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.words.values)
    }

    pub fn contains(&self, index: usize) -> bool {
        index < self.layout.count && self.words.values.len() == self.layout.words()
    }

    pub fn set(&mut self, index: usize, sample: &Sample) {
        let layout = self.layout;
        let base = index * layout.stride();
        let words = &mut self.words.values;
        let valid = match layout.kind {
            KIND_RGBA => {
                words[base] = if sample.flags & COLOR_VALID != 0 { sample.rgba } else { 0 };
                true
            }
            KIND_VALUE => {
                words[base] = value_word(sample.value, sample.palette_indices);
                sample.flags & VALUE_VALID != 0
            }
            KIND_CATEGORY => {
                words[base] = sample.category;
                sample.flags & CATEGORY_VALID != 0
            }
            KIND_AGE => {
                words[base..base + 2].copy_from_slice(&sample.acquisition_ns);
                sample.flags & VALUE_VALID != 0
            }
            _ => true,
        };
        if layout.radius && sample.flags & RADIUS_VALID != 0 {
            words[base + layout.colors as usize] = sample.radius_m.to_bits();
        }
        if let Some(start) = layout.mask_start()
            && valid
        {
            words[start + index / 32] |= 1 << (index % 32);
        }
    }

    pub fn radius_m(&self, index: usize) -> Option<f32> {
        if !self.layout.radius {
            return None;
        }
        let word = self.words.values.get(index * self.layout.stride() + self.layout.colors as usize)?;
        (*word != 0).then(|| f32::from_bits(*word))
    }

    /// The sample as the shader reads it back; invalid fields read as zero.
    pub fn sample(&self, index: usize) -> Option<Sample> {
        if !self.contains(index) {
            return None;
        }
        let layout = self.layout;
        let words = &self.words.values[index * layout.stride()..(index + 1) * layout.stride()];
        let valid = layout
            .mask_start()
            .is_none_or(|start| self.words.values[start + index / 32] >> (index % 32) & 1 != 0);
        let mut sample = Sample::default();
        match layout.kind {
            KIND_RGBA if words[0] != 0 => {
                sample.rgba = words[0];
                sample.flags |= COLOR_VALID;
            }
            KIND_VALUE if valid => {
                (sample.value, sample.palette_indices) = value_parts(words[0]);
                sample.flags |= VALUE_VALID;
            }
            KIND_CATEGORY if valid => {
                sample.category = words[0];
                sample.flags |= CATEGORY_VALID;
            }
            KIND_AGE if valid => {
                sample.acquisition_ns = [words[0], words[1]];
                sample.flags |= VALUE_VALID;
            }
            _ => {}
        }
        if let Some(radius) = self.radius_m(index) {
            sample.radius_m = radius;
            sample.flags |= RADIUS_VALID;
        }
        Some(sample)
    }

    /// The words of the samples at `ordinals`, in that order.
    pub fn gather(&self, budget: &CpuBudget, ordinals: &[u32]) -> Result<Self, StorageError> {
        let mut result = Self::new(budget, Layout { count: ordinals.len(), ..self.layout })?;
        let stride = self.layout.stride();
        let masks = self.layout.mask_start().zip(result.layout.mask_start());
        for (index, ordinal) in ordinals.iter().map(|o| *o as usize).enumerate() {
            if !self.contains(ordinal) {
                return Err(invalid("coarse attribute outside source page"));
            }
            result.words.values[index * stride..(index + 1) * stride]
                .copy_from_slice(&self.words.values[ordinal * stride..(ordinal + 1) * stride]);
            if let Some((from, to)) = masks
                && self.words.values[from + ordinal / 32] >> (ordinal % 32) & 1 != 0
            {
                result.words.values[to + index / 32] |= 1 << (index % 32);
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Name {
    bytes: [u8; MAX_ATTRIBUTE_NAME_BYTES],
    len: u8,
}

impl Name {
    pub(super) fn new(value: &str) -> Result<Self, StorageError> {
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
    pub attributes: Attributes,
    pub coarse: Arc<super::coarse::Samples>,
    pub geometry: Arc<Geometry>,
    pub alpha: super::alpha::Summary,
    pub max_radius_m: f32,
    _charge: Charge,
}

impl Appearance {
    #[cfg(test)]
    pub fn samples(&self) -> Vec<Sample> {
        (0..self.attributes.layout.count).filter_map(|i| self.attributes.sample(i)).collect()
    }

    /// Keeps style identity and coarse samples for a hollow page, without full samples.
    pub fn hollow(
        &self,
        budget: &CpuBudget,
        geometry: Arc<Geometry>,
    ) -> Result<Arc<Self>, StorageError> {
        let bytes = (size_of::<Self>() + 2 * size_of::<usize>()) as u64;
        Ok(Arc::new(Self {
            key: self.key.clone(),
            attributes: self.attributes.hollow(budget)?,
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
        let count = geometry.positions.values.len();
        let layout = Layout::new(&key.selector, radius.is_some(), count);
        let mut attributes = Attributes::new(budget, layout)?;
        let mut alpha = super::alpha::Summary::default();
        let mut max_radius_m = 0.0f32;
        let rotation = frame.pose.rotation_wxyz;
        let rotation = DQuat::from_xyzw(rotation[1], rotation[2], rotation[3], rotation[0]);
        let translation = DVec3::from_array(frame.pose.translation_m);
        let origin =
            DVec3::from_array(frame.origin_m) + DVec3::from_array(geometry.descriptor().origin_m);
        for render_index in 0..count {
            let mut sample = Sample::default();
            let index = geometry
                .source_row(render_index)
                .ok_or_else(|| invalid("render sample is out of bounds"))?;
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
                        Some((_, column)) => exact_u64(column, index)?,
                        None => Some(0),
                    };
                    if let Some(offset_ns) = offset_ns {
                        if time.is_some()
                            && let Some(capture) = &frame.capture
                            && capture.source_time_ns.is_some()
                        {
                            capture.source_sample_ns(offset_ns).map_err(invalid)?;
                        }
                        sample.acquisition_ns = [offset_ns as u32, (offset_ns >> 32) as u32];
                        sample.flags |= VALUE_VALID;
                    }
                }
            }
            attributes.set(render_index, &sample);
            alpha.add(&sample);
            if sample.flags & RADIUS_VALID != 0 {
                max_radius_m = max_radius_m.max(sample.radius_m);
            }
        }
        Ok(Arc::new(Self {
            key,
            coarse: super::coarse::Samples::build(budget, &geometry.coarse, &attributes)?,
            attributes,
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
