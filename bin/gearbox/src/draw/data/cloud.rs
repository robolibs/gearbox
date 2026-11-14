use std::sync::Arc;

use gearbox_api::draw_data::{Charge, CpuBudget, MemoryClass, StorageError, StoredPage};
use gearbox_api::wire::draw_data::{
    Attribute, AttributeSemantic, CloudSchema, ColumnView, Normalization, PageDescriptor,
    ScalarType,
};

pub(super) const MAX_LOCAL_ERROR_M: f64 = 0.001;

#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct Position {
    pub xyz: [f32; 3],
}

#[derive(Debug)]
pub(super) struct Records<T> {
    pub values: Vec<T>,
    _charge: Charge,
}

impl<T: Copy + Default> Records<T> {
    pub fn new(budget: &CpuBudget, count: usize) -> Result<Self, StorageError> {
        let bytes = count
            .checked_mul(size_of::<T>())
            .ok_or(StorageError::Allocation)?;
        let charge = budget
            .reserve(bytes as u64)?
            .charge(bytes as u64, MemoryClass::Scratch)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| StorageError::Allocation)?;
        if values.capacity() != count {
            return Err(StorageError::Allocation);
        }
        values.resize(count, T::default());
        Ok(Self {
            values,
            _charge: charge,
        })
    }
}

#[derive(Debug)]
pub(super) struct SpatialMetadata {
    pub rebase_m: [f64; 3],
    pub bounds_m: Option<[[f64; 3]; 2]>,
    pub max_error_m: f64,
    pub hierarchy: super::hierarchy::Hierarchy,
    pub tree: Records<super::tree::TreeNode>,
    pub coarse: super::coarse::Representatives,
    _charge: Charge,
}

/// One page's spatial metadata plus, while resident, its source payload.
#[derive(Debug)]
pub(super) struct Geometry {
    source: Option<Arc<StoredPage>>,
    descriptor: PageDescriptor,
    count: usize,
    pub positions: Records<Position>,
    /// Source row of each render sample; empty when every source row is valid.
    rows: Records<u32>,
    pub metadata: Arc<SpatialMetadata>,
    position_column: usize,
    source_id_column: usize,
    position_type: ScalarType,
    _charge: Charge,
}

struct Placement {
    positions: Records<Position>,
    rows: Records<u32>,
    rebase_m: [f64; 3],
    bounds_m: Option<[[f64; 3]; 2]>,
    max_error_m: f64,
    position_column: usize,
    source_id_column: usize,
    position_type: ScalarType,
}

impl std::ops::Deref for Geometry {
    type Target = SpatialMetadata;

    fn deref(&self) -> &Self::Target {
        &self.metadata
    }
}

impl Geometry {
    #[cfg(test)]
    pub fn prepare(
        budget: &CpuBudget,
        schema: &CloudSchema,
        source: Arc<StoredPage>,
    ) -> Result<Arc<Self>, StorageError> {
        Self::prepare_cancellable(budget, schema, source, &|| Ok(()))
    }

    pub fn prepare_cancellable(
        budget: &CpuBudget,
        schema: &CloudSchema,
        source: Arc<StoredPage>,
        check: &impl Fn() -> Result<(), StorageError>,
    ) -> Result<Arc<Self>, StorageError> {
        let placement = place(budget, schema, &source, check)?;
        let metadata_charge = super::worker::charge(budget, size_of::<SpatialMetadata>() + 16)?;
        let metadata = Arc::new(SpatialMetadata {
            rebase_m: placement.rebase_m,
            bounds_m: placement.bounds_m,
            max_error_m: placement.max_error_m,
            hierarchy: super::hierarchy::Hierarchy::empty(budget)?,
            tree: Records::new(budget, 0)?,
            coarse: super::coarse::Representatives::empty(budget)?,
            _charge: metadata_charge,
        });
        let mut geometry = Self::assemble(budget, source, placement, metadata)?;
        let hierarchy = super::hierarchy::Hierarchy::build(budget, &geometry, check)?;
        let tree = super::tree::build(budget, &hierarchy, geometry.rebase_m, geometry.max_error_m)?;
        let metadata = Arc::get_mut(&mut geometry.metadata).unwrap();
        metadata.hierarchy = hierarchy;
        metadata.tree = tree;
        let coarse = super::coarse::Representatives::build(budget, &geometry, check)?;
        Arc::get_mut(&mut geometry.metadata).unwrap().coarse = coarse;
        check()?;
        Ok(Arc::new(geometry))
    }

    /// Restores the payload of a hollow page against its existing spatial metadata.
    pub fn reload(
        budget: &CpuBudget,
        schema: &CloudSchema,
        source: Arc<StoredPage>,
        previous: &Geometry,
        check: &impl Fn() -> Result<(), StorageError>,
    ) -> Result<Arc<Self>, StorageError> {
        if source.descriptor() != previous.descriptor() {
            return Err(invalid(
                "reloaded page descriptor differs from its metadata",
            ));
        }
        let placement = place(budget, schema, &source, check)?;
        let metadata = &previous.metadata;
        if placement.positions.values.len() != previous.count
            || placement.rebase_m != metadata.rebase_m
            || placement.bounds_m != metadata.bounds_m
        {
            return Err(invalid("reloaded page placement differs from its metadata"));
        }
        let geometry = Self::assemble(budget, source, placement, metadata.clone())?;
        check()?;
        Ok(Arc::new(geometry))
    }

