use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{
    MAX_ATTRIBUTE_NAME_BYTES, MAX_ATTRIBUTES, MAX_PAGE_BYTES, MAX_PAGE_POINTS, MAX_SCHEMA_BYTES,
    SCHEMA_REVISION,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    U8,
    U16,
    U32,
    U64,
    I16,
    I32,
    F32,
    F64,
}

impl ScalarType {
    pub const fn size(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::F64 => 8,
        }
    }

    pub const fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Normalization {
    None,
    Unorm,
    Snorm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeSemantic {
    Position,
    SourceId,
    Color,
    Intensity,
    Category,
    Confidence,
    Normal,
    Radius,
    AcquisitionOffset,
    Ring,
    ReturnIndex,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attribute {
    pub name: String,
    pub scalar: ScalarType,
    pub components: u8,
    pub semantic: AttributeSemantic,
    pub units: String,
    pub normalization: Normalization,
    pub nullable: bool,
}

impl Attribute {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty()
            || self.name.len() > MAX_ATTRIBUTE_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err(
                "attribute name must be 1..63 UTF-8 bytes without control characters".into(),
            );
        }
        if self.units.len() > 63 || self.units.chars().any(char::is_control) {
            return Err(format!("{}: invalid units", self.name));
        }
        if !(1..=4).contains(&self.components) {
            return Err(format!("{}: component count must be 1..4", self.name));
        }
        let normalized = match self.normalization {
            Normalization::None => true,
            Normalization::Unorm => matches!(
                self.scalar,
                ScalarType::U8 | ScalarType::U16 | ScalarType::U32
            ),
            Normalization::Snorm => matches!(self.scalar, ScalarType::I16 | ScalarType::I32),
        };
        if !normalized {
            return Err(format!(
                "{}: normalization is incompatible with its scalar type",
                self.name
            ));
        }
        let raw = self.normalization == Normalization::None;
        let metric = self.units == "m" && self.scalar.is_float() && raw;
        let valid = match self.semantic {
            AttributeSemantic::Position => metric && self.components == 3,
            AttributeSemantic::SourceId => {
                self.scalar == ScalarType::U64 && self.components == 1 && raw && !self.nullable
            }
            AttributeSemantic::Color => {
                self.scalar == ScalarType::U8
                    && self.components == 4
                    && self.normalization == Normalization::Unorm
            }
            AttributeSemantic::Normal => self.scalar.is_float() && self.components == 3 && raw,
            AttributeSemantic::Radius => metric && self.components == 1,
            AttributeSemantic::Confidence => self.scalar.is_float() && self.components == 1 && raw,
            AttributeSemantic::AcquisitionOffset => {
                self.scalar == ScalarType::U64 && self.components == 1 && raw && self.units == "ns"
            }
            AttributeSemantic::Category
            | AttributeSemantic::Ring
            | AttributeSemantic::ReturnIndex => {
                matches!(
                    self.scalar,
                    ScalarType::U8 | ScalarType::U16 | ScalarType::U32
                ) && self.components == 1
                    && raw
            }
            AttributeSemantic::Intensity => self.components == 1,
            AttributeSemantic::Custom => true,
        };
        if !valid {
            return Err(format!(
                "{}: type, units, or normalization do not match its semantic",
                self.name
            ));
        }
        Ok(())
    }

    pub fn byte_len(&self, count: u64) -> Result<usize, String> {
        self.validate()?;
        let bytes = count
            .checked_mul(self.components as u64)
            .and_then(|n| n.checked_mul(self.scalar.size() as u64))
            .ok_or_else(|| format!("{}: attribute byte count overflow", self.name))?;
        usize::try_from(bytes)
            .map_err(|_| format!("{}: attribute does not fit address space", self.name))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudSchema {
    pub revision: u32,
    pub attributes: Vec<Attribute>,
}

impl CloudSchema {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision != SCHEMA_REVISION {
            return Err(format!(
                "unsupported cloud schema revision {}",
                self.revision
            ));
        }
        if self.attributes.len() > MAX_ATTRIBUTES {
            return Err("cloud schema exceeds 32 attributes".into());
        }
        for (index, field) in self.attributes.iter().enumerate() {
            field.validate()?;
            if self.attributes[..index]
                .iter()
                .any(|previous| previous.name == field.name)
            {
                return Err(format!("duplicate attribute {}", field.name));
            }
        }
        for semantic in [AttributeSemantic::Position, AttributeSemantic::SourceId] {
            if self
                .attributes
                .iter()
                .filter(|a| a.semantic == semantic)
                .count()
                != 1
            {
                return Err(format!("cloud requires exactly one {semantic:?} attribute"));
            }
        }
        Ok(())
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_SCHEMA_BYTES {
            return Err("cloud schema exceeds 8 KiB".into());
        }
        let schema: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid cloud schema: {e}"))?;
        schema.validate()?;
        Ok(schema)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_SCHEMA_BYTES {
            return Err("cloud schema exceeds 8 KiB".into());
        }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnData {
    pub values: Vec<u8>,
    pub validity: Vec<u8>,
}

impl ColumnData {
    pub fn as_view(&self) -> ColumnView<'_> {
        ColumnView {
            values: &self.values,
            validity: &self.validity,
        }
    }

    pub fn validate(&self, field: &Attribute, count: u32) -> Result<(), String> {
        self.as_view().validate(field, count)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColumnView<'a> {
    pub values: &'a [u8],
    pub validity: &'a [u8],
}

impl ColumnView<'_> {
    /// Empty masks mean all-valid; populated masks use least-significant bit first.
    fn is_valid(&self, index: usize) -> bool {
        self.validity.is_empty()
            || self
                .validity
                .get(index / 8)
                .is_some_and(|b| b & (1 << (index % 8)) != 0)
    }

    pub fn validate(&self, field: &Attribute, count: u32) -> Result<(), String> {
        let expected = field.byte_len(count.into())?;
        if count > MAX_PAGE_POINTS || expected > MAX_PAGE_BYTES || self.values.len() != expected {
            return Err(format!(
                "{}: column length/count exceeds its bounds",
                field.name
            ));
        }
        if !self.validity.is_empty() {
            if !field.nullable || self.validity.len() != (count as usize).div_ceil(8) {
                return Err(format!(
                    "{}: invalid validity mask length or nullability",
                    field.name
                ));
            }
            let used = count % 8;
            if used != 0 && self.validity.last().is_some_and(|b| b >> used != 0) {
                return Err(format!("{}: nonzero validity padding bits", field.name));
            }
        }
        let stride = field.scalar.size() * field.components as usize;
        for (i, value) in self.values.chunks_exact(stride).enumerate() {
            if !self.is_valid(i) {
                if value.iter().any(|&b| b != 0) {
                    return Err(format!(
                        "{}: invalid samples must have canonical zero bytes",
                        field.name
                    ));
                }
                continue;
            }
            if field.scalar.is_float() {
                let mut floats = [0.0; 4];
                for (component, raw) in value.chunks_exact(field.scalar.size()).enumerate() {
                    floats[component] = if field.scalar == ScalarType::F32 {
                        f32::from_le_bytes(raw.try_into().unwrap()) as f64
                    } else {
                        f64::from_le_bytes(raw.try_into().unwrap())
                    };
                    if !floats[component].is_finite() {
                        return Err(format!("{}: non-finite valid sample", field.name));
                    }
                }
                match field.semantic {
                    AttributeSemantic::Radius if floats[0] <= 0.0 => {
                        return Err("radius must be positive metres".into());
                    }
                    AttributeSemantic::Confidence if !(0.0..=1.0).contains(&floats[0]) => {
                        return Err("confidence must be in 0..1".into());
                    }
                    AttributeSemantic::Normal
                        if (floats[..3].iter().map(|v| v * v).sum::<f64>() - 1.0).abs() > 0.002 =>
                    {
                        return Err("normal must have unit length".into());
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudPage {
    pub count: u32,
    pub columns: Vec<ColumnData>,
}

impl CloudPage {
    pub fn validate(&self, schema: &CloudSchema) -> Result<(), String> {
        schema.validate()?;
        if self.count > MAX_PAGE_POINTS || self.columns.len() != schema.attributes.len() {
            return Err("cloud page point/column count is invalid".into());
        }
        let mut bytes = 0usize;
        for column in &self.columns {
            bytes = bytes
                .checked_add(column.values.len())
                .and_then(|n| n.checked_add(column.validity.len()))
                .ok_or("cloud page byte count overflow")?;
        }
        if bytes > MAX_PAGE_BYTES {
            return Err("cloud page exceeds 32 MiB".into());
        }
        for (column, field) in self.columns.iter().zip(&schema.attributes) {
            column.validate(field, self.count)?;
            if field.semantic == AttributeSemantic::SourceId {
                let mut ids = HashSet::with_capacity(self.count as usize);
                for raw in column.values.chunks_exact(8) {
                    if !ids.insert(u64::from_le_bytes(raw.try_into().unwrap())) {
                        return Err("duplicate source ID within cloud page".into());
                    }
                }
            }
        }
        Ok(())
    }
}
