//! Read-only audit of every USD package under an asset root: what the
//! compiler makes of each machine, which profile it uses, its variant
//! selections and the layers it depends on. Never writes to the root.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::compile::{CompileOptions, Selection};
use super::diagnostics::Severity;
use super::manifest;
use super::model::ProfileKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// Machines that declare a schemaVersion must be accepted.
    Declared,
    /// Every machine must be a schemaVersion 2 machine and accepted.
    V2,
}

pub struct AuditArgs {
    pub asset_root: PathBuf,
    pub out: Option<PathBuf>,
    pub requirement: Requirement,
}

pub fn parse_args(args: &[String]) -> Result<AuditArgs, String> {
    let mut root = None;
    let mut out = None;
    let mut requirement = Requirement::Declared;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (key, inline) = match arg.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (arg.as_str(), None),
        };
        let mut value = || inline.clone().or_else(|| it.next().cloned()).ok_or_else(|| format!("{key} needs a value"));
        match key {
            "--asset-root" => root = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--require" => {
                requirement = match value()?.as_str() {
                    "declared" => Requirement::Declared,
                    "v2" => Requirement::V2,
                    other => return Err(format!("--require {other}: expected declared or v2")),
                }
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let asset_root = root.ok_or("--asset-root DIR is required")?;
    if !asset_root.is_dir() {
        return Err(format!("{} is not a directory", asset_root.display()));
    }
    Ok(AuditArgs { asset_root, out, requirement })
}

