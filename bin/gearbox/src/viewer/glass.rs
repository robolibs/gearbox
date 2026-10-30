//! Glass the eye passes through: glass fades out over the last stretch
//! before the eye, on the fragments nearest it alone, so flying through a
//! pane opens a clear spot in its haze where the whole pane dropped in one
//! frame, and every other pane stays as it is.

use std::collections::HashMap;

use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

pub(crate) type GlassMaterial = ExtendedMaterial<StandardMaterial, NearFade>;

/// Glass is a blended material no more opaque than this, or one whose
/// opacity is drawn from its texture, as a foil's is.
const GLASS_ALPHA: f32 = 0.35;
/// Within the first distance of the eye glass is clear; beyond the second, as it is.
const CLEAR_M: f32 = 0.15;
const SHOWN_M: f32 = 1.2;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct NearFade {
    /// Metres from the eye where the glass begins to show, and where it shows in full, in x and y.
    #[uniform(100)]
    band: Vec4,
}

impl MaterialExtension for NearFade {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox/viewer/glass.wgsl".into()
    }
}

pub(crate) struct GlassPlugin;

impl Plugin for GlassPlugin {
    fn build(&self, app: &mut App) {
        if app.get_sub_app(bevy::render::RenderApp).is_some() {
            bevy::asset::embedded_asset!(app, "glass.wgsl");
            app.add_plugins(MaterialPlugin::<GlassMaterial>::default())
                .add_systems(PostUpdate, fade_glass_by_the_eye);
        }
    }
}

/// Whether a material is glass.
fn is_glass(m: &StandardMaterial) -> bool {
    m.alpha_mode == AlphaMode::Blend && (m.base_color.alpha() <= GLASS_ALPHA || m.base_color_texture.is_some())
}

/// Moves every glass surface onto the material that fades by the eye, each
/// glass material made once; a surface whose material has not loaded yet
/// is tried again the next frame.
fn fade_glass_by_the_eye(
    mut commands: Commands,
    added: Query<(Entity, &MeshMaterial3d<StandardMaterial>), Added<MeshMaterial3d<StandardMaterial>>>,
    mut waiting: Local<Vec<(Entity, Handle<StandardMaterial>)>>,
    mut made: Local<HashMap<AssetId<StandardMaterial>, Option<Handle<GlassMaterial>>>>,
    standard: Res<Assets<StandardMaterial>>,
    mut glass: ResMut<Assets<GlassMaterial>>,
) {
    waiting.extend(added.iter().map(|(entity, material)| (entity, material.0.clone())));
    waiting.retain(|(entity, material)| {
        let fading = match made.get(&material.id()) {
            Some(fading) => fading.clone(),
            None => {
                let Some(m) = standard.get(material) else {
                    return true;
                };
                let band = Vec4::new(CLEAR_M, SHOWN_M, 0.0, 0.0);
                let fading = is_glass(m).then(|| glass.add(GlassMaterial { base: m.clone(), extension: NearFade { band } }));
                made.insert(material.id(), fading.clone());
                fading
            }
        };
        if let Some(fading) = fading
            && let Ok(mut surface) = commands.get_entity(*entity)
        {
            surface.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(fading));
        }
        false
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glass_is_a_clear_blend_or_a_foil_and_nothing_else_is() {
        let blend = |alpha: f32| StandardMaterial { base_color: Color::srgba(0.7, 0.8, 0.8, alpha), alpha_mode: AlphaMode::Blend, ..default() };
        assert!(is_glass(&blend(0.13)), "laminated glass");
        assert!(is_glass(&StandardMaterial { base_color_texture: Some(Handle::default()), ..blend(1.0) }), "foil");
        assert!(!is_glass(&blend(0.85)), "a cloudy culture");
        assert!(!is_glass(&StandardMaterial { base_color: Color::srgba(0.7, 0.8, 0.8, 0.13), ..default() }), "an opaque surface");
    }
}
