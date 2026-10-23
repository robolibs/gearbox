//! What the runtime does with each structural and control feature, with the
//! tests that show it. Every machine reports the entries it uses; using an
//! unsupported one rejects a strict machine.

use std::collections::BTreeMap;

use super::compile::Builder;
use super::diagnostics::Code;
use super::model::*;

pub struct Capability {
    pub feature: &'static str,
    pub status: CapabilityStatus,
    pub reason: &'static str,
    pub evidence: &'static [&'static str],
}

use CapabilityStatus::{SupportedApproximate as Approx, SupportedExact as Exact, Unsupported};

pub const TABLE: &[Capability] = &[
    Capability {
        feature: "joint.revolute",
        status: Exact,
        reason: "hinge about the authored axis with limits and drives",
        evidence: &["s01_floating_fixed_and_branched", "c01_revolute_and_prismatic_commands_move_their_joints"],
    },
    Capability {
        feature: "joint.prismatic",
        status: Exact,
        reason: "slider along the authored axis with limits and drives",
        evidence: &["s16_joint_type_capabilities", "c01_revolute_and_prismatic_commands_move_their_joints"],
    },
    Capability {
        feature: "joint.fixed",
        status: Exact,
        reason: "both frames locked",
        evidence: &["s16_joint_type_capabilities", "a_fixed_joint_keeps_the_rotation_of_its_authored_frames"],
    },
    Capability {
        feature: "joint.d6",
        status: Exact,
        reason: "each axis free, limited, locked (low > high) or driven",
        evidence: &["s16_joint_type_capabilities", "a_d6_hinge_swings_down_to_its_limit"],
    },
    Capability {
        feature: "joint.spherical",
        status: Approx,
        reason: "ball joint; cone limits are converted, their solver response is soft",
        evidence: &["s16_joint_type_capabilities", "ball_distance_and_break_limits_carry_their_authored_sides", "a_spherical_cone_limit_stops_the_swing"],
    },
    Capability {
        feature: "joint.distance",
        status: Exact,
        reason: "min/max distance held by the backend's distance constraint",
        evidence: &["s16_joint_type_capabilities", "ball_distance_and_break_limits_carry_their_authored_sides", "a_distance_joint_holds_its_maximum"],
    },
    Capability {
        feature: "joint.break_limits",
        status: Approx,
        reason: "break force and torque converted; breakage follows the backend's effort estimate",
        evidence: &["ball_distance_and_break_limits_carry_their_authored_sides", "a_break_force_releases_an_overloaded_joint"],
    },
    Capability {
        feature: "joint.world_anchor",
        status: Exact,
        reason: "a world-anchored joint keeps its DOFs against a fixed anchor body",
        evidence: &["s03_world_endpoint_on_either_side", "a_world_revolute_joint_keeps_its_degree_of_freedom"],
    },
    Capability {
        feature: "joint.closure",
        status: Approx,
        reason: "excludeFromArticulation joints are soft loop constraints outside the reduced tree",
        evidence: &["s06_loop_closure_is_physical_and_never_a_parent"],
    },
    Capability {
        feature: "drive.force",
        status: Exact,
        reason: "spring-damper force drive with gains and cap in SI",
        evidence: &["force_drive_gains_follow_their_dimensions", "s13_units_are_equivalent_across_stages"],
    },
    Capability {
        feature: "drive.acceleration",
        status: Exact,
        reason: "gains act per unit of joint inertia; angular gains per degree, as Isaac/PhysX reads them",
        evidence: &[
            "acceleration_drive_gains_ignore_mass_and_length",
            "c02_force_and_acceleration_drives_keep_their_type",
            "native_acceleration_drive_follows_the_degree_convention",
        ],
    },
    Capability {
        feature: "limit.one_sided",
        status: Exact,
        reason: "an unbounded side stays unbounded",
        evidence: &["s21_one_sided_and_unbounded_limits"],
    },
    Capability {
        feature: "command.position",
        status: Exact,
        reason: "drive target position on the exact DOF",
        evidence: &["c01_revolute_and_prismatic_commands_move_their_joints"],
    },
    Capability {
        feature: "command.velocity",
        status: Exact,
        reason: "drive target velocity on the exact DOF",
        evidence: &["c01_revolute_and_prismatic_commands_move_their_joints"],
    },
    Capability {
        feature: "command.effort",
        status: Exact,
        reason: "generalized force on the joint coordinate",
        evidence: &["c01_true_effort_is_a_generalized_force"],
    },
    Capability {
        feature: "command.multi_dof_joint",
        status: Unsupported,
        reason: "spherical and distance joints take no commands",
        evidence: &["s16_joint_type_capabilities"],
    },
    Capability {
        feature: "body.uniform_scale",
        status: Approx,
        reason: "joint frames and centre of mass follow the body's uniform scale; mass and inertia are taken as authored",
        evidence: &["s12_physical_scale"],
    },
    Capability {
        feature: "composition.instance",
        status: Unsupported,
        reason: "instanceable prims are not traversed",
        evidence: &["s15_variants_payloads_and_instances"],
    },
    Capability {
        feature: "composition.unloaded_payload",
        status: Unsupported,
        reason: "unloaded payloads are not read",
        evidence: &["s15_variants_payloads_and_instances"],
    },
];

