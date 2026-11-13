use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::frames::bounded_text;
use super::{CloudSchema, DataFrame, OctreeConfig, Placement, Presentation, SCHEMA_REVISION};

pub const MAX_DESCRIPTOR_BYTES: usize = 12 * 1024;
pub const MAX_SCAN_BYTES: usize = 4 * 1024;
pub const MAX_SCANS: u32 = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataKey {
    pub namespace: String,
    pub id: String,
}

impl DataKey {
    pub fn new(namespace: &str, id: &str) -> Result<Self, String> {
        let key = Self {
            namespace: namespace.trim_matches('/').into(),
            id: id.into(),
        };
        key.validate()?;
        Ok(key)
    }

    pub fn validate(&self) -> Result<(), String> {
        bounded_text(&self.namespace, 255, true)?;
        bounded_text(&self.id, 127, false)?;
        if self.namespace.trim_matches('/') != self.namespace {
            return Err("namespace must not have outer slashes".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgeClock {
    Simulation,
    Source,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum StreamMode {
    Latest,
    RetainedMap,
    Accumulate {
        max_scans: u32,
        max_age_s: f64,
        max_bytes: u64,
        age_clock: AgeClock,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Organization {
    pub width: u32,
    pub height: u32,
}

impl Organization {
    pub fn validate(&self, source_count: u64) -> Result<(), String> {
        if (self.width == 0) != (self.height == 0)
            || self.width as u64 * self.height as u64 != source_count
        {
            return Err("organized dimensions must exactly cover source samples including invalid observations".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataShape {
    Cloud {
        schema: CloudSchema,
    },
    Octree {
        config: OctreeConfig,
        frame: DataFrame,
    },
    /// Oriented discs: cloud columns with one position, normal and metric radius each.
    Surfels {
        schema: CloudSchema,
        frame: DataFrame,
    },
    /// Indexed triangle blocks, one per page.
    MeshBlocks {
        frame: DataFrame,
    },
    /// Sparse field blocks, one per page, keyed by block.
    Volume {
        field: super::VolumeField,
        frame: DataFrame,
    },
    /// Layered elevation tiles, one page per tile and layer.
    Elevation {
        grid: super::ElevationGrid,
        frame: DataFrame,
    },
}

impl DataShape {
    pub fn kind(&self) -> super::DatasetKind {
        use super::DatasetKind;
        match self {
            Self::Cloud { .. } => DatasetKind::Cloud,
            Self::Octree { .. } => DatasetKind::Octree,
            Self::Surfels { .. } => DatasetKind::Surfels,
            Self::MeshBlocks { .. } => DatasetKind::MeshBlocks,
            Self::Volume { .. } => DatasetKind::Volume,
            Self::Elevation { .. } => DatasetKind::Elevation,
        }
    }

    /// How this shape's pages are encoded.
    pub fn encoding(&self) -> super::PageEncoding {
        use super::PageEncoding;
        match self {
            Self::Cloud { .. } | Self::Surfels { .. } => PageEncoding::CloudColumnsV1,
            Self::Octree { .. } => PageEncoding::OccupancyLeavesV1,
            Self::MeshBlocks { .. } => PageEncoding::MeshBlockV1,
            Self::Volume { .. } => PageEncoding::VolumeBlockV1,
            Self::Elevation { .. } => PageEncoding::ElevationTileV1,
        }
    }

    /// The column schema of shapes stored as cloud columns.
    pub fn columns(&self) -> Option<&CloudSchema> {
        match self {
            Self::Cloud { schema } | Self::Surfels { schema, .. } => Some(schema),
            _ => None,
        }
    }

    /// The dataset frame of shapes that own no scans.
    pub fn frame(&self) -> Option<&DataFrame> {
        match self {
            Self::Cloud { .. } => None,
            Self::Octree { frame, .. } | Self::Surfels { frame, .. } | Self::MeshBlocks { frame } | Self::Volume { frame, .. } | Self::Elevation { frame, .. } => Some(frame),
        }
    }
}

/// The vertex attributes a mesh block can carry, for presentation validation.
pub fn mesh_block_schema() -> CloudSchema {
    use super::{Attribute, AttributeSemantic, Normalization, ScalarType};
    let field = |name: &str, scalar, components, semantic, units: &str, normalization| Attribute {
        name: name.into(),
        scalar,
        components,
        semantic,
        units: units.into(),
        normalization,
        nullable: true,
    };
    let mut position = field("position", ScalarType::F32, 3, AttributeSemantic::Position, "m", Normalization::None);
    position.nullable = false;
    let mut id = field("vertex", ScalarType::U64, 1, AttributeSemantic::SourceId, "", Normalization::None);
    id.nullable = false;
    CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            position,
            id,
            field("normal", ScalarType::F32, 3, AttributeSemantic::Normal, "", Normalization::None),
            field("color", ScalarType::U8, 4, AttributeSemantic::Color, "", Normalization::Unorm),
            field("confidence", ScalarType::F32, 1, AttributeSemantic::Confidence, "", Normalization::None),
        ],
    }
}

/// A surfel schema has exactly one float normal and one metric radius.
pub fn validate_surfel_schema(schema: &CloudSchema) -> Result<(), String> {
    use super::AttributeSemantic;
    let count = |semantic| schema.attributes.iter().filter(|a| a.semantic == semantic).count();
    if count(AttributeSemantic::Normal) != 1 || count(AttributeSemantic::Radius) != 1 {
        return Err("surfels need exactly one normal and one radius attribute".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanDescriptor {
    pub revision: u32,
    pub scan_key: u64,
    pub source: Option<String>,
    pub frame: DataFrame,
    pub source_count: u64,
    pub organization: Option<Organization>,
}

impl ScanDescriptor {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision != SCHEMA_REVISION {
            return Err("unsupported scan descriptor revision".into());
        }
        if let Some(source) = &self.source {
            bounded_text(source, 255, false)?;
        }
        self.frame.validate()?;
        if let Some(organization) = self.organization {
            organization.validate(self.source_count)?;
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_SCAN_BYTES {
            return Err("scan descriptor exceeds 4 KiB".into());
        }
        Ok(bytes)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_SCAN_BYTES {
            return Err("scan descriptor exceeds 4 KiB".into());
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid scan descriptor: {e}"))?;
        value.validate()?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataDescriptor {
    pub revision: u32,
    pub key: DataKey,
    pub shape: DataShape,
    pub stream: StreamMode,
    pub lifetime_s: Option<f64>,
    pub presentation: Presentation,
}

impl DataDescriptor {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision != SCHEMA_REVISION {
            return Err("unsupported dataset descriptor revision".into());
        }
        self.key.validate()?;
        if self.lifetime_s.is_some_and(|v| !v.is_finite() || v <= 0.0) {
            return Err("lifetime must be positive finite simulation seconds or null".into());
        }
        let mesh = mesh_block_schema();
        let synthetic = match &self.shape {
            DataShape::Volume { field, .. } => super::volume::volume_schema(field),
            DataShape::Elevation { grid, .. } => super::elevation::elevation_schema(grid),
            _ => mesh.clone(),
        };
        let schema = match &self.shape {
            DataShape::Cloud { schema } => {
                schema.to_json()?;
                Some(schema)
            }
            DataShape::Octree { config, frame } => {
                config.validate()?;
                frame.validate()?;
                self.presentation
                    .color
                    .validate_capture(frame.capture.as_ref())?;
                None
            }
            DataShape::Surfels { schema, frame } => {
                schema.to_json()?;
                validate_surfel_schema(schema)?;
                frame.validate()?;
                self.presentation.color.validate_capture(frame.capture.as_ref())?;
                Some(schema)
            }
            DataShape::MeshBlocks { frame } => {
                frame.validate()?;
                self.presentation.color.validate_capture(frame.capture.as_ref())?;
                Some(&mesh)
            }
            DataShape::Volume { field, frame } => {
                field.validate()?;
                frame.validate()?;
                self.presentation.color.validate_capture(frame.capture.as_ref())?;
                if let Some(view) = &self.presentation.volume {
                    view.validate(field)?;
                }
                Some(&synthetic)
            }
            DataShape::Elevation { grid, frame } => {
                grid.validate()?;
                frame.validate()?;
                self.presentation.color.validate_capture(frame.capture.as_ref())?;
                Some(&synthetic)
            }
        };
        if self.presentation.volume.is_some() && !matches!(self.shape, DataShape::Volume { .. }) {
            return Err("volume views need a volume dataset".into());
        }
        if let StreamMode::Accumulate {
            max_scans,
            max_age_s,
            max_bytes,
            ..
        } = self.stream
        {
            if !matches!(self.shape, DataShape::Cloud { .. })
                || max_scans == 0
                || max_scans > MAX_SCANS
                || !max_age_s.is_finite()
                || max_age_s <= 0.0
                || max_bytes == 0
            {
                return Err("accumulation requires capture-time cloud snapshots and finite positive scan/age/byte bounds".into());
            }
        }
        let octree = match &self.shape {
            DataShape::Octree { config, .. } => Some(config),
            _ => None,
        };
        let has_capture = match &self.shape {
            DataShape::Cloud { .. } => true,
            shape => shape.frame().is_some_and(|frame| frame.capture.is_some()),
        };
        self.presentation.validate(schema, octree, has_capture)
    }

    pub fn validate_scan(&self, scan: &ScanDescriptor) -> Result<(), String> {
        self.validate()?;
        self.validate_scan_inner(scan)
    }

    fn validate_scan_inner(&self, scan: &ScanDescriptor) -> Result<(), String> {
        scan.validate()?;
        let DataShape::Cloud { schema } = &self.shape else {
            return Err("only cloud datasets own scans".into());
        };
        match self.stream {
            StreamMode::RetainedMap if scan.scan_key != 0 => {
                return Err("static retained cloud uses scan key zero".into());
            }
            StreamMode::Latest | StreamMode::Accumulate { .. }
                if scan.scan_key == 0 || scan.frame.capture.is_none() =>
            {
                return Err("acquired frames require a nonzero scan key and capture stamp".into());
            }
            _ => {}
        }
        if let StreamMode::Accumulate { age_clock, .. } = self.stream {
            if scan.frame.placement != Placement::Snapshot {
                return Err("accumulation requires a capture-time snapshot for each scan".into());
            }
            if age_clock == AgeClock::Source
                && scan
                    .frame
                    .capture
                    .as_ref()
                    .is_none_or(|s| s.source_time_ns.is_none())
            {
                return Err("source-age accumulation requires a named source clock".into());
            }
        }
        self.presentation
            .color
            .validate(Some(schema), None, scan.frame.capture.is_some())?;
        self.presentation
            .color
            .validate_capture(scan.frame.capture.as_ref())
    }

    pub fn validate_scans(&self, scans: &[ScanDescriptor]) -> Result<(), String> {
        self.validate()?;
        let limit = match self.stream {
            StreamMode::Accumulate { max_scans, .. } => max_scans as usize,
            _ => 1,
        };
        if scans.len() > limit {
            return Err("scan table exceeds stream scan limit".into());
        }
        let mut keys = HashSet::new();
        let mut clock = None;
        let mut epoch = None;
        for scan in scans {
            self.validate_scan_inner(scan)?;
            if !keys.insert(scan.scan_key) {
                return Err("duplicate scan key".into());
            }
            if let StreamMode::Accumulate { age_clock, .. } = self.stream {
                let stamp = scan.frame.capture.as_ref().ok_or("missing capture stamp")?;
                if epoch.is_some_and(|previous| previous != stamp.scene_epoch) {
                    return Err("accumulated scans must share a scene epoch".into());
                }
                epoch = Some(stamp.scene_epoch);
                if age_clock == AgeClock::Source {
                    let current = stamp
                        .source_clock
                        .as_deref()
                        .ok_or("missing source clock")?;
                    if clock.is_some_and(|previous| previous != current) {
                        return Err("source-age scans must share one clock domain".into());
                    }
                    clock = Some(current);
                }
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return Err("dataset descriptor exceeds 12 KiB".into());
        }
        Ok(bytes)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return Err("dataset descriptor exceeds 12 KiB".into());
        }
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|e| format!("invalid dataset descriptor: {e}"))?;
        value.validate()?;
        Ok(value)
    }
}
