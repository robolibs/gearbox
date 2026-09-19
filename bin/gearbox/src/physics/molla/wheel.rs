use super::*;

impl MollaBackend {
    pub(super) fn configure_wheel_force(&mut self, desc: WheelForceDesc) -> Result<(), String> {
        if !desc.supported_mass.is_finite()
            || desc.supported_mass <= 0.0
            || !desc.radius.is_finite()
            || desc.radius <= 0.0
            || !desc.forward.is_finite()
            || desc.forward.length_squared() < 1e-12
        {
            return Err("invalid wheel mass, radius or heading".into());
        }
        let body = self
            .bodies
            .get(&desc.body)
            .ok_or("unknown wheel body")?
            .handle;
        let joint = self.joints.get(&desc.joint).ok_or("unknown wheel joint")?;
        let mut world = self.shared.world();
        let authored = world
            .scene
            .joint(joint.handle)
            .ok_or("missing wheel joint")?;
        let parent = authored.parent.ok_or("wheel joint has no bearing body")?;
        let (heading_body, direction) = if authored.child == body {
            (parent, 1.0)
        } else if parent == body {
            (authored.child, -1.0)
        } else {
            return Err("wheel is not a joint endpoint".into());
        };
        let parent_pose = world.scene.body_pose(parent).map_err(|e| e.to_string())?;
        let bearing_pose = world
            .scene
            .body_pose(heading_body)
            .map_err(|e| e.to_string())?;
        let axle = parent_pose.rotation
            * authored.frame_parent.rotation
            * authored.axis.normalize()
            * direction;
        let roll = axle.cross(DVec3::Y).normalize_or_zero();
        let alignment = roll.dot(desc.forward.normalize());
        if alignment.abs() < 0.1 {
            return Err("wheel axle does not define a forward rolling direction".into());
        }
        let sign = alignment.signum();
        let load = desc.supported_mass * 9.81;
        let stiffness = load / (0.04 * desc.radius);
        let config = molla_solvers::wheel_forces::WheelDesc {
            body,
            spin_joint: joint.handle,
            heading_body,
            local_heading: bearing_pose.rotation.conjugate() * roll * sign,
            local_hub: desc.local_hub,
            spin_sign: sign,
            radius: desc.radius,
            normal_stiffness: stiffness,
            normal_damping: 1.4 * (stiffness * desc.supported_mass).sqrt(),
            tire: Arc::new(molla_vehicle::TmSimpleTire {
                c_kappa: load * 10.0,
                c_alpha: load * 8.0,
                pneumatic_trail: 0.03 * desc.radius,
            }),
        };
        let RigidWorld { scene, wheels, .. } = &mut *world;
        wheels
            .insert(scene, config)
            .map_err(|error| error.to_string())?;
        self.wheels.insert(desc.body, (desc.joint, sign));
        Ok(())
    }

    pub(super) fn wheel_force_output(&self, body: BodyId) -> Option<WheelForceOutput> {
        let (joint, _) = self.wheels.get(&body)?;
        let wheel = self.bodies.get(&body)?.handle;
        self.joints.get(joint)?;
        let world = self.shared.world();
        let mut out = WheelForceOutput::default();
        let mut impulse = 0.0;
        for sample in world
            .wheels
            .samples(&world.scene)
            .iter()
            .filter(|s| s.wheel == wheel)
        {
            let normal_impulse = sample.output.fz.max(0.0) * sample.dt;
            if !sample.output.in_contact || normal_impulse <= 0.0 {
                continue;
            }
            impulse += normal_impulse;
            out.normal += sample.normal * normal_impulse;
            out.contact_point += DVec3::from_array(sample.output.contact_point) * normal_impulse;
            out.grip_force += sample.friction * normal_impulse;
            out.slip_ratio += sample.output.slip_ratio * normal_impulse;
            out.slip_angle += sample.output.slip_angle * normal_impulse;
        }
        if impulse > 0.0 {
            out.in_contact = true;
            out.normal = out.normal.normalize_or_zero();
            out.contact_point /= impulse;
            out.slip_ratio /= impulse;
            out.slip_angle /= impulse;
            out.normal_force = impulse / self.wheel_step_dt;
            out.grip_force /= self.wheel_step_dt;
        }
        Some(out)
    }
}
