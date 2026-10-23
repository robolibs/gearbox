//! Structural fixtures S01–S22 of plan 001. Each case compiles a USDA file
//! from `tests/fixtures/robot_contract/` and checks the resolved model.

use bevy::math::{DQuat, DVec3};

use super::*;
use super::diagnostics::Code;

pub(crate) fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/robot_contract").join(name)
}

pub(crate) fn compile_fixture(name: &str, selection: Selection) -> CompileResult {
    let options = CompileOptions { selection, ..Default::default() };
    compile_file(&fixture(name), &options).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn world(name: &str) -> CompileResult {
    compile_fixture(name, Selection::World)
}

fn machine<'a>(r: &'a CompileResult, root: &str) -> &'a MachineDescription {
    r.machine(root).unwrap_or_else(|| {
        panic!("no machine at {root}; have {:?}", r.machines.iter().map(|m| &m.root).collect::<Vec<_>>())
    })
}

fn has(m: &MachineDescription, code: Code, path_part: &str) -> bool {
    m.diagnostics.iter().any(|d| d.code == code && d.path.as_deref().unwrap_or("").contains(path_part))
}

fn error_codes(m: &MachineDescription) -> Vec<&'static str> {
    m.errors().map(|d| d.code.as_str()).collect()
}

fn joint<'a>(m: &'a MachineDescription, path: &str) -> &'a JointDescription {
    &m.joints[m.joint_index(path).unwrap_or_else(|| panic!("no joint {path}"))]
}

fn dof_of<'a>(m: &'a MachineDescription, joint: &str) -> &'a DofDescription {
    &m.dofs[m.dof(joint, None).unwrap_or_else(|| panic!("no scalar DOF on {joint}")).dof]
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * (1.0 + a.abs().max(b.abs()))
}

fn close_v(a: DVec3, b: DVec3) -> bool {
    (a - b).length() < 1e-6
}

fn tree_parent(m: &MachineDescription, child: &str) -> Option<String> {
    let c = m.frames.iter().position(|f| f.path == child)?;
    m.tree.iter().find(|e| e.child == c).map(|e| m.frames[e.parent].path.clone())
}

fn frame_world(m: &MachineDescription, path: &str) -> RigidTransform {
    let index = m.frame_index(path).unwrap_or_else(|| panic!("no frame {path}"));
    let (body, pose) = m.frame_in_body(index).expect("frame rides a body");
    m.bodies[body].initial_pose.mul(&pose)
}

pub(crate) fn s01() {
    let r = world("s01_floating_fixed_branched.usda");
    assert_eq!(r.machines.len(), 3, "{:?}", r.machines.iter().map(|m| &m.root).collect::<Vec<_>>());
    let floater = machine(&r, "/World/Floater");
    assert!(floater.accepted(), "{:?}", floater.diagnostics);
    assert_eq!((floater.bodies.len(), floater.dofs.len()), (1, 0));
    assert_eq!(floater.control_body, Some(0));
    assert!(floater.controllers.is_empty());

    let arm = machine(&r, "/World/Arm");
    assert!(arm.accepted(), "{:?}", arm.diagnostics);
    assert_eq!((arm.bodies.len(), arm.dofs.len()), (3, 2));
    assert_eq!(joint(arm, "/World/Arm/anchor").class, JointClass::WorldAnchor);
    assert_eq!(arm.articulations.len(), 1);
    assert!(arm.articulations[0].fixed_base);
    assert_eq!(arm.bodies[arm.control_body.unwrap()].path, "/World/Arm/base");
    assert_eq!(tree_parent(arm, "/World/Arm/link2").as_deref(), Some("/World/Arm/link1"));
    let shoulder = dof_of(arm, "/World/Arm/shoulder");
    assert!(close(shoulder.limit.lower.unwrap(), -std::f64::consts::FRAC_PI_2, 1e-9));
    assert!(dof_of(arm, "/World/Arm/elbow").limit.lower.is_none());

    let branch = machine(&r, "/World/Branch");
    assert!(branch.accepted(), "{:?}", branch.diagnostics);
    assert_eq!((branch.bodies.len(), branch.dofs.len()), (4, 3));
    assert_eq!(branch.bodies[branch.control_body.unwrap()].path, "/World/Branch/torso");
    assert!(!branch.articulations[0].fixed_base);
    assert_eq!(tree_parent(branch, "/World/Branch/finger").as_deref(), Some("/World/Branch/left"));
    assert_eq!(tree_parent(branch, "/World/Branch/right").as_deref(), Some("/World/Branch/torso"));
}

pub(crate) fn s02() {
    let r = world("s02_articulation_markers.usda");
    assert_eq!(r.machines.len(), 3, "{:?}", r.machines.iter().map(|m| &m.root).collect::<Vec<_>>());
    for (root, base) in [("/World/A", "/World/A/base"), ("/World/B", "/World/B/base"), ("/World/C", "/World/C/base")] {
        let m = r.machines.iter().find(|m| m.root.starts_with(root)).unwrap_or_else(|| panic!("no machine for {root}"));
        assert!(m.accepted(), "{root}: {:?}", m.diagnostics);
        assert_eq!(m.bodies.len(), 2, "{root}");
        let hinge = m.dof(&format!("{root}/hinge"), None).expect("hinge DOF");
        assert_eq!(m.dofs[hinge.dof].token, DofToken::Angular);
        assert_eq!(m.bodies[m.control_body.unwrap()].path, base, "{root}");
        assert_eq!(m.articulations.len(), 1, "{root}");
        assert_eq!(m.articulations[0].markers.len(), 1, "{root}");
    }
    let c = r.machines.iter().find(|m| m.root.starts_with("/World/C")).unwrap();
    assert!(c.articulations[0].fixed_base);
}

