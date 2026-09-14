//! Linear-light mip chains for packaged field textures.

use bevy::{image::ImageSampler, prelude::*, render::render_resource::TextureFormat};

pub fn prepare_textures(
    mut events: MessageReader<AssetEvent<Image>>,
    server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    for event in events.read() {
        let AssetEvent::Added { id } = event else {
            continue;
        };
        let Some(path) = server.get_path(*id) else {
            continue;
        };
        if !path
            .to_string()
            .starts_with("embedded://gearbox_sim/fields/")
            || !path.path().to_string_lossy().contains("/textures/")
        {
            continue;
        }
        let Some(mut image) = images.get_mut(*id) else {
            continue;
        };
        add_mips(&mut image);
        info!("field texture {path}: {} mip levels", image.texture_descriptor.mip_level_count);
    }
}

fn add_mips(image: &mut Image) {
    let (channels, srgb) = match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb => (4, true),
        TextureFormat::Rgba8Unorm => (4, false),
        TextureFormat::R8Unorm => (1, false),
        TextureFormat::Rg8Unorm => (2, false),
        _ => return,
    };
    if image.texture_descriptor.mip_level_count != 1
        || image.texture_descriptor.size.depth_or_array_layers != 1
    {
        return;
    }
    let Some(data) = image.data.as_mut() else {
        return;
    };
    let mut width = image.texture_descriptor.size.width as usize;
    let mut height = image.texture_descriptor.size.height as usize;
    if width == 0 || height == 0 || data.len() != width * height * channels {
        return;
    }
    let linear: [f32; 256] = std::array::from_fn(|i| {
        let c = i as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    let mut offset = 0;
    while width > 1 || height > 1 {
        let next_width = (width / 2).max(1);
        let next_height = (height / 2).max(1);
        let next_offset = data.len();
        for y in 0..next_height {
            for x in 0..next_width {
                let x0 = x * width / next_width;
                let x1 = (x + 1) * width / next_width;
                let y0 = y * height / next_height;
                let y1 = (y + 1) * height / next_height;
                for channel in 0..channels {
                    let mut sum = 0.0;
                    for sy in y0..y1 {
                        for sx in x0..x1 {
                            let value = data[offset + (sy * width + sx) * channels + channel];
                            sum += if srgb && channel < 3 {
                                linear[value as usize]
                            } else {
                                value as f32 / 255.0
                            };
                        }
                    }
                    let mean = sum / ((x1 - x0) * (y1 - y0)) as f32;
                    let encoded = if srgb && channel < 3 {
                        if mean <= 0.0031308 {
                            mean * 12.92
                        } else {
                            1.055 * mean.powf(1.0 / 2.4) - 0.055
                        }
                    } else {
                        mean
                    };
                    data.push((encoded.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
        }
        offset = next_offset;
        width = next_width;
        height = next_height;
        image.texture_descriptor.mip_level_count += 1;
    }
    let mut sampler = bevy::image::ImageSamplerDescriptor::linear();
    sampler.anisotropy_clamp = 8;
    sampler.address_mode_u = bevy::image::ImageAddressMode::Repeat;
    sampler.address_mode_v = bevy::image::ImageAddressMode::Repeat;
    image.sampler = ImageSampler::Descriptor(sampler);
}
