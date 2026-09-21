use bevy::math::{Mat4, Quat, Vec3};
use openusd::sdf::Path;

#[test]
fn oxbo_bunker_stays_in_its_closed_blender_pose() {
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let stage = openusd::usd::Stage::open(assets.join("oxbo.usd").to_str().unwrap()).unwrap();
    let mut world = Mat4::IDENTITY;
    for path in [
        "/robot",
        "/robot/chassis",
        "/robot/chassis/JOINT___hopper_lift",
        "/robot/chassis/JOINT___hopper_lift/JOINT___unload_conveyor_fold",
    ] {
        let path: Path = path.parse().unwrap();
        let t = usd_bevy::read::xform::read_transform(&stage, &path)
            .unwrap()
            .unwrap();
        world *= Mat4::from_scale_rotation_translation(
            Vec3::from(t.scale),
            Quat::from_array(t.rotate),
            Vec3::from(t.translate),
        );
    }
    let expected = Mat4::from_rotation_translation(
        Quat::from_rotation_y(-30.0_f32.to_radians()),
        Vec3::new(1.12, -2.08, 2.049),
    );
    assert!(
        world.abs_diff_eq(expected, 0.00002),
        "bunker transform: {world:?}"
    );
}
