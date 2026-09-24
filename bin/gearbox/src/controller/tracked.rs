use super::*;
use crate::physics::backend::{JointId, TrackForceDesc};
#[cfg(test)]
#[path = "tracked_machine_test.rs"]
mod machine_test;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TrackSpec {
    pub link: String,
    pub carrier: String,
    pub sprocket: String,
    pub contacts: Vec<String>,
    pub side: f64,
    pub radius: f64,
    pub axle: [f64; 3],
    pub forward: [f64; 3],
    pub path: Vec<[f32; 3]>,
    pub treads: Vec<String>,
    #[serde(default)]
    pub fem: Option<crate::physics::fem::track_mesh::TrackFemSpec>,
    #[serde(default)]
    pub fem_contacts: Option<crate::physics::fem::track_contacts::TrackContactSpec>,
    #[serde(default)]
    pub fem_visuals: Option<crate::physics::fem::visuals::TrackVisualSpec>,
}

pub(super) fn discover(stage: &openusd::usd::Stage, prim: &SdfPath) -> Result<Vec<TrackSpec>, String> {
    let Some(json) = read_string(stage, prim, "gearbox:machine:tracks") else { return Ok(Vec::new()) };
    let mut tracks: Vec<TrackSpec> = serde_json::from_str(&json).map_err(|e| format!("invalid tracks: {e}"))?;
    if tracks.len() != 2 || tracks[0].side * tracks[1].side != -1.0 {
        return Err("tracked drive requires one left and one right belt".into());
    }
    let mut owned = HashSet::new();
    for track in &mut tracks {
        let axle = DVec3::from_array(track.axle);
        let forward = DVec3::from_array(track.forward);
        if !track.radius.is_finite() || track.radius <= 0.0
            || !axle.is_normalized() || !forward.is_normalized() || axle.dot(forward).abs() > 1e-6
            || track.side.abs() != 1.0 || track.contacts.is_empty()
            || track.axle != [1.0, 0.0, 0.0]
            || track.path.len() < 3 || track.treads.is_empty()
            || track.path.iter().flatten().any(|v| !v.is_finite())
            || track.path.iter().any(|p| p[0].abs() > 1e-6)
            || perimeter(&track.path) < 0.01 {
            return Err("invalid track geometry or axes".into());
        }
        for path in [&mut track.carrier, &mut track.sprocket].into_iter()
            .chain(track.contacts.iter_mut()).chain(track.treads.iter_mut()) {
            *path = rebase_asset_root_target(prim.as_str(), path);
            if !path.starts_with(&format!("{}/", prim.as_str())) {
                return Err("track target is outside its machine".into());
            }
        }
        for path in std::iter::once(&track.sprocket).chain(&track.contacts).chain(&track.treads) {
            if !owned.insert(path.clone()) { return Err("duplicate track ownership".into()); }
        }
        if let Some(contacts) = &mut track.fem_contacts {
            for shape in &mut contacts.shapes {
                shape.body = rebase_asset_root_target(prim.as_str(), &shape.body);
                if !shape.body.starts_with(&format!("{}/", prim.as_str())) {
                    return Err("FEM contact target is outside its machine".into());
                }
            }
        }
        if let Some(visuals) = &mut track.fem_visuals {
            for path in visuals.deformable.iter_mut().chain(std::iter::once(&mut visuals.material)) {
                *path = rebase_asset_root_target(prim.as_str(), path);
                if !path.starts_with(&format!("{}/", prim.as_str())) {
                    return Err("FEM visual target is outside its machine".into());
                }
            }
        }
    }
    Ok(tracks)
}

fn perimeter(path: &[[f32; 3]]) -> f32 {
    (0..path.len()).map(|i| (Vec3::from_array(path[(i + 1) % path.len()]) - Vec3::from_array(path[i])).length()).sum()
}

fn sample(path: &[[f32; 3]], distance: f64) -> (Vec3, Quat) {
    let mut remaining = distance.rem_euclid(perimeter(path) as f64) as f32;
    for i in 0..path.len() {
        let start = Vec3::from_array(path[i]);
        let delta = Vec3::from_array(path[(i + 1) % path.len()]) - start;
        let length = delta.length();
        if length > 1e-8 && remaining <= length {
            let tangent = delta / length;
            return (start + tangent * remaining, Quat::from_rotation_x(tangent.z.atan2(tangent.y)));
        }
        remaining -= length;
    }
    (Vec3::from_array(path[0]), Quat::IDENTITY)
}

fn belt_speeds(cmd: CmdVel, width: f64) -> [f64; 2] {
    if !cmd.linear_mps.is_finite() || !cmd.angular_rps.is_finite() { return [0.0; 2]; }
    let v = f64::from(cmd.linear_mps.clamp(-3.0, 3.0));
    let yaw = f64::from(cmd.angular_rps) * width * 0.5;
    let speeds = [v - yaw, v + yaw];
    let scale = speeds.iter().fold(3.0_f64, |m, v| m.max(v.abs())) / 3.0;
    speeds.map(|v| v / scale)
}

