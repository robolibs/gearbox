use bevy::camera::primitives::Aabb;
use bevy::math::{Affine3A, Vec3A};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use molla_vehicle::real_tire::LoadedTireEnvelope;

pub(super) type TyreMaterial = ExtendedMaterial<StandardMaterial, TyreExtension>;

#[derive(Clone, Copy, Debug, Reflect, ShaderType, PartialEq)]
pub(super) struct TyreFrame {
    to_contact: Mat4,
    from_contact: Mat4,
    envelope: Vec4,
    ground: Vec4,
}

impl TyreFrame {
    pub fn new(to_contact: Affine3A, envelope: LoadedTireEnvelope, ground: Option<f64>) -> Self {
        Self {
            to_contact: Mat4::from(to_contact),
            from_contact: Mat4::from(to_contact.inverse()),
            envelope: Vec4::new(
                envelope.radius as f32,
                envelope.bead_radius as f32,
                envelope.width as f32,
                envelope.loaded_radius as f32,
            ),
            ground: Vec4::new(
                ground.unwrap_or(0.0) as f32,
                f32::from(ground.is_some()),
                0.0,
                0.0,
            ),
        }
    }

    pub fn bounds(self, source: Aabb) -> Aabb {
        let centre = self.to_contact.transform_point3(source.center.into());
        let row_y = Vec3::new(
            self.to_contact.x_axis.y,
            self.to_contact.y_axis.y,
            self.to_contact.z_axis.y,
        );
        let min_y = centre.y - row_y.abs().dot(source.half_extents.into());
        let compression =
            (self.envelope.x - self.envelope.w.clamp(self.envelope.y, self.envelope.x)).max(0.0);
        let lift = if self.ground.y > 0.0 {
            compression.max(-self.ground.x - min_y)
        } else {
            compression
        };
        let expansion = self.from_contact.y_axis.truncate().abs() * lift
            + self.from_contact.z_axis.truncate().abs() * (0.65 * compression);
        Aabb {
            center: source.center,
            half_extents: source.half_extents + Vec3A::from(expansion),
        }
    }
}

#[derive(Clone, Copy, Debug, Reflect, ShaderType, PartialEq)]
pub(super) struct TyreUniform {
    current: TyreFrame,
    previous: TyreFrame,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(super) struct TyreExtension {
    #[storage(100, read_only)]
    state: Handle<ShaderBuffer>,
}

impl MaterialExtension for TyreExtension {
    fn vertex_shader() -> ShaderRef {
        "embedded://gearbox/viewer/tyre_gpu.wgsl".into()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn deferred_vertex_shader() -> ShaderRef {
        Self::vertex_shader()
    }
}

pub(super) struct GpuRubber {
    pub source: Handle<StandardMaterial>,
    pub material: Handle<TyreMaterial>,
    pub bounds: Aabb,
    buffer: Handle<ShaderBuffer>,
    state: TyreUniform,
}

impl GpuRubber {
    pub fn new(
        source: Handle<StandardMaterial>,
        base: StandardMaterial,
        bounds: Aabb,
        frame: TyreFrame,
        materials: &mut Assets<TyreMaterial>,
        buffers: &mut Assets<ShaderBuffer>,
    ) -> Self {
        let state = TyreUniform {
            current: frame,
            previous: frame,
        };
        let buffer = buffers.add(ShaderBuffer::from(state));
        let material = materials.add(TyreMaterial {
            base,
            extension: TyreExtension {
                state: buffer.clone(),
            },
        });
        Self {
            source,
            material,
            bounds,
            buffer,
            state,
        }
    }

    pub fn update(&mut self, frame: TyreFrame, buffers: &mut Assets<ShaderBuffer>) {
        let old = self.state;
        let next = TyreUniform {
            current: frame,
            previous: old.current,
        };
        if next != old {
            if let Some(mut buffer) = buffers.get_mut(&self.buffer) {
                buffer.set_data(next);
                self.state = next;
            }
        }
    }

    #[cfg(test)]
    pub fn assert_frame(&self, frame: TyreFrame) {
        assert_eq!(self.state.current, frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_updates_reuse_buffer_and_material_handles() {
        let mut materials = Assets::<TyreMaterial>::default();
        let mut buffers = Assets::<ShaderBuffer>::default();
        let envelope = LoadedTireEnvelope {
            radius: 1.0,
            bead_radius: 0.5,
            width: 0.4,
            loaded_radius: 0.7,
        };
        let low = TyreFrame::new(Affine3A::IDENTITY, envelope, Some(0.7));
        let high = TyreFrame::new(
            Affine3A::IDENTITY,
            LoadedTireEnvelope {
                loaded_radius: 0.95,
                ..envelope
            },
            Some(0.95),
        );
        let mut gpu = GpuRubber::new(
            Handle::default(),
            StandardMaterial::default(),
            Aabb::default(),
            low,
            &mut materials,
            &mut buffers,
        );
        let material = gpu.material.clone();
        let buffer = gpu.buffer.clone();
        let before = buffers.get(&buffer).unwrap().data.clone();
        gpu.update(high, &mut buffers);
        assert_eq!(
            gpu.state,
            TyreUniform {
                current: high,
                previous: low
            }
        );
        assert_ne!(before, buffers.get(&buffer).unwrap().data);
        gpu.update(high, &mut buffers);
        assert_eq!(
            gpu.state,
            TyreUniform {
                current: high,
                previous: high
            }
        );
        let settled = buffers.get(&buffer).unwrap().data.clone();
        gpu.update(high, &mut buffers);
        assert_eq!(settled, buffers.get(&buffer).unwrap().data);
        assert_eq!(gpu.material, material);
        assert_eq!(gpu.buffer, buffer);
        assert_eq!(materials.get(&material).unwrap().extension.state, buffer);
        assert_eq!((materials.len(), buffers.len()), (1, 1));
        assert_eq!(settled.unwrap().len(), 320);
    }

    #[test]
    fn conservative_bounds_contain_grounded_deformation_with_scaled_meshes() {
        let source = Aabb {
            center: Vec3A::ZERO,
            half_extents: Vec3A::new(1.05, 1.05, 0.2),
        };
        for scale in [Vec3::ONE, Vec3::new(0.8, 1.2, 2.0)] {
            let to_contact = Affine3A::from_scale_rotation_translation(
                scale,
                Quat::from_rotation_z(0.4),
                Vec3::ZERO,
            );
            for ground in [None, Some(0.7), Some(-0.1)] {
                let envelope = LoadedTireEnvelope {
                    radius: 1.0,
                    bead_radius: 0.5,
                    width: 0.4,
                    loaded_radius: 0.7,
                };
                let bounds = TyreFrame::new(to_contact, envelope, ground).bounds(source);
                for i in 0..180 {
                    let a = i as f32 * std::f32::consts::TAU / 180.0;
                    let p = Vec3::new(1.05 * a.cos(), 1.05 * a.sin(), 0.2);
                    let contact = to_contact.transform_point3(p);
                    let deformed = super::super::tyre_mesh::supported_vertex(
                        envelope,
                        contact.to_array().map(f64::from),
                        ground,
                    );
                    let local = to_contact
                        .inverse()
                        .transform_point3(Vec3::from_array(deformed.map(|v| v as f32)));
                    assert!(
                        (local - Vec3::from(bounds.center))
                            .abs()
                            .cmple(Vec3::from(bounds.half_extents) + Vec3::splat(1e-6))
                            .all()
                    );
                }
            }
        }
    }
}
