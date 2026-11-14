use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub(super) struct Uploads {
    positions: u64,
    attributes: u64,
    styles: u64,
    placements: u64,
}

impl Uploads {
    fn total(&self) -> u64 {
        self.positions + self.attributes + self.styles + self.placements
    }

    fn difference(&self, before: &Self) -> Result<Self> {
        let difference =
            |after: u64, before: u64| after.checked_sub(before).ok_or("upload counter decreased");
        Ok(Self {
            positions: difference(self.positions, before.positions)?,
            attributes: difference(self.attributes, before.attributes)?,
            styles: difference(self.styles, before.styles)?,
            placements: difference(self.placements, before.placements)?,
        })
    }

    fn verify(&self, items: u32, allocated: &Allocations, total: u64) -> Result<()> {
        require(
            *self
                == Self {
                    positions: u64::from(items) * 12,
                    attributes: u64::from(items) * 4,
                    styles: allocated.styles,
                    placements: allocated.placements,
                }
                && self.total() == total,
            &format!("single-page upload mismatch: {self:?}, items {items}, total {total}"),
        )
    }
}

fn changed_page(source: &(PageDescriptor, Vec<u8>)) -> Result<(PageDescriptor, Vec<u8>)> {
    let mut page = source.0.decode_cloud(&fixture::schema(), &source.1)?;
    for point in page.columns[0].values.chunks_exact_mut(12) {
        let z = f32::from_le_bytes(point[8..12].try_into()?) + 0.25;
        point[8..12].copy_from_slice(&z.to_le_bytes());
    }
    for color in page.columns[2].values.chunks_exact_mut(4) {
        color[0] = color[0].wrapping_add(23);
    }
    let bytes = page.encode(&fixture::schema())?;
    let descriptor = PageDescriptor {
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        ..source.0.clone()
    };
    require(
        descriptor.crc32 != source.0.crc32,
        "delta must change source bytes",
    )?;
    Ok((descriptor, bytes))
}

pub(super) fn run(
    data: &mut DrawDataClient,
    writer: &mut DrawDataWriter,
    fixture: &fixture::Fixture,
    root: &Path,
    mut revision: DatasetRevision,
    presentation: Presentation,
) -> Result<()> {
    let descriptor = DataDescriptor {
        presentation,
        ..fixture.descriptor.clone()
    };
    let limits = data.capabilities().limits.clone();
    let mut records = Vec::new();
    for index in [7, fixture.pages.len() - 1] {
        let before = wait_report(root, "before page delta", |value| {
            value.allocations.million()
        })?;
        let (page, bytes) = changed_page(&fixture.pages[index])?;
        let pages = [DataUploadPage {
            descriptor: &page,
            bytes: &bytes,
        }];
        let update = DataUpload {
            descriptor: &descriptor,
            mode: CommitMode::Delta,
            base: Some(revision),
            scans: &[],
            pages: &pages,
            removed_pages: &[],
            removed_scans: &[],
        };
        let reply = writer.upload(&update, deadline(), None)?;
        let next = DatasetRevision {
            incarnation: reply.incarnation,
            generation: reply.stored_generation,
        };
        require(
            next.incarnation == revision.incarnation && next.generation == revision.generation + 1,
            "page delta did not preserve incarnation and advance its exact base",
        )?;
        let receipt = visible(data, &descriptor.key, next)?;
        let after = wait_report(root, "page delta", |value| {
            value.allocations.million() && value.allocations.uploaded > before.allocations.uploaded
        })?;
        let uploaded = after.uploads.difference(&before.uploads)?;
        let total = after
            .allocations
            .uploaded
            .checked_sub(before.allocations.uploaded)
            .ok_or("total uploads decreased")?;
        uploaded.verify(page.items, &after.allocations, total)?;
        require(
            after.allocations.max_frame_upload <= limits.upload_bytes_per_frame
                && after.allocations.gpu_peak <= limits.gpu_bytes,
            "page delta exceeded GPU or per-frame upload budget",
        )?;
        let current = status(data, &descriptor.key)?;
        require(
            current.failure.is_none() && current.resources.gpu_bytes == after.allocations.gpu_bytes,
            "page delta status disagrees with live GPU resources",
        )?;
        records.push(serde_json::json!({
            "page":page, "base":revision, "revision":next, "visibility":receipt,
            "uploaded_delta":uploaded, "uploaded_total":total,
            "before_uploads":before.uploads, "after_uploads":after.uploads,
            "allocations":after.allocations, "status":current
        }));
        revision = next;
    }
    record(
        root,
        "page-delta",
        serde_json::json!({"result":"pass", "deltas":records}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_and_partial_page_edits_preserve_ids_and_change_position_and_color() {
        let value = fixture::make(DataKey::new(NS, "delta").unwrap(), false).unwrap();
        for index in [7, 15] {
            let source = &value.pages[index];
            let (descriptor, bytes) = changed_page(source).unwrap();
            let old = source
                .0
                .decode_cloud(&fixture::schema(), &source.1)
                .unwrap();
            let new = descriptor.decode_cloud(&fixture::schema(), &bytes).unwrap();
            assert_eq!(descriptor.address, source.0.address);
            assert_eq!(descriptor.items, source.0.items);
            assert_eq!(descriptor.byte_len, source.0.byte_len);
            assert_ne!(descriptor.crc32, source.0.crc32);
            assert_eq!(new.columns[1].values, old.columns[1].values);
            for (new, old) in new.columns[0]
                .values
                .chunks_exact(12)
                .zip(old.columns[0].values.chunks_exact(12))
            {
                assert_eq!(&new[..8], &old[..8]);
                assert_eq!(
                    f32::from_le_bytes(new[8..12].try_into().unwrap()),
                    f32::from_le_bytes(old[8..12].try_into().unwrap()) + 0.25
                );
            }
            for (new, old) in new.columns[2]
                .values
                .chunks_exact(4)
                .zip(old.columns[2].values.chunks_exact(4))
            {
                assert_eq!(new[0], old[0].wrapping_add(23));
                assert_eq!(&new[1..], &old[1..]);
            }
        }
    }

    #[test]
    fn upload_oracle_rejects_extra_records_missing_metadata_and_decreasing_counters() {
        let allocations = Allocations {
            styles: 5184,
            placements: 3328,
            ..Default::default()
        };
        for items in [65_536, 16_960] {
            let expected = Uploads {
                positions: items as u64 * 12,
                attributes: items as u64 * 4,
                styles: allocations.styles,
                placements: allocations.placements,
            };
            expected
                .verify(items, &allocations, expected.total())
                .unwrap();
            for wrong in [
                Uploads {
                    positions: expected.positions + 16,
                    ..expected.clone()
                },
                Uploads {
                    attributes: expected.attributes + 32,
                    ..expected.clone()
                },
                Uploads {
                    styles: 0,
                    ..expected.clone()
                },
                Uploads {
                    placements: 0,
                    ..expected.clone()
                },
            ] {
                assert!(wrong.verify(items, &allocations, wrong.total()).is_err());
            }
            assert!(
                expected
                    .verify(items, &allocations, expected.total() + 4)
                    .is_err()
            );
            assert!(Uploads::default().difference(&expected).is_err());
            assert_eq!(expected.difference(&expected).unwrap(), Uploads::default());
        }
    }
}