    /// Same page and metadata identity without the source payload.
    pub fn hollow(&self, budget: &CpuBudget) -> Result<Arc<Self>, StorageError> {
        let bytes = (size_of::<Self>() + 2 * size_of::<usize>()) as u64;
        Ok(Arc::new(Self {
            source: None,
            descriptor: self.descriptor.clone(),
            count: self.count,
            positions: Records::new(budget, 0)?,
            rows: Records::new(budget, 0)?,
            metadata: self.metadata.clone(),
            position_column: self.position_column,
            source_id_column: self.source_id_column,
            position_type: self.position_type,
            _charge: budget
                .reserve(bytes)?
                .charge(bytes, MemoryClass::Metadata)?,
        }))
    }

    fn assemble(
        budget: &CpuBudget,
        source: Arc<StoredPage>,
        placement: Placement,
        metadata: Arc<SpatialMetadata>,
    ) -> Result<Self, StorageError> {
        let bytes = (size_of::<Self>() + 2 * size_of::<usize>()) as u64;
        Ok(Self {
            descriptor: source.descriptor().clone(),
            count: placement.positions.values.len(),
            source: Some(source),
            positions: placement.positions,
            rows: placement.rows,
            metadata,
            position_column: placement.position_column,
            source_id_column: placement.source_id_column,
            position_type: placement.position_type,
            _charge: budget
                .reserve(bytes)?
                .charge(bytes, MemoryClass::Metadata)?,
        })
    }

    pub fn source(&self) -> Option<&Arc<StoredPage>> {
        self.source.as_ref()
    }

    pub fn resident(&self) -> bool {
        self.source.is_some()
    }

    pub fn descriptor(&self) -> &PageDescriptor {
        &self.descriptor
    }

    /// Valid samples in the page, resident or not.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Local position of an ordinal, from the payload or its coarse representative.
    pub fn local_point(&self, ordinal: u32) -> Option<[f32; 3]> {
        if self.resident() {
            return self.positions.values.get(ordinal as usize).map(|p| p.xyz);
        }
        let index = self.metadata.coarse.index(ordinal)?;
        self.metadata
            .coarse
            .positions
            .values
            .get(index as usize)
            .map(|p| p.xyz)
    }
}

fn place(
    budget: &CpuBudget,
    schema: &CloudSchema,
    source: &StoredPage,
    check: &impl Fn() -> Result<(), StorageError>,
) -> Result<Placement, StorageError> {
    {
        check()?;
        schema.validate().map_err(invalid)?;
        let position_column = schema
            .attributes
            .iter()
            .position(|a| a.semantic == AttributeSemantic::Position)
            .ok_or_else(|| invalid("position column missing"))?;
        let source_id_column = schema
            .attributes
            .iter()
            .position(|a| a.semantic == AttributeSemantic::SourceId)
            .ok_or_else(|| invalid("source ID column missing"))?;
        let field = &schema.attributes[position_column];
        let column = source
            .column(position_column)
            .ok_or_else(|| invalid("not a cloud page"))?;
        column
            .validate(field, source.descriptor().items)
            .map_err(invalid)?;
        let mut count = 0;
        for index in 0..source.descriptor().items as usize {
            if index % 1024 == 0 {
                check()?;
            }
            count += usize::from(valid(column, index));
        }
        let mut positions = Records::<Position>::new(budget, count)?;
        let compacted = count != source.descriptor().items as usize;
        let mut rows = Records::<u32>::new(budget, if compacted { count } else { 0 })?;
        let mut rebase_m = [0.0; 3];
        let mut bounds_m: Option<[[f64; 3]; 2]> = None;
        let mut max_error_m: f64 = 0.0;
        let mut output = 0;
        for index in 0..source.descriptor().items as usize {
            if index % 1024 == 0 {
                check()?;
            }
            if !valid(column, index) {
                continue;
            }
            let point = position(column, field.scalar, index)?;
            if output == 0 {
                rebase_m = point;
            }
            let mut xyz = [0.0f32; 3];
            let mut point_error = 0.0f64;
            for axis in 0..3 {
                let local = point[axis] - rebase_m[axis];
                xyz[axis] = local as f32;
                let error = (f64::from(xyz[axis]) - local).abs();
                if !local.is_finite() || !xyz[axis].is_finite() || error > MAX_LOCAL_ERROR_M {
                    return Err(invalid(
                        "page exceeds finite one-millimetre local GPU precision",
                    ));
                }
                point_error = point_error.hypot(error);
            }
            if point_error > MAX_LOCAL_ERROR_M {
                return Err(invalid(
                    "page exceeds one-millimetre local GPU position error",
                ));
            }
            max_error_m = max_error_m.max(point_error);
            let bounds = bounds_m.get_or_insert([point, point]);
            for axis in 0..3 {
                bounds[0][axis] = bounds[0][axis].min(point[axis]);
                bounds[1][axis] = bounds[1][axis].max(point[axis]);
            }
            positions.values[output] = Position { xyz };
            if compacted {
                rows.values[output] = index as u32;
            }
            output += 1;
        }
        Ok(Placement {
            positions,
            rows,
            rebase_m,
            bounds_m,
            max_error_m,
            position_column,
            source_id_column,
            position_type: field.scalar,
        })
    }
}