#[derive(Default)]
pub(super) struct Bindings {
    tracks: HashMap<ControllerKey, Vec<BodyId>>,
    failures: HashMap<ControllerKey, String>,
    yaw_trim: HashMap<ControllerKey, f64>,
}

fn body_id(root: Entity, path: &str, prims: &Query<(Entity, &UsdPrimRef)>, parents: &Query<&ChildOf>, physics: &crate::physics::PhysicsWorld) -> Option<BodyId> {
    physics.entity_to_body.get(&find_prim_entity(root, path, prims, parents)?).copied()
}

pub(super) fn apply(
    inventory: Res<ControllerInventory>, commands: Res<ControllerCommands>,
    active: Res<gearbox_api::PhysicsActive>,
    prims: Query<(Entity, &UsdPrimRef)>, parents: Query<&ChildOf>,
    mut physics: ResMut<crate::physics::PhysicsWorld>,
    mut states: ResMut<ControllerStates>, mut values: ResMut<crate::services::LinkValues>,
    mut bindings: Local<Bindings>,
) {
    let mut live = HashSet::new();
    for machine in &inventory.machines {
        let Some(root) = machine.scene_root else { continue };
        for controller in &machine.controllers {
            if !controller.enabled || controller.controller_type != "builtin:tracked_cmd_vel" { continue; }
            let key = ControllerKey::new(root, &machine.id, &controller.instance);
            live.insert(key.clone());
            if !active.0 { continue; }
            let result = (|| -> Result<Vec<BodyId>, String> {
                if machine.tracks.len() != 2 { return Err("missing two authored track definitions".into()); }
                let width = f64::from(controller.track_width.ok_or("missing trackWidth")?);
                if !width.is_finite() || width <= 0.0 { return Err("invalid trackWidth".into()); }
                let chassis = body_id(root, controller.body.as_ref().or(machine.body.as_ref()).ok_or("missing chassis")?, &prims, &parents, &physics).ok_or("chassis not loaded")?;
                let mut cmd = commands.cmd_vel.get(&key).copied().unwrap_or_default();
                if !cmd.linear_mps.is_finite() || !cmd.angular_rps.is_finite() { cmd = CmdVel::default(); }
                cmd.angular_rps = cmd.angular_rps.clamp(-2.0, 2.0);
                let grounded = machine.tracks.iter().all(|spec| body_id(root, &spec.sprocket, &prims, &parents, &physics)
                    .and_then(|id| physics.track_output(id)).is_some_and(|s| s.contacts > 0));
                let trim = bindings.yaw_trim.entry(key.clone()).or_default();
                if !grounded || cmd.angular_rps.abs() < 0.001 { *trim = 0.0; }
                else {
                    let error = cmd.angular_rps as f64 - physics.body(chassis).ok_or("missing chassis")?.angvel().y;
                    let dt = physics.pending_steps as f64 / physics.step_hz;
                    *trim = (*trim + 3.0 * error * dt).clamp(-6.0, 6.0);
                    cmd.angular_rps += (2.0 * error + *trim) as f32;
                }
                let speeds = belt_speeds(cmd, width);
                let mut rotors = Vec::new();
                let mut encoders = Vec::new();
                for spec in &machine.tracks {
                    let carrier = body_id(root, &spec.carrier, &prims, &parents, &physics).ok_or("track carrier not loaded")?;
                    let sprocket = body_id(root, &spec.sprocket, &prims, &parents, &physics).ok_or("sprocket not loaded")?;
                    let joint = physics.joint_between(carrier, sprocket).ok_or("drive joint not loaded")?;
                    if physics.track_output(sprocket).is_none() {
                        let carrier_link = machine.links.get(&spec.link).ok_or("unknown track link")?;
                        let material = |name: &str, fallback| carrier_link.values.iter().find(|(k, _)| k == name).map_or(fallback, |(_, v)| *v);
                        let contact_colliders = spec.contacts.iter().map(|path| {
                            let entity = find_prim_entity(root, path, &prims, &parents).ok_or("belt collider not loaded")?;
                            physics.entity_to_collider.get(&entity).copied().ok_or("belt collider not registered")
                        }).collect::<Result<Vec<_>, _>>()?;
                        physics.configure_track(TrackForceDesc {
                            carrier, sprocket, joint, contact_colliders,
                            local_axle: DVec3::from_array(spec.axle), local_forward: DVec3::from_array(spec.forward),
                            pitch_radius: spec.radius, longitudinal_friction: material("track_friction_long", 0.85), lateral_friction: material("track_friction_lateral", 0.65),
                            slip_damping: material("track_slip_damping", 8000.0), max_torque: controller.max_wheel_torque_nm.unwrap_or(300.0) as f64,
                            max_power: controller.max_power_kw.unwrap_or(10.0) as f64 * 500.0, speed_gain: material("track_speed_gain", 120.0),
                        })?;
                    }
                    physics.set_track_speed(sprocket, speeds[usize::from(spec.side < 0.0)])?;
                    rotors.push(sprocket);
                    if let Some(output) = physics.track_output(sprocket) {
                        encoders.push((output.travel / spec.radius, output.angular_velocity));
                        for (name, value) in [
                            ("track_travel_m", output.travel), ("track_speed_mps", output.angular_velocity * spec.radius),
                            ("track_torque_nm", output.motor_torque), ("track_force_n", output.longitudinal_force),
                            ("track_normal_load_n", output.normal_load), ("track_contacts", output.contacts as f64),
                        ] { values.set(&machine.id, &spec.link, name, value); }
                        let rollers: Vec<(JointId, f64)> = machine.links.links.iter().filter(|link| link.parent.as_deref() == Some(&spec.link))
                            .filter_map(|link| {
                                let body = body_id(root, link.body_prim.as_deref()?, &prims, &parents, &physics)?;
                                if body == sprocket { return None; }
                                let radius = link.values.iter().find(|(k, _)| k == "rolling_radius_m")?.1;
                                (radius > 0.0).then_some((physics.joint_between(carrier, body)?, radius))
                            }).collect();
                        for (joint, radius) in rollers {
                            if let Some(j) = physics.joint_mut(joint, true) {
                                j.set_motor_model(JointAxis::AngX, MotorModel::Force);
                                j.set_motor_velocity(JointAxis::AngX, output.angular_velocity * spec.radius / radius, 1.0);
                                j.set_motor_max_force(JointAxis::AngX, 2.0);
                            }
                        }
                    }
                }
                let body = physics.body(chassis).ok_or("missing chassis state")?;
                let pos = body.translation();
                let (region, position_m) = crate::globe::site_local(pos.x, pos.y, pos.z);
                let (roll_rad, pitch_rad) = machine_roll_pitch_rad(body);
                states.states.insert(key.clone(), ControllerState {
                    region, position_m, heading_rad: machine_heading_rad(body), roll_rad, pitch_rad,
                    linear_speed_mps: body_forward_vector(body).map_or(0.0, |f| body.linvel().dot(f)),
                    yaw_rate_rps: body.angvel().y, wheel_encoders: encoders,
                });
                Ok(rotors)
            })();
            match result {
                Ok(rotors) => {
                    if !bindings.tracks.contains_key(&key) { info!("gearbox-control: {} torque-driven tracks ready", machine.id); }
                    bindings.tracks.insert(key.clone(), rotors);
                    bindings.failures.remove(&key);
                }
                Err(error) => {
                    for spec in &machine.tracks {
                        if let Some(id) = body_id(root, &spec.sprocket, &prims, &parents, &physics) { physics.remove_track(id); }
                    }
                    if bindings.failures.get(&key) != Some(&error) { warn!("gearbox-control: {} tracks: {error}", machine.id); }
                    bindings.failures.insert(key.clone(), error);
                    bindings.tracks.remove(&key);
                    bindings.yaw_trim.remove(&key);
                    states.states.remove(&key);
                }
            }
        }
    }
    bindings.tracks.retain(|key, rotors| {
        if live.contains(key) { true } else {
            for &id in rotors.iter() { physics.remove_track(id); }
            states.states.remove(key);
            false
        }
    });
    bindings.failures.retain(|key, _| live.contains(key));
    bindings.yaw_trim.retain(|key, _| live.contains(key));
}

