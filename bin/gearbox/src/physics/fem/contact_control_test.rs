use super::track_contacts::FemTrackContacts;
use molla_solvers::fem_rigid_gpu::SoftRigidShapeGpu;

pub(super) fn frictionless_drive_contacts(
    contacts: &FemTrackContacts,
    drive_bodies: &[u32],
    teeth: bool,
) -> Vec<SoftRigidShapeGpu> {
    assert_eq!(contacts.shapes.len(), contacts.names.len());
    let mut output = Vec::new();
    let mut removed = vec![0; drive_bodies.len()];
    let mut supports = vec![0; drive_bodies.len()];
    for (shape, name) in contacts.shapes.iter().zip(&contacts.names) {
        let mut is_tooth = false;
        if let Some(index) = drive_bodies.iter().position(|&body| body == shape.ids[1]) {
            is_tooth = name
                .rsplit(':')
                .next()
                .unwrap()
                .starts_with("tooth_envelope_");
            if is_tooth {
                assert_eq!(shape.ids[0], 1);
                removed[index] += 1;
            } else {
                assert_eq!(
                    shape.ids[0], 2,
                    "non-cylindrical smooth sprocket support: {name}"
                );
                let q = bevy::prelude::Quat::from_array(shape.rotation);
                let axis = q * bevy::prelude::Vec3::Y;
                assert!((axis.x.abs() - 1.0).abs() < 1e-6);
                assert_eq!([shape.position[1], shape.position[2]], [0.0; 2]);
                supports[index] += 1;
            }
        }
        if teeth || !is_tooth {
            let mut shape = *shape;
            shape.data[3] = 0.0;
            output.push(shape);
        }
    }
    assert!(!drive_bodies.is_empty());
    assert!(removed.iter().all(|&count| count > 0));
    assert!(supports.iter().all(|&count| count > 0));
    output
}

#[test]
fn toothless_control_retains_smooth_and_passive_geometry() {
    let support = SoftRigidShapeGpu {
        position: [0.04, 0.0, 0.0, 0.0],
        rotation: [
            0.0,
            0.0,
            -std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        ],
        data: [0.094, 0.007, 0.0, 0.6],
        ids: [2, 7, 0, 0],
    };
    let tooth = SoftRigidShapeGpu {
        ids: [1, 7, 0, 0],
        ..support
    };
    let passive = SoftRigidShapeGpu {
        ids: [1, 8, 0, 0],
        ..tooth
    };
    let contacts = FemTrackContacts {
        shapes: vec![support, tooth, passive],
        names: vec![
            "drive:cover".into(),
            "drive:tooth_envelope_00".into(),
            "passive:tooth_envelope_00".into(),
        ],
    };
    let full = frictionless_drive_contacts(&contacts, &[7], true);
    let control = frictionless_drive_contacts(&contacts, &[7], false);
    assert_eq!(full.len(), 3);
    assert_eq!(control.len(), 2);
    for (actual, expected) in control.iter().zip([full[0], full[2]]) {
        assert_eq!(actual.position, expected.position);
        assert_eq!(actual.rotation, expected.rotation);
        assert_eq!(actual.data, expected.data);
        assert_eq!(actual.ids, expected.ids);
        assert_eq!(actual.data[3], 0.0);
    }
    assert_eq!(contacts.shapes[0].data[3], 0.6);
}

#[test]
#[ignore = "requires GEARBOX_TRACK_ASSET through oslo make test-fem-machine"]
fn ceol_fem_machine_binding_toothless_contact_control() {
    let (app, spec, layout) = super::machine_gpu_test::asset();
    let rigid = super::rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
    let contacts = FemTrackContacts::prepare(app.world(), &spec, &layout, &rigid).unwrap();
    let bodies = layout
        .tracks
        .iter()
        .map(|t| rigid.bodies[&t.sprocket].index() as u32)
        .collect::<Vec<_>>();
    let full = frictionless_drive_contacts(&contacts, &bodies, true);
    let control = frictionless_drive_contacts(&contacts, &bodies, false);
    let teeth = spec
        .tracks
        .iter()
        .map(|t| t.fem.as_ref().unwrap().sprocket_teeth)
        .sum::<usize>();
    assert_eq!(full.len() - control.len(), teeth);
    for body in bodies {
        assert_eq!(control.iter().filter(|s| s.ids[1] == body).count(), 6);
    }
    eprintln!(
        "CEOL toothless control: {} shapes retained, {teeth} tooth boxes removed, all smooth supports retained",
        control.len()
    );
}
