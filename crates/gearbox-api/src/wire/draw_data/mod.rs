//! Typed spatial drawing data, independent of the legacy drawing wire layout.

mod attributes;
mod capabilities;
mod capture;
mod descriptor;
mod envelope;
mod frames;
mod identity;
mod octree;
pub mod lidar;
pub mod mesh;
mod operations;
pub(crate) mod pages;
mod presentation;
mod status;
pub mod volume;
pub mod elevation;

pub use attributes::{
    Attribute, AttributeSemantic, CloudPage, CloudSchema, ColumnData, ColumnView, Normalization,
    ScalarType,
};
pub use capabilities::{DataCapabilities, DataFeatures, DataLimits, DatasetKind};
pub use capture::{
    CaptureConvention, MAX_NATIVE_CAPTURE_BYTES, MAX_NATIVE_CAPTURE_PROPS_BYTES,
    NATIVE_CAPTURE_PROP, NativeLidarCapture, lidar_chunk_capacity,
};
pub use descriptor::{
    AgeClock, DataDescriptor, DataKey, DataShape, MAX_DESCRIPTOR_BYTES, MAX_SCAN_BYTES, MAX_SCANS,
    Organization, ScanDescriptor, StreamMode, mesh_block_schema, validate_surfel_schema,
};
pub use envelope::{
    BeginMetadata, CommitMode, DATA_CHUNK_BYTES, DATA_KEY_BYTES, DATA_METADATA_BYTES, DataReply,
    DataRequest, MAX_DECLARED_PAGES, data_code, data_op, data_stage,
};
pub use frames::{CaptureStamp, CaptureTransform, DataFrame, DatumSnapshot, Placement, RigidPose};
pub use identity::DatasetRevision;
pub use octree::{
    GridBounds, MAX_OCTREE_DEPTH, OccupancyLeaf, OccupancyState, OccupancyValue, OctreeConfig,
    VoxelKey, validate_leaf_set,
};
pub use operations::{
    DataListQuery, DataOperation, DataStatusQuery, MAX_CURSOR_BYTES, MAX_LIST_ITEMS,
    MAX_PROVIDER_NAME_BYTES, NamespaceFilter, ProviderAttachment, ViewIdentity,
};
pub use pages::{
    CloudPageView, MAX_ENCODED_PAGE_BYTES, MAX_PAGE_LEAVES, OccupancyPageView, PageAddress,
    PageDescriptor, PageEncoding, decode_occupancy_page, encode_occupancy_page, page_crc32,
};
pub use presentation::{
    ClipBounds, ColorMode, ColorSpace, NormalArrows, OctreeDisplay, PaletteEntry, PointSize,
    Presentation, VoxelStyle,
};

pub const SCHEMA_REVISION: u32 = 1;
pub use status::{
    DataCounters, DataFailure, DataPhase, DataResourceUsage, DatasetStatus, DrawSummary,
    DrawSummaryPage, FrontierDisposition, GenerationVisibility, PageResidency, SummaryOwner,
    TransferProgress, VisibilityTarget,
};
pub const MAX_ATTRIBUTES: usize = 32;
pub const MAX_ATTRIBUTE_NAME_BYTES: usize = 63;
pub const MAX_SCHEMA_BYTES: usize = 8 * 1024;
pub const MAX_PAGE_POINTS: u32 = 65_536;
pub const MAX_PAGE_BYTES: usize = 32 * 1024 * 1024;
pub use volume::{
    Channel, FieldAnswer, FieldKind, FieldQuery, FieldSample, Interpolation, Support, VolumeField,
    VolumeSlice, VolumeView, volume_block_key, volume_page_id,
};
pub use elevation::{
    ELEVATION, ElevationDelta, ElevationGrid, ElevationLayer, ElevationTile, ElevationWindow,
    RollingMap, RollingState, elevation_page_id, elevation_tile_of,
};
