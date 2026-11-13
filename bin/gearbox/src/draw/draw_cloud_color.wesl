import gearbox::draw::draw_cloud_types::{CloudSample, CloudAge, cloud_age_coordinate, cloud_age_palette, CLOUD_COLOR_VALID, CLOUD_VALUE_VALID, CLOUD_CATEGORY_VALID};

struct CloudStyle {
    constant: vec4<f32>,
    missing: vec4<f32>,
    controls: vec4<u32>,
    effects: vec4<f32>,
    ramp: array<vec4<f32>, 256>,
    categories: array<vec4<u32>, 64>,
};
@group(3) @binding(0) var<uniform> style: CloudStyle;

fn cloud_map_color(value: f32, indices: u32) -> vec4<f32> {
    if style.controls.y != 0u {
        let index = (indices >> ((style.controls.y - 1u) * 8u)) & 255u;
        return style.ramp[index];
    }
    let at = clamp(value, 0.0, 1.0) * 255.0;
    let low = u32(floor(at));
    return mix(style.ramp[low], style.ramp[min(low + 1u, 255u)], fract(at));
}

fn cloud_sample_color(sample: CloudSample, age: CloudAge) -> vec4<f32> {
    switch style.controls.x {
        case 0u: { return style.constant; }
        case 1u: {
            if (sample.flags & CLOUD_COLOR_VALID) == 0u { return vec4(0.0); }
            return unpack4x8unorm(sample.rgba);
        }
        case 2u: {
            if (sample.flags & CLOUD_VALUE_VALID) == 0u { return style.missing; }
            return cloud_map_color(sample.value, sample.palette_indices);
        }
        case 3u: {
            if (sample.flags & CLOUD_CATEGORY_VALID) != 0u {
                for (var index = 0u; index < style.controls.z; index += 1u) {
                    if style.categories[index].x == sample.category { return unpack4x8unorm(style.categories[index].y); }
                }
            }
            return style.missing;
        }
        case 4u: {
            if (sample.flags & CLOUD_VALUE_VALID) == 0u { return style.missing; }
            let offset = vec2(sample.acquisition_ns[0], sample.acquisition_ns[1]);
            if style.controls.y != 0u { return cloud_map_color(0.0, cloud_age_palette(offset, age)); }
            return cloud_map_color(cloud_age_coordinate(offset, age), 0u);
        }
        default: { return style.missing; }
    }
}