pub(crate) fn s03() {
    let r = world("s03_world_endpoints.usda");
    let p0 = machine(&r, "/World/P0");
    assert!(p0.accepted(), "{:?}", p0.diagnostics);
    let pivot = joint(p0, "/World/P0/pivot");
    assert_eq!(pivot.class, JointClass::WorldAnchor);
    assert_eq!(pivot.body0.endpoint, Endpoint::World);
    assert_eq!(pivot.dofs.len(), 1, "world revolute keeps its DOF");
    assert!(close_v(pivot.local0.translation, DVec3::new(0.0, 0.0, 2.0)));
    assert!(close(dof_of(p0, "/World/P0/pivot").initial_position.value, 0.0, 1e-9));

    let p1 = machine(&r, "/World/P1");
    let rail = joint(p1, "/World/P1/rail");
    assert_eq!(rail.class, JointClass::WorldAnchor);
    assert_eq!(rail.body1.endpoint, Endpoint::World);
    let q = dof_of(p1, "/World/P1/rail");
    assert_eq!(q.token, DofToken::Linear);
    assert!(close(q.initial_position.value, -0.5, 1e-9), "{}", q.initial_position.value);
    assert_eq!(q.initial_position.source, Provenance::Derived);
}

pub(crate) fn s04() {
    let r = world("s04_invalid_targets.usda");
    let m = machine(&r, "/World/Bad");
    assert!(!m.accepted());
    let typo = joint(m, "/World/Bad/typo");
    let inactive = joint(m, "/World/Bad/inactive");
    assert_eq!(typo.class, JointClass::Invalid);
    assert_eq!(inactive.class, JointClass::Invalid);
    match (&typo.body1.endpoint, &inactive.body1.endpoint) {
        (Endpoint::Invalid { reason: a, .. }, Endpoint::Invalid { reason: b, .. }) => {
            assert!(a.contains("does not exist"), "{a}");
            assert!(b.contains("inactive"), "{b}");
        }
        other => panic!("expected invalid endpoints, got {other:?}"),
    }
    assert!(has(m, Code::InvalidEndpoint, "/World/Bad/typo"));
    assert!(has(m, Code::InvalidEndpoint, "/World/Bad/inactive"));
    assert!(!typo.dofs.is_empty() && typo.body0.endpoint == Endpoint::Body(m.body_index("/World/Bad/base").unwrap()));
    let post = joint(m, "/World/Bad/to_post");
    assert_eq!(post.body0.endpoint, Endpoint::World);
    assert_eq!(post.body0.via.as_deref(), Some("static:/World/Bad/post"));
    assert!(close_v(post.local0.translation, DVec3::ZERO));
    assert_eq!(post.class, JointClass::WorldAnchor);
    assert_eq!(joint(m, "/World/Bad/good").class, JointClass::Tree);
}

pub(crate) fn s05() {
    let r = world("s05_reversed_endpoints.usda");
    let fwd = machine(&r, "/World/Fwd");
    let rev = r.machines.iter().find(|m| m.root.starts_with("/World/Rev")).expect("rev machine");
    let q_fwd = dof_of(fwd, "/World/Fwd/hinge").initial_position.value;
    let q_rev = dof_of(rev, "/World/Rev/hinge").initial_position.value;
    assert!(close(q_fwd, 30f64.to_radians(), 1e-9), "{q_fwd}");
    assert!(close(q_rev, -30f64.to_radians(), 1e-9), "{q_rev}");
    let edge = rev.tree.iter().find(|e| e.joint.is_some()).expect("joint edge");
    assert!(edge.reversed);
    assert_eq!(rev.frames[edge.parent].path, "/World/Rev/base");
    let edge = fwd.tree.iter().find(|e| e.joint.is_some()).expect("joint edge");
    assert!(!edge.reversed);
}

pub(crate) fn s06() {
    let r = world("s06_loops.usda");
    let bar = machine(&r, "/World/FourBar");
    assert!(bar.accepted(), "{:?}", bar.diagnostics);
    assert_eq!(joint(bar, "/World/FourBar/a_closure").class, JointClass::Closure);
    assert!(!bar.tree.iter().any(|e| e.joint == bar.joint_index("/World/FourBar/a_closure")));
    assert_eq!(tree_parent(bar, "/World/FourBar/rocker").as_deref(), Some("/World/FourBar/coupler"));
    assert_eq!(tree_parent(bar, "/World/FourBar/crank").as_deref(), Some("/World/FourBar/ground"));
    assert_eq!(bar.articulations.len(), 1);

    let loop_ = machine(&r, "/World/Unexcluded");
    assert!(!loop_.accepted());
    assert!(has(loop_, Code::UnclassifiedCycle, "/World/Unexcluded/ca"));
}

