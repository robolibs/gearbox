use super::*;
use gearbox_api::{draw_kind, draw_op};

fn legacy_summary(
    client: &Client,
    namespace: &str,
    id: &str,
    kind: u32,
    points: u64,
) -> Result<()> {
    let listed = client.draw_list(namespace)?;
    let item = listed
        .iter()
        .find(|item| item.id() == id)
        .ok_or("legacy summary missing")?;
    ensure(
        item.kind == kind
            && item.props().get("points").as_deref() == Some(points.to_string().as_str()),
        "legacy replacement changed kind or point count",
    )
}

fn attacks(client: &Client, key: &DataKey) -> Result<()> {
    for operation in [
        draw_op::UPSERT,
        draw_op::APPEND,
        draw_op::EXTEND,
        draw_op::UPDATE,
        draw_op::PATCH,
    ] {
        let mut entity = DrawEntity::points(&key.namespace, &key.id, &[[3.0; 3]]);
        entity.op = operation;
        if operation == draw_op::APPEND {
            entity.seq = 42;
            entity.last = 0;
        }
        let result = client.draw(&entity)?;
        ensure(
            !result.is_ok() && result.message().contains("kind_conflict"),
            "legacy mutation did not explicitly reject dataset ownership",
        )?;
    }
    ensure(
        !client
            .draw_list(&key.namespace)?
            .iter()
            .any(|item| item.id() == key.id),
        "legacy list leaked a dataset or failed append",
    )
}

