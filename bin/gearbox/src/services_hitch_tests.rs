use super::*;

fn fixture() -> (openusd::usd::Stage, MachineInstanceSpec) {
    let usd = r#"#usda 1.0
def Xform "robot" (prepend apiSchemas = ["GearboxMachineAPI", "GearboxControllerAPI:hitch"]) {
    rel gearbox:machine:body = </robot/base>
    token gearbox:controller:hitch:type = "builtin:hitch"
    rel gearbox:controller:hitch:target = [</robot/left_joint>, </robot/right_joint>]
    def Xform "base" (prepend apiSchemas = ["PhysicsRigidBodyAPI"]) {}
    def Xform "left" (prepend apiSchemas = ["PhysicsRigidBodyAPI"]) {}
    def Xform "right" (prepend apiSchemas = ["PhysicsRigidBodyAPI"]) {}
    def PhysicsPrismaticJoint "left_joint" {
        rel physics:body0 = </robot/base>
        rel physics:body1 = </robot/left>
        float gearbox:joint:frequencyHz = 240
        float gearbox:joint:dampingRatio = 0.9
    }
    def PhysicsPrismaticJoint "right_joint" {
        rel physics:body0 = </robot/base>
        rel physics:body1 = </robot/right>
        float gearbox:joint:frequencyHz = -1
    }
}
"#;
    let path = std::env::temp_dir().join(format!("gearbox_hitch_{}_{:?}.usda", std::process::id(), std::thread::current().id()));
    std::fs::write(&path, usd).unwrap();
    let stage = openusd::usd::Stage::open(path.to_str().unwrap()).unwrap();
    let machine = crate::controller::discover_machines_from_stage(&stage).unwrap().remove(0);
    std::fs::remove_file(path).unwrap();
    (stage, machine)
}

#[test]
fn grouped_hitch_discovers_both_targets_and_uses_leader() {
    let (_, machine) = fixture();
    let controller = &machine.controllers[0];
    assert_eq!(controller.target.as_deref(), Some("/robot/left_joint"));
    assert_eq!(controller_joints(&machine, controller), ["/robot/left_joint", "/robot/right_joint"]);
    let left = moved_link(&machine.links, "/robot/left_joint").unwrap();
    let right = moved_link(&machine.links, "/robot/right_joint").unwrap();
    let mut values = LinkValues::default();
    values.set(&machine.id, &left.name, "position", 0.7);
    values.set(&machine.id, &right.name, "position", 0.1);
    assert_eq!(grouped_position(&machine, controller, &values), Some(0.7));
    let mut single = controller.clone();
    single.target_joints.truncate(1);
    assert_eq!(grouped_position(&machine, &single, &values), None);
}

#[test]
fn grouped_hitch_reads_compliance_and_rejects_invalid_frequency() {
    use crate::physics::{attach::attach_joint_softness, markers::UsdPhysicsJoint};
    let (stage, _) = fixture();
    let mut world = World::new();
    let left = world.spawn(UsdPhysicsJoint::default()).id();
    let right = world.spawn(UsdPhysicsJoint::default()).id();
    attach_joint_softness(&mut world, left, &stage, "/robot/left_joint");
    attach_joint_softness(&mut world, right, &stage, "/robot/right_joint");
    let softness = world.get::<UsdPhysicsJoint>(left).unwrap().softness.unwrap();
    assert_eq!(softness.0, 240.0);
    assert!((softness.1 - 0.9).abs() < 1e-6);
    assert_eq!(world.get::<UsdPhysicsJoint>(right).unwrap().softness, None);
}