pub(crate) fn s07() {
    let r = world("s07_disabled.usda");
    let m = machine(&r, "/World/Chain");
    assert_eq!(joint(m, "/World/Chain/j2").class, JointClass::Disabled);
    assert_eq!(joint(m, "/World/Chain/j3_closure").class, JointClass::Disabled);
    assert_eq!(joint(m, "/World/Chain/j1").class, JointClass::Tree);
    assert!(tree_parent(m, "/World/Chain/link2").is_none());
    assert!(has(m, Code::DisconnectedBody, "/World/Chain/link2"));
    assert!(!m.articulations.iter().any(|a| a.bodies.contains(&m.body_index("/World/Chain/link2").unwrap()) && a.bodies.len() > 1));
    assert_eq!(dof_of(m, "/World/Chain/j2").token, DofToken::Angular, "disabled joints stay described");
}

pub(crate) fn s08() {
    let r = compile_fixture("s08_base_frame.usda", Selection::Asset { entry: None });
    let m = machine(&r, "/rover");
    assert!(m.accepted(), "{:?}", m.diagnostics);
    assert_eq!(m.profile, ProfileKind::GearboxV2);
    let base = m.base_frame.unwrap();
    assert_eq!(m.frames[base].path, "/rover/chassis/base_footprint");
    assert_eq!(m.frames[base].role, FrameRole::Base);
    assert!(close_v(frame_world(m, "/rover/chassis/base_footprint").translation, DVec3::ZERO));
    let root_edge = &m.tree[0];
    assert_eq!((root_edge.parent, m.frames[root_edge.child].path.as_str()), (base, "/rover/chassis"));
    let control = m.control_frame.as_ref().unwrap();
    assert_eq!(control.source, Provenance::Authored);
    let forward = frame_world(m, "/rover/chassis/control").rotation * DVec3::X;
    assert!(close_v(forward, DVec3::Y), "{forward:?}");
}

pub(crate) fn s09() {
    let r = compile_fixture("s09_sites.usda", Selection::Asset { entry: None });
    let m = machine(&r, "/weeder");
    assert!(m.accepted(), "{:?}", m.diagnostics);
    let tool = m.frame_index("/weeder/arm/tool_tip").unwrap();
    let ball = m.frame_index("/weeder/probe/ball_tip").unwrap();
    assert_ne!(tool, ball);
    assert_eq!(m.frames[tool].owner, FrameOwner::Body(m.body_index("/weeder/arm").unwrap()));
    assert_eq!(m.frames[ball].owner, FrameOwner::Body(m.body_index("/weeder/probe").unwrap()));
    assert!(close_v(m.frames[tool].transform.translation, DVec3::new(0.3, 0.0, -0.2)));
    assert!(close_v(m.frames[ball].transform.translation, DVec3::new(0.0, 0.0, -0.15)));
    assert!(close_v(frame_world(m, "/weeder/arm/tool_tip").translation, DVec3::new(1.3, 0.0, 0.4)));
    assert!(close_v(frame_world(m, "/weeder/probe/ball_tip").translation, DVec3::new(1.3, 0.0, 0.05)));
    let camera = m.frame_index("/weeder/chassis/camera_link").unwrap();
    assert_eq!(m.frames[camera].role, FrameRole::Sensor);
    assert!(m.frames[camera].transform.rotation.angle_between(DQuat::from_rotation_y(20f64.to_radians())) < 1e-6);
    let hitch = m.frame_index("/weeder/chassis/rear_hitch").unwrap();
    assert_eq!(m.frames[hitch].role, FrameRole::Coupling);
    assert_eq!(tree_parent(m, "/weeder/probe/ball_tip").as_deref(), Some("/weeder/probe"));
    assert_eq!(tree_parent(m, "/weeder/probe").as_deref(), Some("/weeder/arm"));
    assert!(m.bodies.len() == 3, "no dummy bodies for frames");
}

pub(crate) fn s10() {
    let z = world("s10_axes_zup.usda");
    let y = world("s10_axes_yup.usda");
    let (mz, my) = (machine(&z, "/World/Robot"), machine(&y, "/World/Robot"));
    assert!(mz.accepted() && my.accepted(), "{:?} {:?}", mz.diagnostics, my.diagnostics);
    for path in ["/World/Robot/base", "/World/Robot/link"] {
        let (bz, by) = (&mz.bodies[mz.body_index(path).unwrap()], &my.bodies[my.body_index(path).unwrap()]);
        assert!(close_v(bz.initial_pose.translation, by.initial_pose.translation), "{path}: {bz:?} {by:?}");
        assert!(close_v(bz.initial_pose.translation, if path.ends_with("base") { DVec3::new(1.0, 2.0, 3.0) } else { DVec3::new(1.0, 2.0, 4.0) }));
    }
    let (jz, jy) = (joint(mz, "/World/Robot/yaw"), joint(my, "/World/Robot/yaw"));
    let axis_world = |m: &MachineDescription, j: &JointDescription| {
        let f = compile::frame_world(m, &j.body0.endpoint, &j.local0).unwrap();
        (f.translation, f.rotation * j.axis.unwrap().vector())
    };
    let ((pz, az), (py, ay)) = (axis_world(mz, jz), axis_world(my, jy));
    assert!(close_v(pz, py) && close_v(az, ay) && close_v(az, DVec3::Z), "{pz:?} {py:?} {az:?} {ay:?}");
    let (qz, qy) = (dof_of(mz, "/World/Robot/yaw").initial_position.value, dof_of(my, "/World/Robot/yaw").initial_position.value);
    assert!(close(qz, qy, 1e-9) && close(qz, 45f64.to_radians(), 1e-9), "{qz} {qy}");

    let c = compile_fixture("s10_control_frame.usda", Selection::Asset { entry: None });
    let cart = machine(&c, "/cart");
    assert!(cart.accepted(), "{:?}", cart.diagnostics);
    let control = cart.control_frame.as_ref().unwrap();
    let forward = frame_world(cart, &cart.frames[control.frame].path.clone()).rotation * DVec3::X;
    let expected = DQuat::from_rotation_z(120f64.to_radians()) * DVec3::X;
    assert!(close_v(forward, expected), "{forward:?}");
}

