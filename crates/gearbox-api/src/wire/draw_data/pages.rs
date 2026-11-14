use serde::{Deserialize, Serialize};

use super::{
    AttributeSemantic, CloudPage, CloudSchema, ColumnData, ColumnView, MAX_ATTRIBUTES,
    MAX_PAGE_BYTES, MAX_PAGE_POINTS, OccupancyLeaf, OccupancyValue, OctreeConfig, VoxelKey,
    validate_leaf_set,
};

const HEADER_BYTES: usize = 16;
const LEAF_BYTES: usize = 48;
pub const MAX_PAGE_LEAVES: u32 = 65_536;
pub const MAX_ENCODED_PAGE_BYTES: usize = MAX_PAGE_BYTES + HEADER_BYTES + 8 * MAX_ATTRIBUTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageAddress {
    pub scan_key: u64,
    pub page_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageEncoding {
    CloudColumnsV1,
    OccupancyLeavesV1,
    /// One indexed triangle block; `items` is its triangle count.
    MeshBlockV1,
    /// One sparse volume block; `items` is its sample count, `page_id` its packed key.
    VolumeBlockV1,
    /// One layer of one elevation tile; `items` is its sample count, `page_id` packs tile and layer.
    ElevationTileV1,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageDescriptor {
    pub address: PageAddress,
    pub encoding: PageEncoding,
    pub items: u32,
    pub byte_len: u64,
    pub crc32: u32,
    pub origin_m: [f64; 3],
}

impl PageDescriptor {
    pub fn validate(&self) -> Result<(), String> {
        if self.byte_len < HEADER_BYTES as u64
            || self.byte_len > MAX_ENCODED_PAGE_BYTES as u64
            || self.origin_m.iter().any(|v| !v.is_finite())
        {
            return Err("page size/origin is invalid".into());
        }
        match self.encoding {
            PageEncoding::CloudColumnsV1 if self.items > MAX_PAGE_POINTS => {
                Err("cloud page count exceeds limit".into())
            }
            PageEncoding::OccupancyLeavesV1
                if self.items > MAX_PAGE_LEAVES
                    || self.byte_len
                        != HEADER_BYTES as u64 + u64::from(self.items) * LEAF_BYTES as u64
                    || self.address.scan_key != 0
                    || self.origin_m != [0.0; 3] =>
            {
                Err("occupancy page size/count/scan/origin is invalid".into())
            }
            PageEncoding::MeshBlockV1 if self.items == 0 || self.items > super::mesh::MAX_MESH_TRIANGLES => {
                Err("mesh block triangle count is invalid".into())
            }
            PageEncoding::VolumeBlockV1
                if !matches!(self.items, 512 | 4096)
                    || self.address.scan_key != 0
                    || self.address.page_id >> 63 != 0
                    || self.origin_m != [0.0; 3] =>
            {
                Err("volume block count/scan/key/origin is invalid".into())
            }
            PageEncoding::ElevationTileV1
                if !(64..=16_384).contains(&self.items)
                    || self.address.scan_key != 0
                    || self.address.page_id >> 52 != 0
                    || self.origin_m != [0.0; 3] =>
            {
                Err("elevation tile count/scan/key/origin is invalid".into())
            }
            _ => Ok(()),
        }
    }

    pub fn verify(&self, bytes: &[u8]) -> Result<(), String> {
        self.validate()?;
        if bytes.len() as u64 != self.byte_len || page_crc32(bytes) != self.crc32 {
            return Err("page length or CRC32 mismatch".into());
        }
        Ok(())
    }

    pub fn decode_cloud(&self, schema: &CloudSchema, bytes: &[u8]) -> Result<CloudPage, String> {
        self.verify(bytes)?;
        if self.encoding != PageEncoding::CloudColumnsV1 {
            return Err("page encoding is not cloud columns".into());
        }
        if u32_at(bytes, 4) != self.items {
            return Err("declared cloud count mismatch".into());
        }
        CloudPage::decode(schema, bytes)
    }

    pub fn decode_occupancy(
        &self,
        config: &OctreeConfig,
        bytes: &[u8],
    ) -> Result<Vec<OccupancyLeaf>, String> {
        self.verify(bytes)?;
        if self.encoding != PageEncoding::OccupancyLeavesV1 {
            return Err("page encoding is not occupancy leaves".into());
        }
        if u32_at(bytes, 4) != self.items {
            return Err("declared occupancy count mismatch".into());
        }
        decode_occupancy_page(config, bytes)
    }
}

/// CRC-32/ISO-HDLC over the complete encoded page, including its header.
pub fn page_crc32(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}

impl CloudPage {
    pub fn encode(&self, schema: &CloudSchema) -> Result<Vec<u8>, String> {
        self.validate(schema)?;
        let payload: usize = self
            .columns
            .iter()
            .map(|column| column.values.len() + column.validity.len())
            .sum();
        let mut out = Vec::with_capacity(HEADER_BYTES + self.columns.len() * 8 + payload);
        out.extend_from_slice(b"GBC1");
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&(self.columns.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for column in &self.columns {
            out.extend_from_slice(&(column.values.len() as u32).to_le_bytes());
            out.extend_from_slice(&(column.validity.len() as u32).to_le_bytes());
        }
        for column in &self.columns {
            out.extend_from_slice(&column.values);
            out.extend_from_slice(&column.validity);
        }
        Ok(out)
    }

    pub fn decode(schema: &CloudSchema, bytes: &[u8]) -> Result<Self, String> {
        let view = CloudPageView::layout(schema, bytes)?;
        let page = Self {
            count: view.count,
            columns: view
                .columns()
                .iter()
                .map(|column| ColumnData {
                    values: column.values.to_vec(),
                    validity: column.validity.to_vec(),
                })
                .collect(),
        };
        page.validate(schema)?;
        Ok(page)
    }
}

#[derive(Debug)]
pub struct CloudPageView<'a> {
    count: u32,
    columns: [ColumnView<'a>; MAX_ATTRIBUTES],
    column_count: usize,
    source_id_column: usize,
}

impl<'a> CloudPageView<'a> {
    fn layout(schema: &CloudSchema, bytes: &'a [u8]) -> Result<Self, String> {
        schema.validate()?;
        check_header(bytes, b"GBC1")?;
        let count = u32_at(bytes, 4);
        let columns = u32_at(bytes, 8) as usize;
        if count > MAX_PAGE_POINTS || columns != schema.attributes.len() || u32_at(bytes, 12) != 0 {
            return Err("invalid cloud page count, columns or reserved bits".into());
        }
        let header = HEADER_BYTES + columns * 8;
        if bytes.len() < header {
            return Err("truncated cloud column table".into());
        }
        let mut payload = 0usize;
        for (index, field) in schema.attributes.iter().enumerate() {
            let values = u32_at(bytes, HEADER_BYTES + index * 8) as usize;
            let validity = u32_at(bytes, HEADER_BYTES + index * 8 + 4) as usize;
            let mask_bytes = (count as usize).div_ceil(8);
            if values != field.byte_len(count.into())?
                || (validity != 0 && (!field.nullable || validity != mask_bytes))
            {
                return Err("cloud column lengths do not match schema/count".into());
            }
            payload = payload
                .checked_add(values)
                .and_then(|n| n.checked_add(validity))
                .ok_or("cloud column length overflow")?;
        }
        if payload > MAX_PAGE_BYTES || header.checked_add(payload) != Some(bytes.len()) {
            return Err("cloud payload length mismatch or page limit exceeded".into());
        }
        let mut cursor = header;
        let mut data = [ColumnView::default(); MAX_ATTRIBUTES];
        for index in 0..columns {
            let values = u32_at(bytes, HEADER_BYTES + index * 8) as usize;
            let validity = u32_at(bytes, HEADER_BYTES + index * 8 + 4) as usize;
            data[index] = ColumnView {
                values: &bytes[cursor..cursor + values],
                validity: &bytes[cursor + values..cursor + values + validity],
            };
            cursor += values + validity;
        }
        Ok(Self {
            count,
            columns: data,
            column_count: columns,
            source_id_column: schema
                .attributes
                .iter()
                .position(|field| field.semantic == AttributeSemantic::SourceId)
                .unwrap(),
        })
    }

    /// Validates packed columns with caller-owned scratch for source-ID uniqueness.
    pub fn validate(
        schema: &CloudSchema,
        bytes: &'a [u8],
        scratch: &mut [u64],
    ) -> Result<Self, String> {
        let view = Self::layout(schema, bytes)?;
        if scratch.len() != view.count as usize {
            return Err("source ID scratch must match the page item count".into());
        }
        for (column, field) in view.columns().iter().zip(&schema.attributes) {
            column.validate(field, view.count)?;
        }
        for (slot, id) in scratch.iter_mut().zip(view.source_ids()) {
            *slot = id;
        }
        scratch.sort_unstable();
        if scratch.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("duplicate source ID within cloud page".into());
        }
        Ok(view)
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn columns(&self) -> &[ColumnView<'a>] {
        &self.columns[..self.column_count]
    }

    pub fn source_id_column(&self) -> usize {
        self.source_id_column
    }

    pub fn source_ids(&self) -> impl ExactSizeIterator<Item = u64> + '_ {
        self.columns[self.source_id_column]
            .values
            .chunks_exact(8)
            .map(|raw| u64::from_le_bytes(raw.try_into().unwrap()))
    }
}

pub fn encode_occupancy_page(
    config: &OctreeConfig,
    leaves: &[OccupancyLeaf],
) -> Result<Vec<u8>, String> {
    if leaves.len() > MAX_PAGE_LEAVES as usize {
        return Err("occupancy page count exceeds limit".into());
    }
    validate_leaf_set(config, leaves)?;
    let mut bytes = Vec::with_capacity(HEADER_BYTES + LEAF_BYTES * leaves.len());
    bytes.extend_from_slice(b"GBO1");
    bytes.extend_from_slice(&(leaves.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&[0; 8]);
    for leaf in leaves {
        for tile in leaf.key.tile {
            bytes.extend_from_slice(&tile.to_le_bytes());
        }
        bytes.extend_from_slice(&leaf.key.morton.to_le_bytes());
        bytes.push(leaf.key.depth);
        bytes.push(
            u8::from(leaf.value.probability.is_some())
                | (u8::from(leaf.value.confidence.is_some()) << 1)
                | (u8::from(leaf.value.class_id.is_some()) << 2),
        );
        bytes.extend_from_slice(&[0; 2]);
        bytes.extend_from_slice(&leaf.value.probability.unwrap_or(0.0).to_le_bytes());
        bytes.extend_from_slice(&leaf.value.confidence.unwrap_or(0.0).to_le_bytes());
        bytes.extend_from_slice(&leaf.value.class_id.unwrap_or(0).to_le_bytes());
    }
    Ok(bytes)
}

pub fn decode_occupancy_page(
    config: &OctreeConfig,
    bytes: &[u8],
) -> Result<Vec<OccupancyLeaf>, String> {
    let count = occupancy_layout(config, bytes)?;
    let leaves: Vec<_> = bytes[HEADER_BYTES..]
        .chunks_exact(LEAF_BYTES)
        .map(occupancy_leaf)
        .collect::<Result<_, _>>()?;
    debug_assert_eq!(leaves.len(), count);
    validate_leaf_set(config, &leaves)?;
    Ok(leaves)
}

fn occupancy_layout(config: &OctreeConfig, bytes: &[u8]) -> Result<usize, String> {
    config.validate()?;
    check_header(bytes, b"GBO1")?;
    let count = u32_at(bytes, 4) as usize;
    if count > MAX_PAGE_LEAVES as usize
        || bytes[8..16] != [0; 8]
        || count
            .checked_mul(LEAF_BYTES)
            .and_then(|n| n.checked_add(HEADER_BYTES))
            != Some(bytes.len())
    {
        return Err("invalid occupancy page count, length or reserved bits".into());
    }
    Ok(count)
}

pub(crate) fn occupancy_leaf(record: &[u8]) -> Result<OccupancyLeaf, String> {
    let flags = record[33];
    if flags & !7 != 0 || record[34..36] != [0; 2] {
        return Err("invalid occupancy record flags/reserved bits".into());
    }
    for bit in 0..3 {
        if flags & (1 << bit) == 0 && record[36 + bit * 4..40 + bit * 4] != [0; 4] {
            return Err("absent occupancy attributes must have zero payload".into());
        }
    }
    Ok(OccupancyLeaf {
        key: VoxelKey {
            tile: std::array::from_fn(|i| {
                i64::from_le_bytes(record[i * 8..i * 8 + 8].try_into().unwrap())
            }),
            morton: u64::from_le_bytes(record[24..32].try_into().unwrap()),
            depth: record[32],
        },
        value: OccupancyValue {
            probability: (flags & 1 != 0).then(|| f32::from_bits(u32_at(record, 36))),
            confidence: (flags & 2 != 0).then(|| f32::from_bits(u32_at(record, 40))),
            class_id: (flags & 4 != 0).then(|| u32_at(record, 44)),
        },
    })
}

#[derive(Debug)]
pub struct OccupancyPageView<'a> {
    bytes: &'a [u8],
}

impl<'a> OccupancyPageView<'a> {
    /// Validates packed leaves using caller-owned Morton-order scratch.
    pub fn validate(
        config: &OctreeConfig,
        bytes: &'a [u8],
        scratch: &mut [VoxelKey],
    ) -> Result<Self, String> {
        let count = occupancy_layout(config, bytes)?;
        if scratch.len() != count {
            return Err("occupancy scratch must match the page item count".into());
        }
        for (slot, record) in scratch
            .iter_mut()
            .zip(bytes[HEADER_BYTES..].chunks_exact(LEAF_BYTES))
        {
            let leaf = occupancy_leaf(record)?;
            leaf.key.validate(config)?;
            leaf.value.validate()?;
            *slot = leaf.key;
        }
        scratch.sort_unstable_by_key(|key| {
            (
                key.tile,
                key.morton << (3 * (config.max_depth - key.depth)),
                key.depth,
            )
        });
        if scratch.windows(2).any(|pair| pair[0].contains(&pair[1])) {
            return Err("duplicate or overlapping occupancy leaves".into());
        }
        Ok(Self { bytes })
    }

    pub fn leaves(&self) -> impl ExactSizeIterator<Item = OccupancyLeaf> + '_ {
        self.bytes[HEADER_BYTES..]
            .chunks_exact(LEAF_BYTES)
            .map(|record| occupancy_leaf(record).unwrap())
    }
}

fn check_header(bytes: &[u8], magic: &[u8; 4]) -> Result<(), String> {
    if bytes.len() < HEADER_BYTES || bytes.len() > MAX_ENCODED_PAGE_BYTES || &bytes[..4] != magic {
        return Err("invalid page magic or encoded byte limit".into());
    }
    Ok(())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