impl Geometry {
    /// Source row of a render sample.
    pub fn source_row(&self, index: usize) -> Option<usize> {
        if index >= self.positions.values.len() {
            return None;
        }
        Some(self.rows.values.get(index).map_or(index, |row| *row as usize))
    }

    pub fn source_position(&self, index: usize) -> Result<[f64; 3], StorageError> {
        let source = self
            .source_row(index)
            .ok_or_else(|| invalid("render sample is out of bounds"))?;
        position(
            self.source
                .as_ref()
                .ok_or_else(|| invalid("page payload is not resident"))?
                .column(self.position_column)
                .ok_or_else(|| invalid("position column missing"))?,
            self.position_type,
            source,
        )
    }

    pub fn source_id(&self, index: usize) -> Option<u64> {
        let source = self.source_row(index)?;
        let raw = self
            .source
            .as_ref()?
            .column(self.source_id_column)?
            .values
            .get(source * 8..source * 8 + 8)?;
        Some(u64::from_le_bytes(raw.try_into().ok()?))
    }
}

pub(super) fn invalid(message: impl Into<String>) -> StorageError {
    StorageError::Invalid(message.into())
}

pub(super) fn valid(column: ColumnView<'_>, index: usize) -> bool {
    column.validity.is_empty()
        || column
            .validity
            .get(index / 8)
            .is_some_and(|b| b & (1 << (index % 8)) != 0)
}

pub(super) fn position(
    column: ColumnView<'_>,
    scalar: ScalarType,
    index: usize,
) -> Result<[f64; 3], StorageError> {
    let mut point = [0.0; 3];
    for (axis, value) in point.iter_mut().enumerate() {
        *value = scalar_value(column.values, scalar, index * 3 + axis)?;
    }
    Ok(point)
}

pub(super) fn exact_u64(column: ColumnView<'_>, index: usize) -> Result<Option<u64>, StorageError> {
    if !valid(column, index) {
        return Ok(None);
    }
    let start = index.checked_mul(8).ok_or(StorageError::Allocation)?;
    let end = start.checked_add(8).ok_or(StorageError::Allocation)?;
    let raw = column
        .values
        .get(start..end)
        .ok_or_else(|| invalid("short u64 column"))?;
    Ok(Some(u64::from_le_bytes(raw.try_into().unwrap())))
}

pub(super) fn scalar_value(
    bytes: &[u8],
    scalar: ScalarType,
    index: usize,
) -> Result<f64, StorageError> {
    let start = index
        .checked_mul(scalar.size())
        .ok_or(StorageError::Allocation)?;
    let end = start
        .checked_add(scalar.size())
        .ok_or(StorageError::Allocation)?;
    let raw = bytes
        .get(start..end)
        .ok_or_else(|| invalid("short column"))?;
    let value = match scalar {
        ScalarType::U8 => f64::from(raw[0]),
        ScalarType::U16 => f64::from(u16::from_le_bytes(raw.try_into().unwrap())),
        ScalarType::U32 => f64::from(u32::from_le_bytes(raw.try_into().unwrap())),
        ScalarType::U64 => u64::from_le_bytes(raw.try_into().unwrap()) as f64,
        ScalarType::I16 => f64::from(i16::from_le_bytes(raw.try_into().unwrap())),
        ScalarType::I32 => f64::from(i32::from_le_bytes(raw.try_into().unwrap())),
        ScalarType::F32 => f64::from(f32::from_le_bytes(raw.try_into().unwrap())),
        ScalarType::F64 => f64::from_le_bytes(raw.try_into().unwrap()),
    };
    if !value.is_finite() {
        return Err(invalid("nonfinite valid scalar"));
    }
    Ok(value)
}

pub(super) fn normalized(
    column: ColumnView<'_>,
    field: &Attribute,
    index: usize,
) -> Result<Option<f64>, StorageError> {
    if !valid(column, index) {
        return Ok(None);
    }
    let raw = scalar_value(column.values, field.scalar, index)?;
    let scale = match field.scalar {
        ScalarType::U8 => u8::MAX as f64,
        ScalarType::U16 => u16::MAX as f64,
        ScalarType::U32 => u32::MAX as f64,
        ScalarType::U64 => u64::MAX as f64,
        ScalarType::I16 => i16::MAX as f64,
        ScalarType::I32 => i32::MAX as f64,
        _ => 1.0,
    };
    Ok(Some(match field.normalization {
        Normalization::None => raw,
        Normalization::Unorm => raw / scale,
        Normalization::Snorm => (raw / scale).max(-1.0),
    }))
}