pub(crate) fn s11() {
    let r = compile_fixture("s11_composed_transforms.usda", Selection::Asset { entry: None });
    let m = machine(&r, "/rig");
    assert!(m.accepted(), "{:?}", m.diagnostics);
    let pivoted = &m.frames[m.frame_index("/rig/base/pivoted").unwrap()];
    assert!(close_v(pivoted.transform.translation, DVec3::new(1.5, -0.5, 0.0)), "{:?}", pivoted.transform);
    assert!(pivoted.transform.rotation.angle_between(DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2)) < 1e-9);
    let reset = &m.frames[m.frame_index("/rig/base/offset/reset").unwrap()];
    assert!(close_v(reset.transform.translation, DVec3::new(0.0, 0.0, 1.0)), "{:?}", reset.transform);
    let matrix = &m.frames[m.frame_index("/rig/base/matrix").unwrap()];
    assert!(close_v(matrix.transform.translation, DVec3::new(3.0, 4.0, 5.0)));
    assert!(matrix.transform.rotation.angle_between(DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2)) < 1e-9);
    let tip = &m.frames[m.frame_index("/rig/sites/tip").unwrap()];
    assert_eq!(tip.owner, FrameOwner::Body(m.body_index("/rig/arm").unwrap()));
    assert!(close_v(tip.transform.translation, DVec3::new(0.5, 0.0, 0.0)), "{:?}", tip.transform);
}

pub(crate) fn s12() {
    let r = world("s12_physical_scale.usda");
    let uniform = machine(&r, "/World/Uniform");
    assert!(uniform.accepted(), "{:?}", uniform.diagnostics);
    let big = &uniform.bodies[uniform.body_index("/World/Uniform/big").unwrap()];
    assert!(close(big.scale, 2.0, 1e-12));
    assert!(close_v(big.mass.center_of_mass.value, DVec3::new(0.2, 0.0, 0.0)));
    let hinge = joint(uniform, "/World/Uniform/hinge");
    assert!(close_v(hinge.local1.translation, DVec3::new(-0.5, 0.0, 0.0)), "{:?}", hinge.local1);
    assert!(close(dof_of(uniform, "/World/Uniform/hinge").initial_position.value, 0.0, 1e-9));
    assert!(uniform.capabilities.iter().any(|c| c.feature == "body.uniform_scale"));
    for (root, code) in [("/World/Stretched", Code::NonUniformScale), ("/World/Sheared", Code::ShearTransform), ("/World/Mirrored", Code::ReflectionTransform)] {
        let m = r.machines.iter().find(|m| m.root.starts_with(root)).unwrap_or_else(|| panic!("{root}"));
        assert!(!m.accepted(), "{root}");
        assert!(has(m, code, root), "{root}: {:?}", m.diagnostics);
    }
}

