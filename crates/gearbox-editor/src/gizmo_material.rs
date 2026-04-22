//! Material wrapper that makes transform-gizmo meshes always render on
//! top of the world — matches the Blender / transform-gizmo behaviour.
//!
//! Extends Bevy's `StandardMaterial` via [`ExtendedMaterial`], keeping
//! the mesh/lit/unlit pipeline unchanged — only the depth-stencil
//! state is overridden. `depth_compare = Always` = pass the depth
//! test regardless of depth buffer; `depth_write_enabled = false` =
//! don't clobber world depth so downstream passes still see it.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};

/// Zero-sized extension that flips the depth-stencil state on
/// whatever base material it wraps.
#[derive(Asset, AsBindGroup, TypePath, Default, Clone)]
pub struct GizmoOnTop {}

impl MaterialExtension for GizmoOnTop {
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_compare = CompareFunction::Always;
            ds.depth_write_enabled = false;
        }
        Ok(())
    }
}

/// `StandardMaterial` with depth testing disabled.
pub type GizmoMaterial = ExtendedMaterial<StandardMaterial, GizmoOnTop>;