pub(super) fn animate(
    inventory: Res<ControllerInventory>, physics: Res<crate::physics::PhysicsWorld>,
    prims: Query<(Entity, &UsdPrimRef)>, parents: Query<&ChildOf>, mut transforms: Query<&mut Transform>,
) {
    for machine in &inventory.machines {
        let Some(root) = machine.scene_root else { continue };
        for spec in &machine.tracks {
            let Some(sprocket) = body_id(root, &spec.sprocket, &prims, &parents, &physics) else { continue };
            let Some(output) = physics.track_output(sprocket) else { continue };
            let length = perimeter(&spec.path) as f64;
            for (i, path) in spec.treads.iter().enumerate() {
                let Some(entity) = find_prim_entity(root, path, &prims, &parents) else { continue };
                if let Ok(mut transform) = transforms.get_mut(entity) {
                    let (position, rotation) = sample(&spec.path, output.travel + i as f64 * length / spec.treads.len() as f64);
                    transform.translation = position;
                    transform.rotation = rotation;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn differential_mix_and_invalid_command_stop() {
        assert_eq!(belt_speeds(CmdVel { linear_mps: 1.0, angular_rps: 0.0 }, 0.8), [1.0, 1.0]);
        assert_eq!(belt_speeds(CmdVel { linear_mps: 0.0, angular_rps: 1.0 }, 0.8), [-0.4, 0.4]);
        assert_eq!(belt_speeds(CmdVel { linear_mps: f32::NAN, angular_rps: 1.0 }, 0.8), [0.0; 2]);
    }
    #[test]
    fn belt_path_wraps_both_directions() {
        let path = [[0., 0., 0.], [0., 1., 0.], [0., 1., 1.], [0., 0., 1.]];
        assert_eq!(sample(&path, 0.5).0, Vec3::new(0., 0.5, 0.));
        assert_eq!(sample(&path, 4.5).0, sample(&path, 0.5).0);
        assert_eq!(sample(&path, -0.5).0, Vec3::new(0., 0., 0.5));
    }
}
