//! ISO 11783-10 device description (DDOP) as ISOXML `DVC` markup, built from
//! a machine's element records (`TOOLS_SPEC.md` §7.5).

use gearbox_api::{ElementRecord, MachineInfo};

/// DDI numbers for the names the sim knows; unknown names keep their name in
/// the designator and get DDI 0xDFFF (request default process data).
fn ddi_number(name: &str) -> u32 {
    match name {
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
        "ActualWorkState" => 142,
        "SectionControlState" => 160,
        "TotalArea" => 116,
        "EffectiveTotalDistance" => 117,
        "DeviceElementOffsetX" => 134,
        "DeviceElementOffsetY" => 135,
        "DeviceElementOffsetZ" => 136,
        "ConnectorType" => 157,
        "ActualCulturalPractice" => 179,
        "PrescriptionControlState" => 158,
        _ => 0xDFFF,
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// ISO frame: x forward, y right, z down, millimetres, from a REP-103
/// `base_link` (x forward, y left, z up).
fn iso_mm(x: f64, y: f64, z: f64) -> (i64, i64, i64) {
    (
        (x * 1000.0).round() as i64,
        (-y * 1000.0).round() as i64,
        (-z * 1000.0).round() as i64,
    )
}

/// One `<DVC>` with its `<DET>`, `<DPD>` and `<DPT>` children.
pub fn render(namespace: &str, info: &MachineInfo, elements: &[ElementRecord]) -> String {
    let props = info.props();
    let kind = props.get("kind").unwrap_or_else(|| "machine".into());
    let version = props
        .get("interface_version")
        .unwrap_or_else(|| "0.1".into());
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<ISO11783_TaskData VersionMajor=\"4\" VersionMinor=\"3\" ManagementSoftwareManufacturer=\"gearbox\" ManagementSoftwareVersion=\"{}\" DataTransferOrigin=\"1\">\n",
        xml_escape(env!("CARGO_PKG_VERSION"))
    ));
    out.push_str(&format!(
        "  <DVC A=\"DVC-1\" B=\"{}\" C=\"{}\" D=\"gearbox\" E=\"{}\" F=\"{}\">\n",
        xml_escape(&format!("{kind} {namespace}")),
        xml_escape(&version),
        xml_escape(&props.get("did").unwrap_or_default()),
        xml_escape(namespace),
    ));

    // Object ids: elements first, then one per process data and property.
    let mut next_object = 1u32;
    let mut element_object: Vec<(u32, u32)> = Vec::new();
    for e in elements {
        element_object.push((e.number, next_object));
        next_object += 1;
    }
    let object_of = |number: u32| {
        element_object
            .iter()
            .find(|(n, _)| *n == number)
            .map(|(_, o)| *o)
            .unwrap_or(0)
    };

    let mut pd_lines = Vec::new();
    for e in elements {
        let (ox, oy, oz) = iso_mm(e.x, e.y, e.z);
        let parent = if e.parent == ElementRecord::NO_PARENT {
            0
        } else {
            object_of(e.parent)
        };
        out.push_str(&format!(
            "    <DET A=\"DET-{}\" B=\"{}\" C=\"{}\" D=\"{}\" E=\"{}\" F=\"{}\">\n",
            e.number,
            object_of(e.number),
            e.iso_type,
            xml_escape(&e.designator()),
            e.number,
            parent,
        ));
        // Offsets and connector type ride as properties on the element.
        let mut refs: Vec<(String, f64, &str)> = vec![
            ("DeviceElementOffsetX".into(), ox as f64, "mm"),
            ("DeviceElementOffsetY".into(), oy as f64, "mm"),
            ("DeviceElementOffsetZ".into(), oz as f64, "mm"),
        ];
        for (ddi, value) in e.process_data() {
            refs.push((ddi, value, ""));
        }
        for (ddi, value, _unit) in refs {
            let object = next_object;
            next_object += 1;
            out.push_str(&format!("      <DOR A=\"{object}\"/>\n"));
            let settable = ddi.starts_with("Setpoint") || ddi == "SectionControlState";
            let scaled = if ddi.starts_with("DeviceElementOffset") {
                value as i64
            } else {
                (value * 1000.0).round() as i64
            };
            if settable {
                pd_lines.push(format!(
                    "    <DPD A=\"{object}\" B=\"{:04X}\" C=\"3\" D=\"{}\" E=\"{}\"/>",
                    ddi_number(&ddi),
                    if ddi == "SectionControlState" { 1 } else { 3 },
                    xml_escape(&ddi),
                ));
            } else {
                pd_lines.push(format!(
                    "    <DPT A=\"{object}\" B=\"{:04X}\" C=\"{scaled}\" D=\"{}\"/>",
                    ddi_number(&ddi),
                    xml_escape(&ddi),
                ));
            }
        }
        out.push_str("    </DET>\n");
    }
    for line in pd_lines {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("  </DVC>\n</ISO11783_TaskData>\n");
    out
}
