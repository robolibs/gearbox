import gearbox::draw::draw_cloud_types::{CloudSample, CloudAge, cloud_age_coordinate, cloud_age_palette, cloud_value_position, cloud_value_indices};

struct CloudStyle {
    constant: vec4<f32>,
    missing: vec4<f32>,
    controls: vec4<u32>,
    effects: vec4<f32>,
    ramp: array<vec4<f32>, 256>,
    categories: array<vec4<u32>, 64>,
};
@group(3) @binding(0) var<uniform> style: CloudStyle;

fn cloud_ramp_color(at: f32) -> vec4<f32> {
    let low = u32(floor(at));
    return mix(style.ramp[low], style.ramp[min(low + 1u, 255u)], fract(at));
}

fn cloud_map_color(value: f32, indices: u32) -> vec4<f32> {
    if style.controls.y != 0u {
        let index = (indices >> ((style.controls.y - 1u) * 8u)) & 255u;
        return style.ramp[index];
    }
    return cloud_ramp_color(clamp(value, 0.0, 1.0) * 255.0);
}

fn cloud_sample_color(sample: CloudSample, age: CloudAge) -> vec4<f32> {
    if style.controls.x == 0u { return style.constant; }
    if sample.kind != style.controls.x { return select(style.missing, vec4(0.0), style.controls.x == 1u); }
    switch style.controls.x {
        case 1u: { return unpack4x8unorm(sample.words.x); }
        case 2u: {
            if sample.valid == 0u { return style.missing; }
            if style.controls.y != 0u { return cloud_map_color(0.0, cloud_value_indices(sample.words.x)); }
            return cloud_ramp_color(cloud_value_position(sample.words.x));
        }
        case 3u: {
            if sample.valid != 0u {
                for (var index = 0u; index < style.controls.z; index += 1u) {
                    if style.categories[index].x == sample.words.x { return unpack4x8unorm(style.categories[index].y); }
                }
            }
            return style.missing;
        }
        case 4u: {
            if sample.valid == 0u { return style.missing; }
            if style.controls.y != 0u { return cloud_map_color(0.0, cloud_age_palette(sample.words, age)); }
            return cloud_map_color(cloud_age_coordinate(sample.words, age), 0u);
        }
        default: { return style.missing; }
    }
}
