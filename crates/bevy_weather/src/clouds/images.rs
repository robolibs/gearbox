use super::uniforms::CloudsImage;
use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
};

pub(crate) const IMAGE_SIZE: u32 = 512;
/// Texels along a side of the ground shadow map.
pub(crate) const GROUND_SHADOW_SIZE: u32 = 512;

pub(crate) fn cloud_image(size: Extent3d, dimension: TextureDimension) -> Image {
    filled_image(size, dimension, &[0, 0, 0, 0, 0, 0, 0, 0x3c])
}

// `texel` is one RGBA pixel of half floats; 0x3c00 is 1.0.
fn filled_image(size: Extent3d, dimension: TextureDimension, texel: &[u8; 8]) -> Image {
    let mut image = Image::new_fill(
        size,
        dimension,
        texel,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage = TextureUsages::COPY_SRC
        | TextureUsages::COPY_DST
        | TextureUsages::STORAGE_BINDING
        | TextureUsages::TEXTURE_BINDING;
    image.sampler = ImageSampler::linear();
    image
}

pub(crate) fn build_images(images: &mut Assets<Image>, resolution: UVec2) -> CloudsImage {
    let output = Extent3d {
        width: resolution.x,
        height: resolution.y,
        depth_or_array_layers: 1,
    };
    CloudsImage {
        cloud_render_image: images.add(cloud_image(output, TextureDimension::D2)),
        history_image: images.add(cloud_image(output, TextureDimension::D2)),
        sky_image: images.add(cloud_image(output, TextureDimension::D2)),
        cloud_atlas_image: images.add(cloud_image(
            Extent3d {
                width: IMAGE_SIZE,
                height: IMAGE_SIZE,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
        )),
        cloud_worley_image: images.add(cloud_image(
            Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 32,
            },
            TextureDimension::D3,
        )),
        // Full sun until the pass first writes it: a sun that waited on its
        // cloud shadows would leave the land dark whenever they were late.
        ground_shadow_image: images.add(filled_image(
            Extent3d {
                width: GROUND_SHADOW_SIZE,
                height: GROUND_SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0, 0x3c, 0, 0, 0, 0, 0, 0x3c],
        )),
    }
}
