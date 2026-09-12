//! ISO 11783-10 device element trees discovered from a machine's prims
//! (`specs/TOOLS_SPEC.md` §7.3): authored `GearboxElementAPI` elements plus
//! the derived device, connector and navigation elements.

use std::collections::HashSet;

use bevy::math::DVec3;
use openusd::sdf::{Path as SdfPath, Value};

use crate::controller::{read_attr, read_token};
use crate::links::{CouplingSide, LinkTree, local_offset_between};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    Device,
    Function,
    Bin,
    Section,
    Unit,
    Connector,
    Navigation,
}

impl ElementKind {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "device" => Self::Device,
            "function" => Self::Function,
            "bin" => Self::Bin,
            "section" => Self::Section,
            "unit" => Self::Unit,
            "connector" => Self::Connector,
            "navigation" => Self::Navigation,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Function => "function",
            Self::Bin => "bin",
            Self::Section => "section",
            Self::Unit => "unit",
            Self::Connector => "connector",
            Self::Navigation => "navigation",
        }
    }

    /// ISO 11783-10 DeviceElementType 1..7.
    pub fn iso_type(self) -> u32 {
        match self {
            Self::Device => 1,
            Self::Function => 2,
            Self::Bin => 3,
            Self::Section => 4,
            Self::Unit => 5,
            Self::Connector => 6,
            Self::Navigation => 7,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElementSpec {
    pub prim_path: String,
    pub kind: ElementKind,
    pub number: u32,
    pub designator: String,
    /// Element number of the parent; the device has none.
    pub parent: Option<u32>,
    /// `gearbox:pd:<Ddi>` values, sorted by name.
    pub process_data: Vec<(String, f64)>,
    /// Prim origin in `base_link`, asset (Z-up) metres.
    pub offset_base_link: DVec3,
    /// DDI 157 connector type for connector elements.
    pub connector_type: Option<u32>,
}

/// DDI names the runtime knows; anything else is authored but only warned
/// about, since the full ISO 11783-11 dictionary is not shipped here.
pub const KNOWN_DDIS: [&str; 22] = [
    "SetpointVolumePerAreaApplicationRate",
    "ActualVolumePerAreaApplicationRate",
    "SetpointMassPerAreaApplicationRate",
    "ActualMassPerAreaApplicationRate",
    "MaximumVolumeContent",
    "ActualVolumeContent",
    "MaximumMassContent",
    "ActualMassContent",
    "ActualWorkingWidth",
    "MaximumWorkingWidth",
    "SetpointWorkState",
    "ActualWorkState",
    "SectionControlState",
    "TotalArea",
    "EffectiveTotalDistance",
    "DeviceElementOffsetX",
    "DeviceElementOffsetY",
    "DeviceElementOffsetZ",
    "ConnectorType",
    "ActualCulturalPractice",
    "PrescriptionControlState",
    "RequestDefaultProcessData",
];

pub fn ddi_number(name: &str) -> Option<u32> {
    Some(match name {
        "SetpointVolumePerAreaApplicationRate" => 1,
        "ActualVolumePerAreaApplicationRate" => 2,
        "SetpointMassPerAreaApplicationRate" => 6,
        "ActualMassPerAreaApplicationRate" => 7,
        "MaximumVolumeContent" => 71,
        "ActualVolumeContent" => 72,
        "MaximumMassContent" => 73,
        "ActualMassContent" => 74,
        "ActualWorkingWidth" => 67,
        "MaximumWorkingWidth" => 70,
        "SetpointWorkState" => 141,
        "ActualWorkState" => 141 + 1,
        "SectionControlState" => 160,
        "TotalArea" => 116,
        "EffectiveTotalDistance" => 117,
        "DeviceElementOffsetX" => 134,
        "DeviceElementOffsetY" => 135,
        "DeviceElementOffsetZ" => 136,
        "ConnectorType" => 157,
        "ActualCulturalPractice" => 179,
        "PrescriptionControlState" => 158,
        "RequestDefaultProcessData" => 0xDFFF,
        _ => return None,
    })
}

/// DDI 157 connector type per `TOOLS_SPEC.md` §2.1.
pub fn connector_type(coupling_kind: &str) -> u32 {
    match coupling_kind {
        "drawbar" => 1,
        "three_point_semi_mounted" => 2,
        "three_point_mounted" => 3,
        "hitch_hook" => 4,
        "clevis" => 5,
        "piton" => 6,
        "cuna" => 7,
        "ball" => 8,
        "chassis_mounted" => 9,
        "pivot_wagon" => 10,
        _ => 0,
    }
}

fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn read_number(stage: &openusd::Stage, prim: &SdfPath, name: &str) -> Option<f64> {
    match read_attr(stage, prim, name)? {
        Value::Float(v) => Some(v as f64),
        Value::Double(v) => Some(v),
        Value::Int(v) => Some(v as f64),
        Value::Uint(v) => Some(v as f64),
        Value::Int64(v) => Some(v as f64),
        Value::Bool(b) => Some(b as u8 as f64),
        _ => None,
    }
}

/// Every `gearbox:pd:*` attribute on a prim, by listing the prim's properties.
fn process_data_of(
    stage: &openusd::Stage,
    prim: &SdfPath,
    warnings: &mut Vec<String>,
) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let names = stage.prim_properties(prim.clone()).unwrap_or_default();
    for name in names {
        let Some(ddi) = name.strip_prefix("gearbox:pd:") else {
            continue;
        };
        if !KNOWN_DDIS.contains(&ddi) {
            warnings.push(format!(
                "{}: gearbox:pd:{ddi} is not a DDI this runtime knows; exported by name only",
                prim.as_str()
            ));
        }
        if let Some(v) = read_number(stage, prim, &name) {
            out.push((ddi.to_string(), v));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The offset of `prim` in the base link's frame, through the prim
/// hierarchy from the base link's prim.
fn offset_in_base(
    stage: &openusd::Stage,
    tree: &LinkTree,
    machine_root: &str,
    prim: &str,
) -> DVec3 {
    let Some(base) = tree.base() else {
        return DVec3::ZERO;
    };
    let base_prim = base.prim_path.as_str();
    if prim == base_prim {
        return DVec3::ZERO;
    }
    if prim.starts_with(&format!("{base_prim}/")) {
        return local_offset_between(stage, base_prim, prim).translation;
    }
    // Sibling subtree: go through the machine root both ways.
    let to_prim = if prim == machine_root {
        DVec3::ZERO
    } else {
        local_offset_between(stage, machine_root, prim).translation
    };
    let to_base = if base_prim == machine_root {
        DVec3::ZERO
    } else {
        local_offset_between(stage, machine_root, base_prim).translation
    };
    to_prim - to_base
}

/// Discover the element tree of the machine at `machine_prim`. Errors follow
/// `TOOLS_SPEC.md` §8; warnings cover unknown DDIs.
pub fn discover_elements(
    stage: &openusd::Stage,
    machine_prim: &SdfPath,
    machine_kind: Option<&str>,
    prims: &[SdfPath],
    tree: &LinkTree,
) -> (Vec<ElementSpec>, Vec<String>, Vec<String>) {
    let root = machine_prim.as_str();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let inside: Vec<&SdfPath> = prims
        .iter()
        .filter(|p| p.as_str() == root || p.as_str().starts_with(&format!("{root}/")))
        .collect();

    let mut authored: Vec<(String, ElementKind, u32, String, Vec<(String, f64)>)> = Vec::new();
    for prim in &inside {
        let schemas = stage.api_schemas(prim).unwrap_or_default();
        let marked = schemas.iter().any(|s| s == "GearboxElementAPI")
            || read_token(stage, prim, "gearbox:element:type").is_some();
        if !marked {
            continue;
        }
        let path = prim.as_str().to_string();
        let kind = match read_token(stage, prim, "gearbox:element:type").as_deref() {
            Some(t) => match ElementKind::parse(t) {
                Some(k) => k,
                None => {
                    errors.push(format!("{path}: unknown gearbox:element:type `{t}`"));
                    continue;
                }
            },
            None => {
                errors.push(format!("{path}: element has no gearbox:element:type"));
                continue;
            }
        };
        let number = match read_attr(stage, prim, "gearbox:element:number") {
            Some(Value::Int(n)) if n >= 0 => n as u32,
            Some(Value::Uint(n)) => n,
            Some(Value::Int(n)) => {
                errors.push(format!("{path}: gearbox:element:number {n} is negative"));
                continue;
            }
            _ if kind == ElementKind::Device => 0,
            _ => {
                errors.push(format!("{path}: element has no gearbox:element:number"));
                continue;
            }
        };
        let designator = match read_attr(stage, prim, "gearbox:element:designator") {
            Some(Value::String(s)) | Some(Value::Token(s)) => s,
            _ => leaf(&path).to_string(),
        };
        let pd = process_data_of(stage, prim, &mut warnings);
        authored.push((path, kind, number, designator, pd));
    }

    // The device: the machine prim, authored or derived.
    let device_authored = authored
        .iter()
        .filter(|(_, k, ..)| *k == ElementKind::Device)
        .count();
    if device_authored > 1 {
        errors.push(format!(
            "{device_authored} elements have type device; exactly one is allowed"
        ));
    }
    let mut elements: Vec<ElementSpec> = Vec::new();
    if device_authored == 0 {
        let mut pd_warnings = Vec::new();
        let pd = process_data_of(stage, machine_prim, &mut pd_warnings);
        elements.push(ElementSpec {
            prim_path: root.to_string(),
            kind: ElementKind::Device,
            number: 0,
            designator: machine_kind.unwrap_or_else(|| leaf(root)).to_string(),
            parent: None,
            process_data: pd,
            offset_base_link: DVec3::ZERO,
            connector_type: None,
        });
    }

    // Authored elements: parent is the nearest ancestor element prim, else
    // the device.
    let element_prims: Vec<(String, u32)> = authored
        .iter()
        .map(|(p, _, n, ..)| (p.clone(), *n))
        .collect();
    let device_number = authored
        .iter()
        .find(|(_, k, ..)| *k == ElementKind::Device)
        .map(|(_, _, n, ..)| *n)
        .unwrap_or(0);
    for (path, kind, number, designator, pd) in &authored {
        let parent = if *kind == ElementKind::Device {
            None
        } else {
            let mut cur = path.clone();
            let mut found = None;
            while let Some(pos) = cur.rfind('/') {
                cur.truncate(pos);
                if cur.is_empty() {
                    break;
                }
                if let Some((_, n)) = element_prims.iter().find(|(p, _)| *p == cur) {
                    found = Some(*n);
                    break;
                }
            }
            Some(found.unwrap_or(device_number))
        };
        // A link's element parent and link parent must agree (§7.3).
        if let Some(link) = tree.by_prim(path)
            && let (Some(parent_number), Some(link_parent)) = (parent, &link.parent)
            && let Some(parent_link) = tree.get(link_parent)
            && let Some((_, parent_elem_number)) = element_prims
                .iter()
                .find(|(p, _)| *p == parent_link.prim_path)
            && *parent_elem_number != parent_number
            && parent_number != device_number
        {
            errors.push(format!(
                "{path}: element parent {parent_number} differs from link parent {} (element {parent_elem_number})",
                link_parent
            ));
        }
        elements.push(ElementSpec {
            prim_path: path.clone(),
            kind: *kind,
            number: *number,
            designator: designator.clone(),
            parent,
            process_data: pd.clone(),
            offset_base_link: offset_in_base(stage, tree, root, path),
            connector_type: None,
        });
    }

    // Derived connectors: every coupler coupling.
    let mut next_number = elements.iter().map(|e| e.number).max().unwrap_or(0) + 1;
    for (link, coupling) in tree.couplings() {
        if coupling.side != CouplingSide::Coupler {
            continue;
        }
        if elements.iter().any(|e| e.prim_path == link.prim_path) {
            continue;
        }
        elements.push(ElementSpec {
            prim_path: link.prim_path.clone(),
            kind: ElementKind::Connector,
            number: next_number,
            designator: coupling.name.clone(),
            parent: Some(device_number),
            process_data: vec![(
                "ConnectorType".to_string(),
                connector_type(&coupling.kind) as f64,
            )],
            offset_base_link: offset_in_base(stage, tree, root, &link.prim_path),
            connector_type: Some(connector_type(&coupling.kind)),
        });
        next_number += 1;
    }

    // Derived navigation: a GNSS link.
    if let Some(nav) = tree
        .links
        .iter()
        .find(|l| l.name == "gps_link" || l.name == "gnss_link")
        && !elements.iter().any(|e| e.prim_path == nav.prim_path)
    {
        elements.push(ElementSpec {
            prim_path: nav.prim_path.clone(),
            kind: ElementKind::Navigation,
            number: next_number,
            designator: nav.name.clone(),
            parent: Some(device_number),
            process_data: Vec::new(),
            offset_base_link: offset_in_base(stage, tree, root, &nav.prim_path),
            connector_type: None,
        });
    }

    // Numbers unique, structure per ISO 11783-10.
    let mut seen: HashSet<u32> = HashSet::new();
    for e in &elements {
        if !seen.insert(e.number) {
            errors.push(format!(
                "{}: gearbox:element:number {} is used twice",
                e.prim_path, e.number
            ));
        }
    }
    let kind_of = |n: u32| elements.iter().find(|e| e.number == n).map(|e| e.kind);
    for e in &elements {
        let Some(parent) = e.parent else { continue };
        let parent_kind = kind_of(parent);
        let ok = match e.kind {
            ElementKind::Device => false,
            ElementKind::Connector | ElementKind::Navigation => {
                parent_kind == Some(ElementKind::Device)
            }
            ElementKind::Section | ElementKind::Unit => matches!(
                parent_kind,
                Some(ElementKind::Function) | Some(ElementKind::Device)
            ),
            ElementKind::Bin => matches!(
                parent_kind,
                Some(ElementKind::Function) | Some(ElementKind::Device)
            ),
            ElementKind::Function => parent_kind.is_some(),
        };
        if !ok {
            errors.push(format!(
                "{}: {} element cannot hang under {}",
                e.prim_path,
                e.kind.as_str(),
                parent_kind.map(|k| k.as_str()).unwrap_or("nothing")
            ));
        }
    }
    (elements, errors, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::discover_machines_from_usd;
    use std::path::Path;

    #[test]
    fn sprayer_has_a_ddop_tree() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/sprayer.usd");
        let machines = discover_machines_from_usd(&path).expect("scan");
        let m = &machines[0];
        assert!(m.links.is_valid(), "{:?}", m.links.errors);
        let kinds: Vec<(&str, u32)> = m
            .elements
            .iter()
            .map(|e| (e.kind.as_str(), e.number))
            .collect();
        assert!(kinds.contains(&("device", 0)), "{kinds:?}");
        assert!(kinds.contains(&("function", 2)), "{kinds:?}");
        assert!(kinds.contains(&("bin", 3)), "{kinds:?}");
        assert!(
            kinds.iter().filter(|(k, _)| *k == "section").count() >= 2,
            "{kinds:?}"
        );
        assert!(kinds.iter().any(|(k, _)| *k == "connector"), "{kinds:?}");
        let boom = m.elements.iter().find(|e| e.number == 2).unwrap();
        assert_eq!(boom.parent, Some(0));
        assert!(
            boom.process_data
                .iter()
                .any(|(n, v)| n == "ActualWorkingWidth" && *v > 5.0)
        );
        let section = m
            .elements
            .iter()
            .find(|e| e.kind == ElementKind::Section)
            .unwrap();
        assert_eq!(section.parent, Some(2));
        let connector = m
            .elements
            .iter()
            .find(|e| e.kind == ElementKind::Connector)
            .unwrap();
        assert_eq!(connector.connector_type, Some(3));
        assert!(connector.offset_base_link.y.abs() > 0.1);
    }
}