pub(crate) fn s13() {
    let a = world("s13_units_m_kg.usda");
    let b = world("s13_units_cm_g.usda");
    let (ma, mb) = (machine(&a, "/World/Robot"), machine(&b, "/World/Robot"));
    assert!(ma.accepted() && mb.accepted(), "{:?} {:?}", ma.diagnostics, mb.diagnostics);
    let invalid = world("s13_units_invalid.usda");
    let cart = machine(&invalid, "/World/Cart");
    assert!(!cart.accepted() && cart.diagnostics.iter().filter(|d| d.code == Code::InvalidUnits).count() == 2, "{:?}", cart.diagnostics);
    let tol = 2e-6;
    for (ba, bb) in ma.bodies.iter().zip(&mb.bodies) {
        assert!(close(ba.mass.mass.as_ref().unwrap().value, bb.mass.mass.as_ref().unwrap().value, tol), "{}", ba.path);
        let (ia, ib) = (ba.mass.diagonal_inertia.as_ref().unwrap().value, bb.mass.diagonal_inertia.as_ref().unwrap().value);
        assert!((ia - ib).abs().max_element() < tol * ia.max_element(), "{ia:?} {ib:?}");
        assert!(close_v(ba.mass.center_of_mass.value, bb.mass.center_of_mass.value));
        assert!(close_v(ba.initial_pose.translation, bb.initial_pose.translation));
        assert!(close_v(ba.initial_linear_velocity.value, bb.initial_linear_velocity.value));
        assert!(close_v(ba.initial_angular_velocity.value, bb.initial_angular_velocity.value));
    }
    let base = &ma.bodies[ma.body_index("/World/Robot/base").unwrap()];
    assert!(close(base.mass.mass.as_ref().unwrap().value, 2.0, tol));
    assert!(close_v(base.initial_linear_velocity.value, DVec3::X));
    assert!(close_v(base.initial_angular_velocity.value, DVec3::new(0.0, 0.0, std::f64::consts::FRAC_PI_2)));
    for (path, k, d, cap, target, tv, lo) in [
        ("/World/Robot/hinge", 100.0, 10.0, 50.0, 30f64.to_radians(), 9f64.to_radians(), -45f64.to_radians()),
        ("/World/Robot/slide", 1000.0, 100.0, 200.0, 0.25, 0.1, -0.5),
    ] {
        for m in [ma, mb] {
            let dof = dof_of(m, path);
            let drive = dof.drive.as_ref().unwrap();
            assert!(close(drive.stiffness.value, k, tol), "{path} k {}", drive.stiffness.value);
            assert!(close(drive.damping.value, d, tol), "{path} d {}", drive.damping.value);
            assert!(close(drive.max_force.value.unwrap(), cap, tol), "{path} cap {:?}", drive.max_force.value);
            assert!(close(drive.target_position.value, target, tol), "{path} target {}", drive.target_position.value);
            assert!(close(drive.target_velocity.value, tv, tol), "{path} tv {}", drive.target_velocity.value);
            assert!(close(dof.limit.lower.unwrap(), lo, tol), "{path} lo {:?}", dof.limit.lower);
        }
        assert!(close_v(joint(ma, path).local0.translation, joint(mb, path).local0.translation));
    }
}

pub(crate) fn s14() {
    let r = compile_fixture("s14_world.usda", Selection::Asset { entry: None });
    assert_eq!(r.machines.len(), 2, "{:?}", r.machines.iter().map(|m| &m.root).collect::<Vec<_>>());
    let (r1, r2) = (machine(&r, "/World/R1"), machine(&r, "/World/R2"));
    assert!(r1.accepted() && r2.accepted(), "{:?} {:?}", r1.diagnostics, r2.diagnostics);
    assert_eq!(r1.machine_id, "world_r1");
    assert_eq!(r2.machine_id, "second", "sublayer opinion");
    for (m, root) in [(r1, "/World/R1"), (r2, "/World/R2")] {
        let boom = m.controllers.iter().find(|c| c.instance == "boom").expect("boom controller");
        let target = &boom.roles["target"];
        assert_eq!(target.len(), 1);
        assert_eq!(m.joints[target[0].joint].path, format!("{root}/Joints/lift"));
        assert_eq!(m.owners[&target[0].dof].owner, "boom");
        assert!(m.bodies.iter().all(|b| b.path.starts_with(root)));
    }
    assert!(close_v(r2.bodies[r2.body_index("/World/R2/boom").unwrap()].initial_pose.translation, DVec3::new(1.0, 4.0, 0.0)));
}

pub(crate) fn s15() {
    let r = world("s15_variants.usda");
    let m = machine(&r, "/World/Robot");
    assert!(m.joint_index("/World/Robot/j2").is_some(), "selected variant `long` adds j2");
    assert!(m.body_index("/World/Robot/link2").is_some());
    assert!(m.body_index("/World/Robot/hanger/weight").is_some(), "loaded payload body");
    assert_eq!(joint(m, "/World/Robot/hang").class, JointClass::Tree);
    assert!(m.accepted(), "{:?}", m.diagnostics);

    let i = compile_fixture("s15_instance.usda", Selection::Asset { entry: None });
    let instance_reported = i.diagnostics.iter().any(|d| d.code == Code::InstanceProxy)
        || i.machines.iter().any(|m| m.diagnostics.iter().any(|d| d.code == Code::InstanceProxy || d.code == Code::UnsupportedCapability));
    assert!(instance_reported, "instanceable prim must be reported: {:?} {:?}", i.diagnostics, i.machines.iter().map(|m| &m.diagnostics).collect::<Vec<_>>());
    assert!(i.machines.iter().all(|m| !m.accepted() || m.bodies.is_empty()), "no silently partial robot");
}