fn packages(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(path.extension().and_then(|e| e.to_str()), Some("usd" | "usda" | "usdc" | "usdz")) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn variants(stage: &openusd::usd::Stage, root: &str) -> Value {
    let Ok(prim) = stage.prim(root) else { return Value::Null };
    let sets = prim.variant_sets();
    let names = sets.names().unwrap_or_default();
    if names.is_empty() {
        return Value::Null;
    }
    let selections: BTreeMap<String, String> = sets.get_all_variant_selections().unwrap_or_default().into_iter().collect();
    let options: BTreeMap<String, Vec<String>> = names.iter().map(|n| (n.clone(), sets.variant_names(n).unwrap_or_default())).collect();
    json!({ "options": options, "selected": selections })
}

/// Layers the package composes, flagging any that live outside the asset root.
fn dependencies(stage: &openusd::usd::Stage, root: &Path, package: &Path) -> Value {
    let layers = stage.layer_identifiers();
    let package_text = package.to_string_lossy().to_string();
    let root_text = root.to_string_lossy().to_string();
    let external: Vec<&String> = layers
        .iter()
        .filter(|l| l.starts_with('/') && !l.starts_with(&root_text) && !l.starts_with(&package_text))
        .collect();
    json!({ "layers": layers.len(), "external": external })
}

pub fn audit(args: &AuditArgs) -> (Value, bool) {
    let mut assets = Vec::new();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    let mut profiles: BTreeMap<String, usize> = BTreeMap::new();
    let mut codes: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for package in packages(&args.asset_root) {
        let relative = package.strip_prefix(&args.asset_root).unwrap_or(&package).to_string_lossy().to_string();
        let stage = match package.to_str().map(openusd::usd::Stage::open) {
            Some(Ok(stage)) => stage,
            Some(Err(e)) => {
                failures.push(format!("{relative}: does not open: {e}"));
                assets.push(json!({ "package": relative, "error": e.to_string() }));
                continue;
            }
            None => continue,
        };
        let options = CompileOptions { selection: Selection::World, load_instance: relative.clone(), ..Default::default() };
        let result = super::compile(&stage, &options);
        let mut machines = Vec::new();
        for m in &result.machines {
            *versions.entry(m.schema_version.map_or("none".to_string(), |v| v.to_string())).or_default() += 1;
            *profiles.entry(m.profile.as_str().to_string()).or_default() += 1;
            for d in m.diagnostics.iter().filter(|d| d.severity != Severity::Info) {
                *codes.entry(format!("{}:{}", d.severity.as_str(), d.code.as_str())).or_default() += 1;
            }
            let required = match args.requirement {
                Requirement::Declared => m.schema_version.is_some(),
                Requirement::V2 => true,
            };
            if required && (!m.accepted() || (args.requirement == Requirement::V2 && m.profile != ProfileKind::GearboxV2)) {
                failures.push(format!("{relative}{}: {} ({} errors)", m.root, m.profile.as_str(), m.errors().count()));
            }
            let errors: Vec<Value> = m.errors().map(manifest::diagnostic).collect();
            let warnings = m.diagnostics.iter().filter(|d| d.severity == Severity::Warning).count();
            machines.push(json!({
                "root": m.root,
                "machine_id": m.machine_id,
                "profile": m.profile.as_str(),
                "schema_version": m.schema_version,
                "kind": m.kind,
                "accepted": m.accepted(),
                "bodies": m.bodies.len(),
                "joints": m.joints.len(),
                "dofs": m.dofs.len(),
                "frames": m.frames.len(),
                "controllers": m.controllers.iter().map(|c| c.controller_type.clone()).collect::<Vec<_>>(),
                "owned_dofs": m.owners.len(),
                "control_frame": m.control_frame.as_ref().map(|c| c.source.as_str()),
                "variants": variants(&stage, &m.root),
                "errors": errors,
                "warnings": warnings,
            }));
        }
        assets.push(json!({
            "package": relative,
            "scan_root": result.scan_root,
            "units": {
                "meters_per_unit": manifest::canon(result.units.meters_per_unit.value),
                "kilograms_per_unit": manifest::canon(result.units.kilograms_per_unit.value),
                "authored": result.units.meters_per_unit.source.as_str(),
            },
            "dependencies": dependencies(&stage, &args.asset_root, &package),
            "stage_diagnostics": result.diagnostics.iter().map(manifest::diagnostic).collect::<Vec<_>>(),
            "machines": machines,
        }));
    }
    let ok = failures.is_empty();
    let report = json!({
        "manifest_version": super::model::MANIFEST_VERSION,
        "asset_root": args.asset_root.to_string_lossy(),
        "requirement": match args.requirement { Requirement::Declared => "declared", Requirement::V2 => "v2" },
        "schema_versions": versions,
        "profiles": profiles,
        "diagnostic_counts": codes,
        "failures": failures,
        "passed": ok,
        "assets": assets,
    });
    (report, ok)
}

/// `gearbox describe-machines FILE [--world] [--entry PRIM]`: the compiled
/// manifest of one USD file on stdout.
pub fn describe_main(args: &[String]) -> i32 {
    let mut file = None;
    let mut selection = Selection::Asset { entry: None };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--world" => selection = Selection::World,
            "--entry" => match it.next() {
                Some(entry) => selection = Selection::Asset { entry: Some(entry.clone()) },
                None => {
                    eprintln!("describe-machines: --entry needs a prim path");
                    return 2;
                }
            },
            other if file.is_none() => file = Some(PathBuf::from(other)),
            other => {
                eprintln!("describe-machines: unexpected argument {other}");
                return 2;
            }
        }
    }
    let Some(file) = file else {
        eprintln!("describe-machines: FILE is required");
        return 2;
    };
    let options = CompileOptions { selection, load_instance: file.to_string_lossy().to_string(), ..Default::default() };
    match super::compile_file(&file, &options) {
        Ok(result) => {
            print!("{}", manifest::to_string(&manifest::result(&result)));
            if result.machines.iter().all(|m| m.accepted()) { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("describe-machines: {e}");
            2
        }
    }
}

/// `gearbox audit-machines --asset-root DIR [--out FILE] [--require declared|v2]`.
pub fn main(args: &[String]) -> i32 {
    let args = match parse_args(args) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("audit-machines: {e}");
            return 2;
        }
    };
    let (report, ok) = audit(&args);
    let text = manifest::to_string(&report);
    match &args.out {
        Some(out) => {
            if let Some(parent) = out.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(out, &text) {
                eprintln!("audit-machines: cannot write {}: {e}", out.display());
                return 2;
            }
            println!("audit-machines: report written to {}", out.display());
        }
        None => print!("{text}"),
    }
    for failure in report["failures"].as_array().into_iter().flatten() {
        eprintln!("audit-machines: requirement failed: {}", failure.as_str().unwrap_or_default());
    }
    println!(
        "audit-machines: {} packages, profiles {}, schema versions {}",
        report["assets"].as_array().map_or(0, Vec::len),
        report["profiles"],
        report["schema_versions"]
    );
    if ok { 0 } else { 1 }
}
