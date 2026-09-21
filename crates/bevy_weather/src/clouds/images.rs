use super::uniforms::CloudsImage;
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
};

pub(crate) const IMAGE_SIZE: u32 = 512;
/// Texels along a side of the ground shadow map.
pub(crate) const GROUND_SHADOW_SIZE: u32 = 768;
/// Texels across and down the weather map of the whole planet.
pub(crate) const GLOBE_WEATHER_SIZE: UVec2 = UVec2::new(2048, 1024);

pub(crate) fn cloud_image(size: Extent3d, dimension: TextureDimension) -> Image {
    filled_image(size, dimension, &[0, 0, 0, 0, 0, 0, 0, 0x3c])
}

/// The shadow map is the sun's light texture and is used *tiled*: the land
/// past its edge repeats the shadows rather than losing its sun. A tiled
/// texture has to be sampled tiled too — left clamped, every repeat is a seam
/// where the filter runs off the edge, and those seams draw a grid one map
/// wide across the ground and through the air the sun lights, in perspective,
/// as if the sky had a ceiling.
fn repeating_image(size: Extent3d, dimension: TextureDimension, texel: &[u8; 8]) -> Image {
    let mut image = filled_image(size, dimension, texel);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
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
        ground_shadow_image: images.add(repeating_image(
            Extent3d {
                width: GROUND_SHADOW_SIZE,
                height: GROUND_SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0, 0x3c, 0, 0, 0, 0, 0, 0x3c],
        )),
        globe_weather_image: images.add(cloud_image(
            Extent3d {
                width: GLOBE_WEATHER_SIZE.x,
                height: GLOBE_WEATHER_SIZE.y,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
        )),
    }
}
