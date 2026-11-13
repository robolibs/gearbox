use serde::{Deserialize, Serialize};

use super::frames::bounded_text;
use super::{
    BeginMetadata, CommitMode, DataRequest, DataShape, MAX_ENCODED_PAGE_BYTES, PageDescriptor,
    Presentation, ScanDescriptor, data_op,
};

pub const MAX_LIST_ITEMS: u32 = 128;
pub const MAX_CURSOR_BYTES: usize = 128;
pub const MAX_PROVIDER_NAME_BYTES: usize = 64;

/// Binds the exact base generation to a host-registered page provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderAttachment {
    pub provider: String,
}

impl ProviderAttachment {
    pub fn validate(&self) -> Result<(), String> {
        bounded_text(&self.provider, MAX_PROVIDER_NAME_BYTES, false)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamespaceFilter {
    pub namespace: String,
}

impl NamespaceFilter {
    pub fn validate(&self) -> Result<(), String> {
        bounded_text(&self.namespace, 255, true)?;
        if self.namespace.trim_matches('/') != self.namespace {
            return Err("namespace filter must not have outer slashes".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataListQuery {
    pub namespace: String,
    pub limit: u32,
    pub cursor: Option<String>,
}

impl Default for DataListQuery {
    fn default() -> Self {
        Self {
            namespace: String::new(),
            limit: 64,
            cursor: None,
        }
    }
}

impl DataListQuery {
    pub fn validate(&self) -> Result<(), String> {
        NamespaceFilter {
            namespace: self.namespace.clone(),
        }
        .validate()?;
        if self.limit == 0 || self.limit > MAX_LIST_ITEMS {
            return Err("list item limit must be 1..128".into());
        }
        if let Some(cursor) = &self.cursor {
            bounded_text(cursor, MAX_CURSOR_BYTES, false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewIdentity {
    pub id: u64,
    pub epoch: u64,
}

impl ViewIdentity {
    pub fn validate(&self) -> Result<(), String> {
        if self.id == 0 || self.epoch == 0 {
            return Err("view identity and incarnation must be nonzero".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataStatusQuery {
    pub generation: Option<u64>,
    pub view: Option<ViewIdentity>,
}

impl DataStatusQuery {
    pub fn validate(&self) -> Result<(), String> {
        if self.generation == Some(0) {
            return Err("generation zero is not retained data".into());
        }
        if let Some(view) = self.view {
            view.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DataOperation {
    Capabilities,
    OpenWriter,
    CloseWriter,
    Begin(BeginMetadata),
    DeclareScan(ScanDescriptor),
    DeclarePage(PageDescriptor),
    Chunk,
    RemovePage,
    RemoveScan,
    Commit,
    Cancel,
    PatchPresentation(Presentation),
    Delete,
    Clear(NamespaceFilter),
    List(DataListQuery),
    Status(DataStatusQuery),
    AttachProvider(ProviderAttachment),
    FieldQuery(super::FieldQuery),
}

impl DataRequest {
    /// Decodes intrinsic operation metadata; ownership and generation checks require a store.
    pub fn operation(&self) -> Result<DataOperation, String> {
        use data_op::*;
        self.validate()?;
        if (self.incarnation == 0) != (self.base_generation == 0) {
            return Err(
                "dataset incarnation and base generation must be specified together".into(),
            );
        }
        Ok(match self.operation {
            CAPABILITIES => DataOperation::Capabilities,
            OPEN_WRITER => DataOperation::OpenWriter,
            CLOSE_WRITER => DataOperation::CloseWriter,
            BEGIN => {
                let metadata: BeginMetadata = self.metadata_as()?;
                metadata.validate()?;
                if metadata.descriptor.key != self.data_key()? {
                    return Err("BEGIN key does not match its descriptor".into());
                }
                if metadata.mode == CommitMode::Delta && self.base_generation == 0 {
                    return Err("delta requires an existing incarnation and base generation".into());
                }
                if self.total_bytes < u64::from(metadata.pages) * 16
                    || self.total_bytes > u64::from(metadata.pages) * MAX_ENCODED_PAGE_BYTES as u64
                {
                    return Err("transaction bytes do not match declared page count bounds".into());
                }
                if !matches!(metadata.descriptor.shape, DataShape::Cloud { .. })
                    && (metadata.scans != 0 || metadata.removed_scans != 0)
                {
                    return Err("only cloud transactions declare or remove scans".into());
                }
                DataOperation::Begin(metadata)
            }
            DECLARE_SCAN => {
                let metadata = ScanDescriptor::from_json(&self.metadata)?;
                if metadata.scan_key != self.scan_key {
                    return Err("scan declaration key mismatch".into());
                }
                DataOperation::DeclareScan(metadata)
            }
            DECLARE_PAGE => {
                let metadata: PageDescriptor = self.metadata_as()?;
                metadata.validate()?;
                if metadata.address.scan_key != self.scan_key
                    || metadata.address.page_id != self.page_id
                {
                    return Err("page declaration address mismatch".into());
                }
                DataOperation::DeclarePage(metadata)
            }
            CHUNK => DataOperation::Chunk,
            REMOVE_PAGE => DataOperation::RemovePage,
            REMOVE_SCAN => DataOperation::RemoveScan,
            COMMIT => DataOperation::Commit,
            CANCEL => DataOperation::Cancel,
            PATCH_PRESENTATION => {
                let metadata: Presentation = self.metadata_as()?;
                metadata.validate_intrinsic()?;
                DataOperation::PatchPresentation(metadata)
            }
            DELETE => DataOperation::Delete,
            CLEAR => {
                let metadata: NamespaceFilter = self.metadata_as()?;
                metadata.validate()?;
                DataOperation::Clear(metadata)
            }
            LIST => {
                let metadata: DataListQuery = self.metadata_as()?;
                metadata.validate()?;
                DataOperation::List(metadata)
            }
            STATUS => {
                let metadata: DataStatusQuery = self.metadata_as()?;
                metadata.validate()?;
                DataOperation::Status(metadata)
            }
            ATTACH_PROVIDER => {
                let metadata: ProviderAttachment = self.metadata_as()?;
                metadata.validate()?;
                if self.base_generation == 0 {
                    return Err("provider attachment requires an exact base generation".into());
                }
                DataOperation::AttachProvider(metadata)
            }
            FIELD_QUERY => {
                let metadata: super::FieldQuery = self.metadata_as()?;
                metadata.validate()?;
                DataOperation::FieldQuery(metadata)
            }
            _ => return Err("unsupported data operation".into()),
        })
    }
}
