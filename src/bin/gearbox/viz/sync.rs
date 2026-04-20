//! Each frame, copy vehicle and wheel poses from the sim into Bevy
//! `Transform`s so the rendered meshes track the physics state.

use bevy::prelude::*;

use super::{GearboxSim, VehicleBody, VehicleWheel};

pub fn sync_vehicle_transforms_system(
    sim: Res<GearboxSim>,
    mut bodies: Query<(&VehicleBody, &mut Transform), Without<VehicleWheel>>,
    mut wheels: Query<(&VehicleWheel, &mut Transform), Without<VehicleBody>>,
) {
    for (body, mut tr) in &mut bodies {
        write_pose(&mut tr, sim.0.vehicle_pose(body.id));
    }
    for (wheel, mut tr) in &mut wheels {
        write_pose(&mut tr, sim.0.wheel_pose(wheel.id, wheel.index));
    }
}

fn write_pose(tr: &mut Transform, pose: gearbox::datapod::Pose) {
    tr.translation = Vec3::new(
        pose.point.x as f32,
        pose.point.y as f32,
        pose.point.z as f32,
    );
    tr.rotation = Quat::from_xyzw(
        pose.rotation.x as f32,
        pose.rotation.y as f32,
        pose.rotation.z as f32,
        pose.rotation.w as f32,
    );
}
