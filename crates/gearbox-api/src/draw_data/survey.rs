//! Deterministic survey heightfield shared by the example writer and the host provider.

use std::sync::Arc;

use crate::wire::draw_data::{
    Attribute, AttributeSemantic, CloudPage, CloudSchema, ColumnData, DATA_CHUNK_BYTES,
    DataDescriptor, DataFrame, DataShape, Normalization, PageAddress, PageDescriptor, PageEncoding,
    SCHEMA_REVISION, ScalarType, ScanDescriptor, StreamMode, page_crc32,
};

use super::{
    CpuBudget, MemoryClass, PageProvider, PageSink, ProviderAvailability, ProviderError,
    ProviderFactory, RegisteredProvider, ScanMetadata, StorageError,
};

pub const SURVEY_PAGE: u32 = 65_536;
pub const SURVEY_MAX_POINTS: u32 = 5_000_000;
pub const SURVEY_SOURCE: &str = "synthetic/survey-heightfield";
pub const SURVEY_PROVIDER: &str = "fixture/survey-heightfield";

pub fn survey_schema() -> CloudSchema {
    CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: [
            ("position", ScalarType::F32, 3, AttributeSemantic::Position),
            ("source_id", ScalarType::U64, 1, AttributeSemantic::SourceId),
            ("rgba", ScalarType::U8, 4, AttributeSemantic::Color),
        ]
        .into_iter()
        .map(|(name, scalar, components, semantic)| Attribute {
            name: name.into(),
            scalar,
            components,
            semantic,
            units: if semantic == AttributeSemantic::Position {
                "m"
            } else {
                ""
            }
            .into(),
            normalization: if semantic == AttributeSemantic::Color {
                Normalization::Unorm
            } else {
                Normalization::None
            },
            nullable: false,
        })
        .collect(),
    }
}

/// Grid side length for a survey of `points` samples.
pub fn survey_side(points: u32) -> u32 {
    (f64::from(points).sqrt().ceil() as u32).max(1)
}

pub fn survey_sample(index: u32, side: u32) -> ([f32; 3], [u8; 4]) {
    let span = (side - 1).max(1) as f32;
    let x = (index % side) as f32 / span * 30.0 - 15.0;
    let y = (index / side) as f32 / span * 30.0 - 15.0;
    let hill =
        |cx: f32, cy: f32, scale: f32| (-((x - cx).powi(2) + (y - cy).powi(2)) / scale).exp();
    let z = 1.4
        + 6.0 * hill(-6.0, 4.0, 22.0)
        + 4.0 * hill(6.0, -4.0, 18.0)
        + 0.45 * (x * 0.6).sin() * (y * 0.5).cos();
    let t = ((z - 0.9) / 6.7).clamp(0.0, 1.0);
    let colors = [
        [60.0, 136.0, 218.0],
        [66.0, 208.0, 165.0],
        [240.0, 181.0, 75.0],
        [224.0, 95.0, 143.0],
    ];
    let s = t * 3.0;
    let lo = (s as usize).min(2);
    let f = s - lo as f32;
    let rgb: [u8; 3] = std::array::from_fn(|channel| {
        (colors[lo][channel] * (1.0 - f) + colors[lo + 1][channel] * f).round() as u8
    });
    ([x, y, z], [rgb[0], rgb[1], rgb[2], 255])
}

pub fn survey_pages(points: u32) -> u64 {
    u64::from(points.div_ceil(SURVEY_PAGE))
}

/// Encodes one survey page; `None` when `page_id` is outside the survey.
pub fn survey_page(points: u32, page_id: u64) -> Option<(PageDescriptor, Vec<u8>)> {
    if points > SURVEY_MAX_POINTS || page_id >= survey_pages(points) {
        return None;
    }
    let side = survey_side(points);
    let start = u32::try_from(page_id).ok()? * SURVEY_PAGE;
    let count = SURVEY_PAGE.min(points - start);
    let mut columns = [12, 8, 4].map(|stride| ColumnData {
        values: Vec::with_capacity(count as usize * stride),
        validity: vec![],
    });
    for index in start..start + count {
        let (position, color) = survey_sample(index, side);
        columns[0]
            .values
            .extend(position.into_iter().flat_map(f32::to_le_bytes));
        columns[1]
            .values
            .extend((u64::MAX - u64::from(index)).to_le_bytes());
        columns[2].values.extend(color);
    }
    let bytes = CloudPage {
        count,
        columns: columns.into(),
    }
    .encode(&survey_schema())
    .ok()?;
    let descriptor = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: count,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    Some((descriptor, bytes))
}

pub fn survey_scan(points: u32) -> ScanDescriptor {
    ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: Some(SURVEY_SOURCE.into()),
        frame: DataFrame::default(),
        source_count: u64::from(points),
        organization: None,
    }
}

/// Regenerates exact survey pages on demand.
pub struct SurveyProvider {
    points: u32,
}

impl PageProvider for SurveyProvider {
    fn owned_bytes(&self) -> u64 {
        0
    }

    fn availability(&self, page: &PageDescriptor) -> ProviderAvailability {
        if page.address.scan_key == 0 && page.address.page_id < survey_pages(self.points) {
            ProviderAvailability::Available
        } else {
            ProviderAvailability::Missing
        }
    }

    fn load(&self, sink: &mut PageSink<'_>) -> Result<(), ProviderError> {
        sink.check()?;
        let scratch = sink.descriptor().byte_len * 2;
        let _scratch = sink
            .budget()
            .reserve(scratch)?
            .charge(scratch, MemoryClass::Scratch)?;
        let (_, bytes) = survey_page(self.points, sink.descriptor().address.page_id)
            .ok_or(ProviderError::Unavailable(ProviderAvailability::Missing))?;
        for chunk in bytes.chunks(DATA_CHUNK_BYTES) {
            sink.push(chunk)?;
        }
        Ok(())
    }
}

/// Builds a [`SurveyProvider`] for datasets declaring the survey scan and schema.
pub struct SurveyFactory;

impl ProviderFactory for SurveyFactory {
    fn name(&self) -> &str {
        SURVEY_PROVIDER
    }

    fn create(
        &self,
        budget: &CpuBudget,
        descriptor: &DataDescriptor,
        scans: &[Arc<ScanMetadata>],
    ) -> Result<Arc<RegisteredProvider>, StorageError> {
        let survey = matches!(&descriptor.shape, DataShape::Cloud { schema } if *schema == survey_schema())
            && matches!(descriptor.stream, StreamMode::RetainedMap);
        let [scan] = scans else {
            return Err(StorageError::Invalid(
                "survey provider requires exactly one scan".into(),
            ));
        };
        let scan = scan.value();
        if !survey
            || scan.scan_key != 0
            || scan.source.as_deref() != Some(SURVEY_SOURCE)
            || scan.source_count > u64::from(SURVEY_MAX_POINTS)
        {
            return Err(StorageError::Invalid(
                "dataset is not a survey-heightfield retained map".into(),
            ));
        }
        RegisteredProvider::new(
            budget,
            SurveyProvider {
                points: scan.source_count as u32,
            },
        )
    }
}

/// Registers every trusted page provider a host ships with.
pub fn register_host_providers(
    backend: &super::DrawingBackend,
) -> Result<(), super::TransactionError> {
    if backend
        .provider_names()
        .iter()
        .any(|name| name == SURVEY_PROVIDER)
    {
        return Ok(());
    }
    backend.register_provider_factory(Arc::new(SurveyFactory))
}
