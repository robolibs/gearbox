//! Shared drawing identity and immutable dataset publication.

mod backend;
mod borrowed_key;
mod generation;
mod gpu;
mod identity;
mod memory;
mod metadata;
mod page;
mod paged_generation;
mod provider;
mod render_reports;
mod storage;
mod stream;
pub mod survey;
mod transaction;
mod visibility;

#[cfg(test)]
mod concurrency_tests;

pub use backend::{DrawingBackend, LegacyDrawGuard, MAX_PROVIDER_FACTORIES};
pub use generation::{Generation, GenerationCounts};
pub use gpu::{GpuBudget, GpuCharge, GpuClass, GpuUsage};
pub use identity::{DatasetEntry, DatasetLifetime, DrawingStore, IdentityError, WritePermit};
pub use memory::{Charge, CpuBudget, MemoryClass, MemoryUsage, Reservation, StorageError};
pub use metadata::{DatasetMetadata, Metadata, ScanMetadata};
pub use page::{ChunkOutcome, PageFootprint, PageMetadata, PageUpload, StagedPage, StoredPage};
pub use paged_generation::PagedGeneration;
pub use provider::{
    PageProvider, PageRequest, PageSink, PageState, ProviderAvailability, ProviderError,
    ProviderFactory, ProviderStatistics, RegisteredProvider, ReloadablePage,
};
pub use render_reports::RenderDiagnostic;
pub use storage::{DatasetStorage, PageSource};
pub use stream::{StreamCounters, StreamUsage, StreamingStore};
pub use transaction::{SharedDrawingStore, TransactionError};
pub use visibility::RenderCompletion;