pub(crate) fn s16() {
    let r = world("s16_joint_types.usda");
    let zoo = machine(&r, "/World/Zoo");
    assert!(zoo.accepted(), "{:?}", zoo.diagnostics);
    let features: Vec<&str> = zoo.capabilities.iter().map(|c| c.feature.as_str()).collect();
    for f in ["joint.revolute", "joint.prismatic", "joint.fixed", "joint.spherical", "joint.distance", "joint.d6", "command.velocity", "command.effort"] {
        assert!(features.contains(&f), "{f} missing from {features:?}");
    }
    let six = joint(zoo, "/World/Zoo/j_six");
    let tokens: Vec<DofToken> = six.dofs.iter().map(|&d| zoo.dofs[d].token).collect();
    assert_eq!(tokens, vec![DofToken::RotY, DofToken::RotZ]);
    let rot_y = &zoo.dofs[six.dofs[0]];
    assert!(close(rot_y.limit.upper.unwrap(), 20f64.to_radians(), 1e-9));
    assert_eq!(zoo.dofs[six.dofs[1]].interfaces.value, vec![Interface::Velocity, Interface::Effort]);
    assert!(joint(zoo, "/World/Zoo/j_ball").dofs.is_empty());
    let rope = dof_of(zoo, "/World/Zoo/j_rope");
    assert_eq!((rope.limit.lower, rope.limit.upper), (Some(0.5), Some(1.5)));

    let bad = machine(&r, "/World/BadCmd");
    assert!(!bad.accepted());
    assert!(has(bad, Code::InvalidInterface, "/World/BadCmd/rope"));
    assert!(has(bad, Code::InvalidInterface, "/World/BadCmd/undriven"));
    assert!(has(bad, Code::InvalidInterface, "/World/BadCmd/ball"), "interfaces on a joint with no DOF");
    assert!(has(bad, Code::InvalidInterface, "/World/BadCmd/moded"), "a default mode needs its interface");
}

pub(crate) fn s17() {
    let r = world("s17_duplicates_and_nesting.usda");
    let dup = machine(&r, "/World/dup");
    assert!(!dup.accepted());
    assert!(has(dup, Code::DuplicateName, "/World/dup/right/hinge"));
    assert!(has(dup, Code::DuplicateName, "/World/dup/a/tip2"));
    let outer = machine(&r, "/World/outer");
    let inner = machine(&r, "/World/outer/inner");
    assert!(!outer.accepted() && !inner.accepted());
    assert!(has(outer, Code::NestedMachineRoot, "/World/outer") && has(inner, Code::NestedMachineRoot, "/World/outer/inner"));
}

pub(crate) fn s18() {
    let r = compile_fixture("s18_isaac_metadata.usda", Selection::Asset { entry: None });
    let m = machine(&r, "/bot");
    assert!(m.accepted(), "{:?}", m.diagnostics);
    assert!(m.isaac);
    assert_eq!(m.bodies.len(), 4, "unlisted links stay in the mechanics");
    let order = m.reporting.as_ref().expect("reporting order");
    assert_eq!(order.bodies.iter().map(|&i| m.bodies[i].path.as_str()).collect::<Vec<_>>(), vec!["/bot/base", "/bot/link1"]);
    assert_eq!(order.joints.iter().map(|&i| m.joints[i].path.as_str()).collect::<Vec<_>>(), vec!["/bot/joints/j2", "/bot/joints/j1"]);
    assert_eq!(m.bodies[m.body_index("/bot/base").unwrap()].name, "chassis_link");
    assert_eq!(joint(m, "/bot/joints/j1").name, "shoulder");
    assert_eq!(dof_of(m, "/bot/joints/j1").isaac_actuator, Some(true));
    assert!(dof_of(m, "/bot/joints/j1").interfaces.value.is_empty(), "an actuator flag grants no interface");
    let j3 = joint(m, "/bot/joints/j3");
    let flags: Vec<(DofToken, Option<bool>)> = j3.dofs.iter().map(|&d| (m.dofs[d].token, m.dofs[d].isaac_actuator)).collect();
    assert_eq!(flags, vec![(DofToken::RotX, Some(false)), (DofToken::RotZ, Some(true))]);
    assert_eq!(m.frames[m.frame_index("/bot/link2/tcp").unwrap()].role, FrameRole::Site);
    assert!(m.controllers.is_empty());

    let bad = compile_fixture("s18_isaac_bad_arrays.usda", Selection::Asset { entry: None });
    let b = machine(&bad, "/bot");
    assert!(!b.accepted());
    assert!(has(b, Code::IsaacArrayLength, "/bot/j"));
    assert!(has(b, Code::IsaacUnknownTarget, "/bot/missing"));
}

pub(crate) fn s19() {
    let r = compile_fixture("s19_mass_policy.usda", Selection::Asset { entry: None });
    let m = machine(&r, "/heavy");
    assert!(m.accepted(), "{:?}", m.diagnostics);
    let chassis = &m.bodies[0];
    assert_eq!(chassis.mass.mass.as_ref().unwrap().value, 3800.0);
    assert_eq!(chassis.mass.mass.as_ref().unwrap().source, Provenance::Authored);
    let inertia = chassis.mass.diagonal_inertia.as_ref().unwrap();
    assert!(close_v(inertia.value, DVec3::splat(1e-6)));
    assert_eq!(chassis.colliders.len(), 1);
}

