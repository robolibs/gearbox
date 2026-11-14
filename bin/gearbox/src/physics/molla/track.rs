use super::*;

impl MollaBackend {
    pub(super) fn configure_track_force(&mut self, desc: TrackForceDesc) -> Result<(), String> {
        let carrier = self
            .bodies
            .get(&desc.carrier)
            .ok_or("unknown track carrier")?
            .handle;
        let sprocket = self
            .bodies
            .get(&desc.sprocket)
            .ok_or("unknown track sprocket")?
            .handle;
        let spin_joint = self
            .joints
            .get(&desc.joint)
            .ok_or("unknown track drive joint")?
            .handle;
        let contact_colliders = desc
            .contact_colliders
            .iter()
            .map(|id| {
                self.colliders
                    .get(id)
                    .map(|c| c.handle)
                    .ok_or("unknown track contact collider")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut world = self.shared.world();
        let RigidWorld { scene, tracks, .. } = &mut *world;
        tracks
            .insert(
                scene,
                molla_solvers::track_forces::TrackDesc {
                    carrier,
                    sprocket,
                    spin_joint,
                    contact_colliders,
                    local_axle: desc.local_axle,
                    local_forward: desc.local_forward,
                    pitch_radius: desc.pitch_radius,
                    material: molla_vehicle::track::TrackMaterial {
                        longitudinal_friction: desc.longitudinal_friction,
                        lateral_friction: desc.lateral_friction,
                        slip_damping: desc.slip_damping,
                    },
                    motor: molla_vehicle::track::TrackMotor {
                        max_torque: desc.max_torque,
                        max_power: desc.max_power,
                        speed_gain: desc.speed_gain,
                    },
                },
            )
            .map_err(|e| e.to_string())
    }
}
