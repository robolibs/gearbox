//! Capture-time placement for native LiDAR chunks: the sensor pose and its
//! site datum, snapshotted at dispatch and carried in the `capture.v1` prop.

use std::sync::atomic::{AtomicU64, Ordering};

use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use gearbox_api::wire::draw_data::{
    CaptureConvention, CaptureStamp, CaptureTransform, DatumSnapshot, NATIVE_CAPTURE_PROP,
    NativeLidarCapture, RigidPose, lidar_chunk_capacity,
};
use gearbox_api::{LIDAR_CHUNK_PULSES, Props, SimResetRequest, clear_scope};
use gearbox_globe::Datum;
use molla_math::Transform as MTransform;

static SCENE_EPOCH: AtomicU64 = AtomicU64::new(1);

/// The scene identity captures are stamped with; a machine reset starts a new one.
pub fn scene_epoch() -> u64 {
    SCENE_EPOCH.load(Ordering::Relaxed)
}

pub fn bump_scene_epoch_on_reset(messages: Option<MessageReader<SimResetRequest>>) {
    let Some(mut messages) = messages else { return };
    if messages
        .read()
        .any(|m| matches!(m.scope, clear_scope::ALL | clear_scope::MACHINES))
    {
        SCENE_EPOCH.fetch_add(1, Ordering::Relaxed);
    }
}

/// A sensor pose in its site's ENU frame, with that site's datum, as dispatched.
#[derive(Clone, Debug, PartialEq)]
pub struct CapturePose {
    pub pose: RigidPose,
    pub datum: DatumSnapshot,
    pub scene_epoch: u64,
}

/// ENU (x east, y north, z up) into site axes (x north, y up, z east).
fn enu_to_site() -> DQuat {
    DQuat::from_mat3(&DMat3::from_cols(DVec3::Z, DVec3::X, DVec3::Y))
}

fn wxyz(q: DQuat) -> [f64; 4] {
    [q.w, q.x, q.y, q.z]
}

impl CapturePose {
    /// `world` is the sensor frame in physics coordinates; `site` is the datum
    /// of the region it stands in and the position within it.
    pub fn of(world: &MTransform, site: (Datum, DVec3), scene_epoch: u64) -> Self {
        let (datum, local) = site;
        let to_enu = enu_to_site().inverse();
        let r = world.rotation;
        let rotation = (to_enu * DQuat::from_xyzw(r.x, r.y, r.z, r.w)).normalize();
        Self {
            pose: RigidPose {
                translation_m: (to_enu * local).to_array(),
                rotation_wxyz: wxyz(rotation),
            },
            datum: DatumSnapshot {
                origin_ecef_m: datum.origin.to_array(),
                axes_to_ecef_wxyz: wxyz((datum.rotation * enu_to_site()).normalize()),
            },
            scene_epoch,
        }
    }

    /// The pose at dispatch for a sensor frame in the live globe sites.
    pub fn dispatched(world: &MTransform) -> Option<Self> {
        let p = world.position;
        let site = crate::globe::physics_site(DVec3::new(p.x, p.y, p.z))?;
        Some(Self::of(world, site, scene_epoch()))
    }

    pub fn native(&self, machine: &str, link: &str, sim_time: f64, sample: u64) -> NativeLidarCapture {
        NativeLidarCapture {
            revision: 1,
            machine: machine.into(),
            link: link.into(),
            convention: CaptureConvention::RightHandedZUpMetres,
            transform: CaptureTransform {
                revision: 1,
                source_frame: format!("{machine}/{link}"),
                target_frame: "world".into(),
                stamp: CaptureStamp {
                    simulation_time_s: sim_time,
                    source_time_ns: None,
                    source_clock: None,
                    sample,
                    scene_epoch: self.scene_epoch,
                },
                pose: self.pose.clone(),
                datum: Some(self.datum.clone()),
            },
        }
    }
}

/// The props every chunk of one sweep carries and how many pulses fit a chunk.
/// A capture that cannot be encoded leaves the chunks without one.
pub fn chunk_props(link: &str, capture: Option<&NativeLidarCapture>) -> (Vec<u8>, usize, Option<String>) {
    let plain = Props::from_pairs(&[("name", link)]).into_bytes();
    let Some(capture) = capture else {
        return (plain, LIDAR_CHUNK_PULSES, None);
    };
    let encoded = capture.to_json().and_then(|json| {
        let props = Props::from_pairs(&[("name", link), (NATIVE_CAPTURE_PROP, &json)]).into_bytes();
        Ok((lidar_chunk_capacity(&props)?, props))
    });
    match encoded {
        Ok((capacity, props)) => (props, capacity, None),
        Err(error) => (plain, LIDAR_CHUNK_PULSES, Some(error)),
    }
}

#[cfg(test)]
#[path = "sensor_capture_tests.rs"]
mod tests;
