use super::*;

#[test]
fn tyre_wrench_readout_matches_substep_impulses_and_clears_stale_loads() {
    for pressure in [false, true] {
        let (mut backend, chassis, wheel, joint, ground) = wheel_rig(-DVec3::Z);
        let guide = backend.joints().into_iter().find(|j| *j != joint).unwrap();
        let fixed = backend.bodies().into_iter().find(|b| *b != chassis && *b != wheel).unwrap();
        backend.remove_joint(guide);
        let direction = DVec3::new(1.0, 0.0, 0.3).normalize();
        backend.insert_joint(fixed, chassis, JointDesc::new(
            JointKind::Prismatic { axis: direction },
            Pose::from_translation(DVec3::Y * 0.49), Pose::IDENTITY,
        ));
        if pressure {
            backend.configure_wheel(WheelForceDesc {
                body: wheel, joint, local_hub: DVec3::ZERO, forward: DVec3::X,
                radius: 0.5, supported_mass: 100.0,
                tyre: Some(PressureTyreDesc::reference(0.3)),
            }).unwrap();
        }
        backend.body_mut(chassis).unwrap().set_linvel(direction * 2.0, true);
        backend.body_mut(wheel).unwrap().set_angvel(-DVec3::Z * 2.0, true);
        for _ in 0..3 {
            backend.step(&|_, _| false);
            let output = backend.wheel_output(wheel).unwrap();
            assert!(output.in_contact && !output.held_support);
            let world = backend.shared.world();
            let handle = backend.bodies[&wheel].handle;
            let samples: Vec<_> = world.wheels.samples(&world.scene).iter()
                .filter(|s| s.wheel == handle).collect();
            assert!(samples.len() > 1);
            let mut force = DVec3::ZERO;
            let mut aligning = DVec3::ZERO;
            let mut rolling = DVec3::ZERO;
            let mut normal_impulse = 0.0;
            let mut slip_ratio = 0.0;
            let mut slip_angle = 0.0;
            for sample in samples {
                force += DVec3::from_array(sample.output.force) * sample.dt;
                aligning += sample.normal * sample.output.aligning_moment * sample.dt;
                rolling += sample.pressure.as_ref().map_or(DVec3::ZERO, |p| p.rolling_moment) * sample.dt;
                let impulse = sample.output.fz * sample.dt;
                normal_impulse += impulse;
                slip_ratio += sample.output.slip_ratio * impulse;
                slip_angle += sample.output.slip_angle * impulse;
            }
            let dt = backend.wheel_step_dt;
            near(output.force, force / dt);
            near(output.aligning_moment, aligning / dt);
            assert!(output.force.x.abs() > 1e-8);
            assert!(output.force.z.abs() > 1e-8);
            assert!(output.aligning_moment.length() > 1e-8);
            assert!((output.normal_force - normal_impulse / dt).abs() < 1e-8);
            assert!((output.slip_ratio - slip_ratio / normal_impulse).abs() < 1e-8);
            assert!((output.slip_angle - slip_angle / normal_impulse).abs() < 1e-8);
            if pressure {
                assert!(rolling.length() > 0.0);
                near(output.pressure.unwrap().rolling_moment, rolling / dt);
            }
            drop(world);
            let mut settings = backend.settings();
            settings.dt *= 1.25;
            backend.set_settings(settings);
            near(backend.wheel_output(wheel).unwrap().force, output.force);
        }
        backend.collider_mut(ground).unwrap().set_enabled(false);
        for stepped in [false, true] {
            if stepped { backend.step(&|_, _| false); }
            let output = backend.wheel_output(wheel).unwrap();
            assert!(!output.in_contact && !output.held_support);
            near(output.force, DVec3::ZERO);
            near(output.aligning_moment, DVec3::ZERO);
            assert_eq!(output.normal_force, 0.0);
            assert_eq!(output.slip_ratio, 0.0);
            if let Some(tyre) = output.pressure { near(tyre.rolling_moment, DVec3::ZERO); }
        }
    }
}

#[test]
fn sleeping_tyre_wrench_is_marked_as_held_not_reintegrated() {
    let (mut backend, chassis, wheel, joint, _) = wheel_rig(-DVec3::Z);
    backend.configure_wheel(WheelForceDesc {
        body: wheel, joint, local_hub: DVec3::ZERO, forward: DVec3::X,
        radius: 0.5, supported_mass: 100.0,
        tyre: Some(PressureTyreDesc::reference(0.3)),
    }).unwrap();
    backend.step(&|_, _| false);
    backend.body_mut(chassis).unwrap().sleep();
    let pose = backend.body(wheel).unwrap().position();
    for _ in 0..3 {
        backend.step(&|_, _| false);
        let output = backend.wheel_output(wheel).unwrap();
        assert!(output.in_contact && output.held_support);
        assert!(output.normal_force > 0.0);
        let world = backend.shared.world();
        let handle = backend.bodies[&wheel].handle;
        let samples: Vec<_> = world.wheels.samples(&world.scene).iter()
            .filter(|s| s.wheel == handle).collect();
        assert!(!samples.is_empty());
        for sample in samples {
            assert_eq!(sample.dt, 0.0);
            near(output.force, DVec3::from_array(sample.output.force));
            near(output.aligning_moment, sample.normal * sample.output.aligning_moment);
            near(output.pressure.unwrap().rolling_moment, sample.pressure.as_ref().unwrap().rolling_moment);
        }
        drop(world);
        assert_eq!(backend.body(wheel).unwrap().position(), pose);
        assert!(backend.body(wheel).unwrap().is_sleeping());
    }
    backend.body_mut(chassis).unwrap().wake_up(true);
    backend.step(&|_, _| false);
    assert!(!backend.wheel_output(wheel).unwrap().held_support);
}
