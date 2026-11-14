use super::*;

fn number(value: &serde_json::Value, field: &str) -> u64 {
    value[field]
        .as_u64()
        .expect("required CPU lifecycle observation")
}

fn record(control: &Control, name: &str, report: &Report) -> Result<()> {
    require(
        number(&report.cpu, "committed") <= number(&report.cpu, "limit"),
        "CPU cap exceeded",
    )?;
    std::fs::write(
        control.root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(report)?,
    )?;
    Ok(())
}

fn points(report: &Report, count: u64) -> bool {
    number(&report.allocations, "points") == count
        && number(&report.allocations, "staging_gpu_bytes") == 0
        && number(&report.allocations, "retiring_gpu_bytes") == 0
}

pub(super) fn run(
    control: &mut Control,
    data: &mut DrawDataClient,
    writer: &mut DrawDataWriter,
) -> Result<()> {
    let key = DataKey::new(NS, "cloud")?;
    let neighbor = DataKey::new(&format!("{NS}-neighbor"), "cloud")?;
    control.set("configure", true, 0)?;
    let baseline = control.wait("baseline", |r| {
        r.views.iter().all(Option::is_some) && points(r, 0)
    })?;
    record(control, "baseline", &baseline)?;
    let first = upload_at(writer, &key, None, [255, 24, 24, 255], -2.5)?;
    let second = upload_at(writer, &neighbor, None, [24, 24, 255, 255], 2.5)?;
    let initial = control.wait("initial", |r| {
        points(r, 4) && pair(r, first.generation, first.generation)
    })?;
    for view in initial.views {
        data.wait_visible_with_deadline(&key, first, view, deadline())?;
        data.wait_visible_with_deadline(&neighbor, second, view, deadline())?;
    }
    record(control, "initial", &initial)?;
    let source_bytes = number(&initial.cpu, "page_bytes") - number(&baseline.cpu, "page_bytes");
    require(
        source_bytes > 0 && source_bytes % 2 == 0,
        "missing symmetric source-page ownership",
    )?;
    control.capture("initial", 0, ["red+blue", "red+blue"])?;
    control.cpu_headroom = Some(0);
    control.set("configure", true, 0)?;
    let pressure = control.wait("pressure", |r| {
        number(&r.cpu, "held") > 0 && number(&r.cpu, "committed") == number(&r.cpu, "limit")
    })?;
    record(control, "pressure", &pressure)?;
    clear(data, writer.lease())?;
    let partial = control.wait("namespace-cleared", |r| {
        points(r, 2)
            && number(&r.cpu, "held") > 0
            && number(&r.cpu, "page_bytes")
                == number(&baseline.cpu, "page_bytes") + source_bytes / 2
            && number(&r.cpu, "snapshot_failures") > number(&pressure.cpu, "snapshot_failures")
    })?;
    record(control, "namespace-cleared", &partial)?;
    control.capture("namespace-cleared", 0, ["blue", "blue"])?;
    call(
        data,
        DataRequest {
            lease: writer.lease(),
            incarnation: second.incarnation,
            base_generation: second.generation,
            ..DataRequest::new(data_op::DELETE)
        }
        .with_key(&neighbor)?,
    )?;
    let deleted = control.wait("deleted", |r| {
        points(r, 0)
            && number(&r.allocations, "gpu_bytes") == 0
            && number(&r.cpu, "held") > 0
            && number(&r.cpu, "allocated") <= number(&baseline.cpu, "allocated")
            && number(&r.cpu, "page_bytes") == number(&baseline.cpu, "page_bytes")
            && number(&r.cpu, "snapshot_failures") > number(&partial.cpu, "snapshot_failures")
    })?;
    record(control, "deleted", &deleted)?;
    control.capture("deleted", 0, ["none", "none"])?;
    control.cpu_headroom = None;
    control.set("configure", true, 0)?;
    let released = control.wait("released", |r| {
        points(r, 0)
            && number(&r.cpu, "held") == 0
            && number(&r.cpu, "allocated") <= number(&baseline.cpu, "allocated")
    })?;
    record(control, "released", &released)?;
    let recreated = upload(writer, &key, None, [255, 24, 255, 255])?;
    require(
        recreated.incarnation != first.incarnation,
        "recreation reused retired identity",
    )?;
    let recovered = control.wait("recreated", |r| {
        points(r, 2)
            && r.completed
                .iter()
                .all(|stamp| stamp.is_some_and(|s| s.incarnation == recreated.incarnation))
    })?;
    for view in recovered.views {
        data.wait_visible_with_deadline(&key, recreated, view, deadline())?;
    }
    record(control, "recreated", &recovered)?;
    control.capture("recreated", 0, ["magenta", "magenta"])?;
    clear(data, writer.lease())?;
    writer.close(deadline())?;
    let final_state = control.wait("final-clear", |r| {
        points(r, 0)
            && number(&r.allocations, "gpu_bytes") == 0
            && number(&r.cpu, "allocated") <= number(&baseline.cpu, "allocated")
    })?;
    record(control, "final-clear", &final_state)?;
    std::fs::write(
        control.root.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "result":"pass", "scope":"native zero-headroom snapshot denial, namespace clear, neighbor survival, delete, release and recreation",
            "baseline_cpu":baseline.cpu, "deleted_under_pressure_cpu":deleted.cpu, "released_cpu":released.cpu,
        "note":"Source pages and CPU allocations return to baseline while the pressure reservation remains active"
        }))?,
    )?;
    Ok(())
}