pub(crate) fn s20() {
    let r = world("s20_drive_defaults.usda");
    let m = machine(&r, "/World/Drives");
    assert!(m.accepted(), "{:?}", m.diagnostics);
    let defaults = dof_of(m, "/World/Drives/defaults_only");
    let drive = defaults.drive.as_ref().expect("applied drive");
    assert_eq!(drive.stiffness.source, Provenance::SchemaDefault);
    assert_eq!(drive.max_force.value, None);
    assert_eq!(drive.drive_type.value, DriveType::Force);
    assert_eq!(defaults.passive, PassiveKind::ZeroGain);
    let none = dof_of(m, "/World/Drives/no_drive");
    assert!(none.drive.is_none());
    assert_eq!(none.passive, PassiveKind::NoDrive);
    let zero = dof_of(m, "/World/Drives/zero_gain");
    assert_eq!(zero.drive.as_ref().unwrap().stiffness.source, Provenance::Authored);
    assert_eq!(zero.passive, PassiveKind::ZeroGain);
    let spring = dof_of(m, "/World/Drives/spring");
    assert_eq!(spring.passive, PassiveKind::Spring);
    assert_eq!(spring.drive.as_ref().unwrap().drive_type.value, DriveType::Acceleration);
    for dof in &m.dofs {
        assert!(dof.interfaces.value.is_empty(), "a drive grants no command interface: {}", dof.name);
        assert_eq!(dof.default_mode.value, DefaultMode::Disabled);
    }
    assert!(m.owners.is_empty());
}

pub(crate) fn s21() {
    let r = world("s21_limits.usda");
    let m = machine(&r, "/World/Limits");
    assert!(m.accepted(), "{:?}", m.diagnostics);
    let lower = dof_of(m, "/World/Limits/lower_only");
    assert!(close(lower.limit.lower.unwrap(), -30f64.to_radians(), 1e-9) && lower.limit.upper.is_none());
    let cont = dof_of(m, "/World/Limits/continuous");
    assert_eq!((cont.limit.lower, cont.limit.upper, cont.limit.source), (None, None, Provenance::SchemaDefault));
    let inf = dof_of(m, "/World/Limits/explicit_infinite");
    assert_eq!((inf.limit.lower, inf.limit.upper, inf.limit.source), (None, None, Provenance::Authored));
    let upper = dof_of(m, "/World/Limits/upper_only");
    assert_eq!((upper.limit.lower, upper.limit.upper), (None, Some(0.2_f32 as f64)));
    assert!(m.capabilities.iter().any(|c| c.feature == "limit.one_sided"));
}

pub(crate) fn s22() {
    let r = world("s22_invalid_values.usda");
    let future = machine(&r, "/World/Future");
    assert!(!future.accepted());
    assert!(has(future, Code::UnknownSchemaVersion, "/World/Future"));
    let explicit = machine(&r, "/World/Explicit");
    assert_eq!((explicit.profile, explicit.schema_version), (ProfileKind::GearboxLegacy, Some(1)));
    assert!(!error_codes(explicit).iter().any(|c| c.contains("schema_version")), "{:?}", explicit.diagnostics);
    let welded = machine(&r, "/World/Welded");
    assert!(welded.accepted(), "a disabled controller on a welded joint only warns: {:?}", welded.diagnostics);
    assert!(welded.diagnostics.iter().any(|d| d.code == Code::UnresolvedControllerJoint && d.severity == diagnostics::Severity::Warning));
    let live = machine(&r, "/World/Live");
    assert!(!live.accepted() && has(live, Code::UnresolvedControllerJoint, "/World/Live/weld"));
    let wordy = machine(&r, "/World/Wordy");
    assert!(has(wordy, Code::InvalidSchemaVersion, "/World/Wordy"));
    let numbers = machine(&r, "/World/Numbers");
    assert!(!numbers.accepted());
    let codes = error_codes(numbers);
    for code in ["invalid_mass", "invalid_inertia", "invalid_numeric", "invalid_drive", "invalid_limit"] {
        assert!(codes.contains(&code), "{code} missing from {codes:?}");
    }
    let again = world("s22_invalid_values.usda");
    assert_eq!(manifest::to_string(&manifest::result(&r)), manifest::to_string(&manifest::result(&again)), "deterministic diagnostics");
}

/// The fixture inventory of plan 001 §9.1; every ID must run.
/// A brake owns the joints the runtime brakes: a trailer's wheelJoints; on a
/// driven machine's wheels it owns nothing, the drive does.
pub(crate) fn s23() {
    let r = world("s23_service_targets.usda");
    let owner_of = |m: &MachineDescription, joint: &str| {
        let r = m.dofs.iter().position(|d| m.joints[d.joint].path == joint).unwrap();
        m.owners.get(&r).map(|o| (o.owner.clone(), o.kind.clone()))
    };
    let trailer = machine(&r, "/World/Trailer");
    assert!(trailer.accepted(), "{:?}", trailer.diagnostics);
    for axle in ["/World/Trailer/left_axle", "/World/Trailer/right_axle"] {
        assert_eq!(owner_of(trailer, axle), Some(("brakes".to_string(), OwnerKind::Service)), "{axle}");
    }
    let cart = machine(&r, "/World/Cart");
    assert!(cart.accepted(), "{:?}", cart.diagnostics);
    assert_eq!(owner_of(cart, "/World/Cart/axle"), Some(("drive".to_string(), OwnerKind::Controller)));
    let steered = machine(&r, "/World/Steered");
    assert!(steered.accepted(), "{:?}", steered.diagnostics);
    for steer in ["/World/Steered/steer_a", "/World/Steered/steer_b"] {
        assert_eq!(owner_of(steered, steer), Some(("drive".to_string(), OwnerKind::Controller)), "{steer}: own steerJoints and the steering role together");
    }
}