pub(super) fn verify(
    client: &Client,
    data: &mut DrawDataClient,
    template: &DataDescriptor,
    retained: DatasetRevision,
) -> Result<()> {
    let namespace = format!("{}/compatibility", template.key.namespace);
    let epoch = data.capabilities().host_epoch;
    let lease = checked(data, &request(data_op::OPEN_WRITER, epoch, 0))?.lease;
    let sequence = std::cell::Cell::new(1u64);
    let next = || {
        let id = sequence.get();
        sequence.set(id + 1);
        id
    };
    let empty = |descriptor: &DataDescriptor, base, mode| {
        begin(epoch, lease, next(), descriptor, base, mode, 0, 0, 0)
    };
    let result = (|| -> Result<()> {
        let points = DrawEntity::points(&namespace, "legacy", &[[1.0; 3]]);
        ensure(
            client.draw(&points)?.is_ok(),
            "legacy initial points failed",
        )?;
        let mut lines = DrawEntity::lines(&namespace, "legacy", &[[1.0; 3], [2.0; 3]]);
        ensure(
            client.draw(&lines)?.is_ok(),
            "legacy kind-changing upsert failed",
        )?;
        legacy_summary(client, &namespace, "legacy", draw_kind::LINES, 2)?;
        let mut first = points.clone();
        first.op = draw_op::APPEND;
        first.seq = 7;
        first.last = 0;
        let mut last = first.clone();
        last.chunk_offset = 1;
        last.last = 1;
        ensure(client.draw(&first)?.is_ok(), "legacy first append failed")?;
        legacy_summary(client, &namespace, "legacy", draw_kind::LINES, 2)?;
        ensure(
            client.draw(&last)?.is_ok(),
            "legacy completed append failed",
        )?;
        legacy_summary(client, &namespace, "legacy", draw_kind::POINTS, 2)?;
        lines.op = draw_op::EXTEND;
        ensure(
            client.draw(&lines)?.is_ok(),
            "legacy kind-changing extend failed",
        )?;
        legacy_summary(client, &namespace, "legacy", draw_kind::LINES, 2)?;

        let before = status(data, epoch, &template.key)?;
        attacks(client, &template.key)?;
        let after = status(data, epoch, &template.key)?;
        ensure(
            after.retained == Some(retained)
                && before.resources.resident_page_bytes == after.resources.resident_page_bytes,
            "legacy attacks changed retained generation or data",
        )?;
        let pending = empty(template, Some(retained), CommitMode::Delta);
        checked(data, &pending)?;
        checked(data, &edit(&pending, data_op::CANCEL))?;

        let mut descriptor = template.clone();
        descriptor.key = DataKey::new(&namespace, "reserved")?;
        let reserved = empty(&descriptor, None, CommitMode::Replace);
        checked(data, &reserved)?;
        attacks(client, &descriptor.key)?;
        checked(data, &edit(&reserved, data_op::CANCEL))?;
        let retry = empty(&descriptor, None, CommitMode::Replace);
        checked(data, &retry)?;
        let first_data = checked(data, &edit(&retry, data_op::COMMIT))?;
        attacks(client, &descriptor.key)?;
        let delta = empty(&descriptor, Some(revision(&first_data)), CommitMode::Delta);
        checked(data, &delta)?;
        let second_data = checked(data, &edit(&delta, data_op::COMMIT))?;
        ensure(
            second_data.stored_generation == 2,
            "rejected legacy append left pending ownership",
        )?;

        let mut old_first = DrawEntity::points(&namespace, "legacy-pending", &[[0.0; 3]]);
        old_first.op = draw_op::APPEND;
        old_first.seq = 9;
        old_first.last = 0;
        ensure(
            client.draw(&old_first)?.is_ok(),
            "legacy pending fixture failed",
        )?;
        let mut contender = template.clone();
        contender.key = DataKey::new(&namespace, "legacy-pending")?;
        let denied = empty(&contender, None, CommitMode::Replace);
        ensure(
            data.request(&denied)?.code == data_code::CONFLICT,
            "typed begin stole incomplete legacy ownership",
        )?;
        let mut old_last = old_first.clone();
        old_last.chunk_offset = 1;
        old_last.last = 1;
        ensure(
            client.draw(&old_last)?.is_ok(),
            "legacy pending completion failed",
        )?;
        let denied = empty(&contender, None, CommitMode::Replace);
        ensure(
            data.request(&denied)?.code == data_code::CONFLICT,
            "typed begin stole live legacy ownership",
        )?;
        ensure(
            client.draw_delete(&namespace, "legacy-pending")?.is_ok(),
            "legacy release failed",
        )?;
        let allowed = empty(&contender, None, CommitMode::Replace);
        checked(data, &allowed)?;
        checked(data, &edit(&allowed, data_op::COMMIT))?;

        let mut octree = contracts::octree();
        octree.key = descriptor.key.clone();
        let wrong_kind = empty(&octree, Some(revision(&second_data)), CommitMode::Replace);
        ensure(
            data.request(&wrong_kind)?.code == data_code::CONFLICT,
            "dataset kind changed without deletion",
        )?;
        ensure(
            client.draw_delete(&namespace, "reserved")?.is_ok(),
            "old delete did not release cloud ownership",
        )?;
        let replacement = empty(&octree, None, CommitMode::Replace);
        checked(data, &replacement)?;
        let replaced = checked(data, &edit(&replacement, data_op::COMMIT))?;
        ensure(
            replaced.incarnation != first_data.incarnation && replaced.stored_generation == 1,
            "kind replacement reused identity",
        )?;
        let legacy = client.draw_list(&namespace)?;
        ensure(
            legacy.len() == 1 && legacy[0].id() == "legacy",
            "old list returned a typed or pending entry",
        )?;

        let neighbor_ns = format!("{namespace}-neighbor");
        let mut neighbor = template.clone();
        neighbor.key = DataKey::new(&neighbor_ns, "data")?;
        let neighbor_begin = empty(&neighbor, None, CommitMode::Replace);
        checked(data, &neighbor_begin)?;
        let neighbor_revision = revision(&checked(data, &edit(&neighbor_begin, data_op::COMMIT))?);
        ensure(
            client
                .draw(&DrawEntity::points(&neighbor_ns, "legacy", &[[0.0; 3]]))?
                .is_ok(),
            "neighbor fixture failed",
        )?;

        let DataShape::Octree { config, .. } = &octree.shape else {
            unreachable!()
        };
        let payload = encode_occupancy_page(
            config,
            &[OccupancyLeaf {
                key: VoxelKey {
                    tile: [0; 3],
                    depth: 1,
                    morton: 0,
                },
                value: OccupancyValue {
                    probability: Some(0.8),
                    confidence: None,
                    class_id: None,
                },
            }],
        )?;
        let page = PageDescriptor {
            address: PageAddress {
                scan_key: 0,
                page_id: 0,
            },
            encoding: PageEncoding::OccupancyLeavesV1,
            items: 1,
            byte_len: payload.len() as u64,
            crc32: page_crc32(&payload),
            origin_m: [0.0; 3],
        };
        let delayed = begin(
            epoch,
            lease,
            next(),
            &octree,
            Some(revision(&replaced)),
            CommitMode::Replace,
            0,
            1,
            page.byte_len,
        );
        checked(data, &delayed)?;
        checked(
            data,
            &edit(&delayed, data_op::DECLARE_PAGE).with_metadata(&page)?,
        )?;
        let middle = payload.len() / 2;
        let chunk = DataRequest {
            payload: payload[..middle].to_vec(),
            ..edit(&delayed, data_op::CHUNK)
        };
        checked(data, &chunk)?;
        ensure(
            client.draw_clear(&namespace)?.is_ok(),
            "old clear failed on mixed namespace",
        )?;
        ensure(
            data.request(&DataRequest {
                offset: middle as u64,
                payload: payload[middle..].to_vec(),
                ..chunk
            })?
            .code
                == data_code::HISTORY_EXPIRED,
            "late chunk resurrected cleared data",
        )?;
        ensure(
            data.request(&edit(&delayed, data_op::COMMIT))?.code == data_code::HISTORY_EXPIRED,
            "late commit resurrected cleared data",
        )?;
        let listing = request(data_op::LIST, epoch, 0).with_metadata(&DataListQuery {
            namespace: namespace.clone(),
            limit: 128,
            cursor: None,
        })?;
        ensure(
            DrawSummaryPage::from_json(&checked(data, &listing)?.metadata)?
                .items
                .is_empty(),
            "old clear left mixed objects",
        )?;
        ensure(
            status(data, epoch, &neighbor.key)?.retained == Some(neighbor_revision),
            "namespace clear deleted neighbor data",
        )?;
        legacy_summary(client, &neighbor_ns, "legacy", draw_kind::POINTS, 1)?;
        ensure(
            client.draw_clear(&neighbor_ns)?.is_ok(),
            "neighbor cleanup failed",
        )?;
        Ok(())
    })();
    let clean = client.draw_clear(&namespace);
    let neighbor = client.draw_clear(&format!("{namespace}-neighbor"));
    let close = checked(data, &request(data_op::CLOSE_WRITER, epoch, lease));
    result?;
    ensure(
        clean?.is_ok() && neighbor?.is_ok(),
        "compatibility cleanup failed",
    )?;
    close?;
    Ok(())
}