pub fn lookup(feature: &str) -> &'static Capability {
    TABLE.iter().find(|c| c.feature == feature).expect("capability in table")
}

pub(super) fn evaluate(b: &mut Builder) {
    let mut used: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut note = |feature: &'static str, path: &str| used.entry(feature).or_default().push(path.to_string());
    for joint in &b.machine.joints {
        if !joint.is_physical() {
            continue;
        }
        let feature = match joint.kind {
            JointKind::Revolute => "joint.revolute",
            JointKind::Prismatic => "joint.prismatic",
            JointKind::Fixed => "joint.fixed",
            JointKind::D6 => "joint.d6",
            JointKind::Spherical => "joint.spherical",
            JointKind::Distance => "joint.distance",
            JointKind::Unknown(_) => continue,
        };
        note(feature, &joint.path);
        if joint.class == JointClass::WorldAnchor {
            note("joint.world_anchor", &joint.path);
        }
        if joint.class == JointClass::Closure {
            note("joint.closure", &joint.path);
        }
        if joint.break_force.is_some() || joint.break_torque.is_some() {
            note("joint.break_limits", &joint.path);
        }
    }
    for dof in &b.machine.dofs {
        let path = &b.machine.joints[dof.joint].path;
        if let Some(drive) = &dof.drive {
            note(if drive.drive_type.value == DriveType::Force { "drive.force" } else { "drive.acceleration" }, path);
        }
        if dof.limit.lower.is_some() != dof.limit.upper.is_some() {
            note("limit.one_sided", path);
        }
        for interface in &dof.interfaces.value {
            note(
                match interface {
                    Interface::Position => "command.position",
                    Interface::Velocity => "command.velocity",
                    Interface::Effort => "command.effort",
                },
                path,
            );
        }
    }
    for body in &b.machine.bodies {
        if (body.scale - 1.0).abs() > 1e-9 {
            note("body.uniform_scale", &body.path);
        }
    }
    for d in &b.scene.diagnostics {
        if let Some(path) = d.path.as_deref().filter(|p| super::scene::Scene::is_under(p, &b.scope.root) || super::scene::Scene::is_under(&b.scope.root, p)) {
            match d.code {
                Code::InstanceProxy => note("composition.instance", path),
                Code::PayloadUnloaded => note("composition.unloaded_payload", path),
                _ => {}
            }
        }
    }
    for (feature, paths) in used {
        let capability = lookup(feature);
        if capability.status == CapabilityStatus::Unsupported {
            for path in &paths {
                b.strict_or_warn(Code::UnsupportedCapability, Some(path), format!("{feature}: {}", capability.reason));
            }
        }
        b.machine.capabilities.push(CapabilityResult {
            feature: feature.to_string(),
            subject: paths.first().cloned(),
            status: capability.status,
            source_property: None,
            reason: format!("{} ({} use{})", capability.reason, paths.len(), if paths.len() == 1 { "" } else { "s" }),
            evidence: capability.evidence.to_vec(),
        });
    }
}