pub(crate) const STRUCTURE_CASES: &[(&str, fn())] = &[
    ("S01", s01),
    ("S02", s02),
    ("S03", s03),
    ("S04", s04),
    ("S05", s05),
    ("S06", s06),
    ("S07", s07),
    ("S08", s08),
    ("S09", s09),
    ("S10", s10),
    ("S11", s11),
    ("S12", s12),
    ("S13", s13),
    ("S14", s14),
    ("S15", s15),
    ("S16", s16),
    ("S17", s17),
    ("S18", s18),
    ("S19", s19),
    ("S20", s20),
    ("S21", s21),
    ("S22", s22),
    ("S23", s23),
];

#[test]
fn structure_fixture_inventory_is_complete_and_passes() {
    let expected: Vec<String> = (1..=23).map(|i| format!("S{i:02}")).collect();
    let ids: Vec<String> = STRUCTURE_CASES.iter().map(|(id, _)| id.to_string()).collect();
    assert_eq!(ids, expected);
    let mut failed = Vec::new();
    for (id, case) in STRUCTURE_CASES {
        if std::panic::catch_unwind(case).is_err() {
            failed.push(*id);
        }
    }
    assert!(failed.is_empty(), "failed structure cases: {failed:?}");
    println!("robot-contract structure inventory: {} cases passed", STRUCTURE_CASES.len());
}

#[test]
fn s01_floating_fixed_and_branched() {
    s01()
}
#[test]
fn s02_articulation_marker_positions() {
    s02()
}
#[test]
fn s03_world_endpoint_on_either_side() {
    s03()
}
#[test]
fn s04_invalid_and_inactive_targets() {
    s04()
}
#[test]
fn s05_reversed_endpoints_keep_their_sign() {
    s05()
}
#[test]
fn s06_loop_closure_is_physical_and_never_a_parent() {
    s06()
}
#[test]
fn s07_disabled_joints_connect_nothing() {
    s07()
}
#[test]
fn s08_frame_only_base() {
    s08()
}
#[test]
fn s09_sites_ride_their_bodies() {
    s09()
}
#[test]
fn s10_up_axis_and_control_frame() {
    s10()
}
#[test]
fn s11_composed_transforms() {
    s11()
}
#[test]
fn s12_physical_scale() {
    s12()
}
#[test]
fn s13_units_are_equivalent_across_stages() {
    s13()
}
#[test]
fn s14_references_and_sublayers() {
    s14()
}
#[test]
fn s15_variants_payloads_and_instances() {
    s15()
}
#[test]
fn s16_joint_type_capabilities() {
    s16()
}
#[test]
fn s17_duplicates_and_nested_roots() {
    s17()
}
#[test]
fn s18_isaac_reporting_and_arrays() {
    s18()
}
#[test]
fn s19_strict_mass_policy() {
    s19()
}
#[test]
fn s20_drive_defaults_and_absence() {
    s20()
}
#[test]
fn s21_one_sided_and_unbounded_limits() {
    s21()
}
#[test]
fn s22_unknown_versions_and_invalid_numbers() {
    s22()
}

/// Fixtures whose normalized manifest is kept beside them as a golden file.
const GOLDEN: &[(&str, bool)] = &[
    ("s01_floating_fixed_branched.usda", true),
    ("s09_sites.usda", false),
    ("s13_units_m_kg.usda", true),
    ("s13_units_cm_g.usda", true),
    ("s18_isaac_metadata.usda", false),
];

fn golden_text(name: &str, world_mode: bool) -> String {
    let selection = if world_mode { Selection::World } else { Selection::Asset { entry: None } };
    manifest::to_string(&manifest::result(&compile_fixture(name, selection)))
}

/// Writes the golden manifests; run on purpose only, never on failure.
#[test]
#[ignore]
fn write_golden_manifests() {
    assert_eq!(std::env::var("GEARBOX_WRITE_GOLDEN").as_deref(), Ok("1"), "set GEARBOX_WRITE_GOLDEN=1");
    for (name, world_mode) in GOLDEN {
        std::fs::write(fixture(&name.replace(".usda", ".manifest.json")), golden_text(name, *world_mode)).unwrap();
    }
}

#[test]
fn manifests_match_their_goldens() {
    for (name, world_mode) in GOLDEN {
        let path = fixture(&name.replace(".usda", ".manifest.json"));
        let golden = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing golden {}", path.display()));
        assert_eq!(golden_text(name, *world_mode), golden, "{name} drifted from its golden manifest");
    }
}

/// Every test a capability cites exists in the source tree.
#[test]
fn capability_evidence_names_real_tests() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut source = String::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                source.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    let mut missing = Vec::new();
    for capability in capabilities::TABLE {
        for test in capability.evidence {
            if !source.contains(&format!("fn {test}(")) {
                missing.push(format!("{}: {test}", capability.feature));
            }
        }
    }
    assert!(missing.is_empty(), "capability evidence without a test: {missing:?}");
}
#[test]
fn s23_service_targets_bind_the_joints_the_runtime_drives() {
    s23()
}
