struct CloudPosition { x: f32, y: f32, z: f32 };

// One point's colour words, their colour kind and whether they are valid.
struct CloudSample { kind: u32, valid: u32, words: vec2<u32> };

// A page's first validity word when every point is valid.
const CLOUD_ALL_VALID: u32 = 0xffffffffu;

// Value words: ramp position in 8.12 fixed point (bits 0-19), raw palette byte (20-27),
// Okabe slot (28-30), below zero (31).
fn cloud_value_position(word: u32) -> f32 {
    return f32(word & 0xfffffu) / 4096.0;
}

fn cloud_value_indices(word: u32) -> u32 {
    let raw = (word >> 20u) & 255u;
    let map = select(raw, 255u, (word >> 31u) != 0u);
    return map | (raw << 8u) | (((word >> 28u) & 7u) << 16u);
}

struct CloudAge {
    anchor: vec4<u32>,
    mapping: vec4<f32>,
    palette: vec4<u32>,
};

fn cloud_sub64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    return vec2(a.x - b.x, a.y - b.y - select(0u, 1u, a.x < b.x));
}

fn cloud_age_coordinate(offset: vec2<u32>, age: CloudAge) -> f32 {
    if age.anchor.z == 0u || all(offset == age.anchor.xy) { return age.mapping.x; }
    let before = offset.y < age.anchor.y || (offset.y == age.anchor.y && offset.x < age.anchor.x);
    if age.anchor.z == 2u { return select(0.0, 1.0, before); }
    let distance = cloud_sub64(select(offset, age.anchor.xy, before), select(age.anchor.xy, offset, before));
    let magnitude = f32(distance.y) * 4294967296.0 + f32(distance.x);
    return clamp(age.mapping.x + select(-magnitude, magnitude, before) * age.mapping.y, 0.0, 1.0);
}

fn cloud_age_palette(offset: vec2<u32>, age: CloudAge) -> u32 {
    let parts = cloud_seconds_parts(offset);
    let borrow = select(0u, 1u, parts.z > age.palette.z);
    let low = parts.x + borrow;
    var rounded = cloud_sub64(age.palette.xy, vec2(low, parts.y + select(0u, 1u, low < parts.x)));
    let negative_half = (age.palette.w & 8u) != 0u && parts.z == age.palette.z &&
        ((rounded.y & 0x80000000u) != 0u || all(rounded == vec2(0u)));
    let tie = select(0u, 1u, negative_half);
    rounded = cloud_sub64(rounded, vec2(tie, 0u));
    let negative = (rounded.y & 0x80000000u) != 0u;
    let raw = select(select(min(rounded.x, 255u), 255u, rounded.y != 0u), 0u, negative);
    let map = select(raw, 255u, negative);
    let okabe = ((age.palette.w & 7u) + 16u - (parts.x & 7u) - borrow - tie) & 7u;
    return map | (raw << 8u) | (okabe << 16u);
}

fn cloud_quad_corner(vertex: u32) -> vec2<f32> {
    let corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0),
        vec2(1.0, 1.0), vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    return corners[vertex];
}

const CLOUD_MAX_FINITE: f32 = 3.4028234663852886e38;

struct CloudMagnitude { fraction: f32, exponent: i32 };

fn cloud_magnitude(value: f32) -> CloudMagnitude {
    let bits = bitcast<u32>(value) & 0x7fffffffu;
    if bits == 0u { return CloudMagnitude(0.0, 0); }
    if bits < 0x00800000u {
        let parts = frexp(f32(bits));
        return CloudMagnitude(parts.fract, parts.exp - 149);
    }
    return CloudMagnitude(bitcast<f32>((bits & 0x007fffffu) | 0x3f000000u), i32(bits >> 23u) - 126);
}

fn cloud_product_magnitude(a: f32, b: f32, c: f32, divisor: f32) -> CloudMagnitude {
    if bitcast<u32>(a) == 0u || bitcast<u32>(b) == 0u || bitcast<u32>(c) == 0u { return CloudMagnitude(0.0, 0); }
    let aa = cloud_magnitude(a);
    let bb = cloud_magnitude(b);
    let cc = cloud_magnitude(c);
    let dd = cloud_magnitude(divisor);
    let product = frexp(aa.fraction * bb.fraction * cc.fraction / dd.fraction);
    let exponent = aa.exponent + bb.exponent + cc.exponent - dd.exponent + product.exp;
    return CloudMagnitude(product.fract, exponent);
}

fn cloud_magnitude_value(value: CloudMagnitude) -> f32 {
    if value.fraction == 0.0 || value.exponent < -149 { return 0.0; }
    if value.exponent > 128 { return CLOUD_MAX_FINITE; }
    return ldexp(value.fraction, value.exponent);
}

fn cloud_positive_product_ratio(a: f32, b: f32, c: f32, divisor: f32) -> f32 {
    return cloud_magnitude_value(cloud_product_magnitude(a, b, c, divisor));
}

fn cloud_radius_px(metric_radius: f32, pixel_radius: f32, projection_y: f32,
    viewport_height: f32, clip_w: f32) -> f32 {
    let bits = bitcast<u32>(metric_radius);
    if bits == 0u || (bits & 0x80000000u) != 0u { return pixel_radius; }
    let projection = abs(projection_y);
    let w = max(abs(clip_w), 1e-8);
    if metric_radius >= 1e-20 && metric_radius <= 1e20 && projection >= 1e-10 &&
        projection <= 1e4 && viewport_height >= 1.0 && viewport_height <= 32768.0 && w <= 1e20 {
        return metric_radius * 0.5 * viewport_height * projection / w;
    }
    return cloud_positive_product_ratio(metric_radius, viewport_height * 0.5, projection, w);
}

struct CloudProjection { position: vec4<f32>, offset_px: vec2<f32>, radius_px: f32, linear_edge: u32 };

fn cloud_sign(value: vec2<f32>) -> vec2<f32> {
    return select(vec2(1.0), vec2(-1.0), (bitcast<vec2<u32>>(value) & vec2(0x80000000u)) != vec2(0u));
}

fn cloud_project(center: vec4<f32>, viewport: vec2<f32>, radius_px: f32, vertex: u32) -> CloudProjection {
    var out: CloudProjection;
    out.position = vec4(2.0, 2.0, 2.0, 1.0);
    if center.w <= 0.0 || center.z < 0.0 || center.z > center.w || any(viewport <= vec2(0.0)) ||
        any((bitcast<vec4<u32>>(center) & vec4(0x7f800000u)) == vec4(0x7f800000u)) { return out; }
    let half_view = viewport * 0.5;
    var center_px: vec2<f32>;
    if all(abs(center.xy) <= vec2(1e20)) && center.w >= 1e-8 {
        center_px = center.xy / center.w * half_view;
    } else {
        center_px = cloud_sign(center.xy) * vec2(
            cloud_positive_product_ratio(abs(center.x), half_view.x, 1.0, center.w),
            cloud_positive_product_ratio(abs(center.y), half_view.y, 1.0, center.w));
    }
    let extent = vec2(max(radius_px + 1.0, 1.0));
    let low = clamp(center_px - min(extent, max(center_px + half_view, vec2(0.0))), -half_view, half_view);
    let high = clamp(center_px + min(extent, max(half_view - center_px, vec2(0.0))), -half_view, half_view);
    let pixel = select(low, high, cloud_quad_corner(vertex) > vec2(0.0));
    var depth = 0.0;
    if center.w >= 1e-20 && center.w <= 1e20 {
        depth = center.z / center.w;
    } else {
        depth = cloud_positive_product_ratio(center.z, 1.0, 1.0, center.w);
    }
    out.position = vec4(pixel / half_view, depth, 1.0);
    out.offset_px = pixel - center_px;
    out.radius_px = radius_px;
    return out;
}

fn cloud_scaled_magnitude(value: CloudMagnitude, exponent: i32) -> f32 {
    return cloud_magnitude_value(CloudMagnitude(value.fraction, value.exponent - exponent));
}

fn cloud_window_coordinate(value: f32, half_extent: f32) -> f32 {
    if value == 0.0 { return 0.0; }
    if value >= half_extent { return 1.0; }
    if value <= -half_extent { return -1.0; }
    if half_extent < 1e-20 {
        return cloud_sign(vec2(value)).x * cloud_positive_product_ratio(abs(value), 1.0, 1.0, half_extent);
    }
    return value / half_extent;
}

fn cloud_project_point(center: vec4<f32>, viewport: vec2<f32>, metric: f32,
    pixels: f32, projection_y: f32, vertex: u32) -> CloudProjection {
    let radius = cloud_radius_px(metric, pixels, projection_y, viewport.y, center.w);
    var out = cloud_project(center, viewport, radius, vertex);
    if out.position.z > 1.0 { return out; }
    let half_view = viewport * 0.5;
    let projected_center = out.position.xy * half_view - out.offset_px;
    if radius < 1e18 && all(abs(projected_center) < vec2(1e18)) { return out; }
    var rr = cloud_magnitude(pixels);
    let bits = bitcast<u32>(metric);
    if bits != 0u && (bits & 0x80000000u) == 0u {
        rr = cloud_product_magnitude(metric, half_view.y, abs(projection_y), max(center.w, 1e-8));
    }
    let xx = cloud_product_magnitude(abs(center.x), half_view.x, 1.0, center.w);
    let yy = cloud_product_magnitude(abs(center.y), half_view.y, 1.0, center.w);
    let hx = cloud_magnitude(half_view.x);
    let hy = cloud_magnitude(half_view.y);
    let exponent = max(max(rr.exponent, max(xx.exponent, yy.exponent)), max(hx.exponent, hy.exponent));
    let r = cloud_scaled_magnitude(rr, exponent);
    let c = cloud_sign(center.xy) * vec2(cloud_scaled_magnitude(xx, exponent), cloud_scaled_magnitude(yy, exponent));
    let half_scaled = vec2(cloud_scaled_magnitude(hx, exponent), cloud_scaled_magnitude(hy, exponent));
    let extent = r + cloud_scaled_magnitude(cloud_magnitude(1.0), exponent);
    let bound = c + cloud_quad_corner(vertex) * extent;
    let ndc = vec2(cloud_window_coordinate(bound.x, half_scaled.x), cloud_window_coordinate(bound.y, half_scaled.y));
    let distance = length(c);
    let delta = r - distance;
    let parts = cloud_magnitude(abs(delta));
    var edge = sign(delta) * cloud_magnitude_value(CloudMagnitude(parts.fraction, parts.exponent + exponent));
    if abs(edge) < max(viewport.x, viewport.y) * 2.0 && distance > 0.0 {
        edge += dot(c / distance, ndc * half_view);
    }
    out.position = vec4(ndc, out.position.zw);
    out.offset_px = vec2(edge, 0.0);
    out.linear_edge = 1u;
    return out;
}

fn cloud_edge_distance(offset_px: vec2<f32>, radius_px: f32) -> f32 {
    let scale = max(radius_px, max(abs(offset_px.x), abs(offset_px.y)));
    if scale < 1e18 { return radius_px - length(offset_px); }
    let radius = cloud_positive_product_ratio(radius_px, 1.0, 1.0, scale);
    let offset = vec2(cloud_positive_product_ratio(abs(offset_px.x), 1.0, 1.0, scale),
        cloud_positive_product_ratio(abs(offset_px.y), 1.0, 1.0, scale));
    return clamp(radius - length(offset), -1.0, 1.0) * scale;
}

fn cloud_coverage(offset_px: vec2<f32>, radius_px: f32) -> f32 {
    return clamp(cloud_edge_distance(offset_px, radius_px) + 0.5, 0.0, 1.0);
}

fn cloud_projected_edge(offset_px: vec2<f32>, radius_px: f32, linear_edge: u32) -> f32 {
    if linear_edge != 0u { return offset_px.x; }
    return cloud_edge_distance(offset_px, radius_px);
}

fn cloud_clip_position(rotation: mat4x4<f32>, point: vec3<f32>) -> vec3<f32> {
    return fma(rotation[0].xyz, vec3(point.x), fma(rotation[1].xyz, vec3(point.y), rotation[2].xyz * point.z));
}

fn cloud_clip_accept(rotation: mat4x4<f32>, low: vec4<f32>, high: vec4<f32>, point: vec3<f32>) -> bool {
    if low.w == 0.0 { return true; }
    if low.w == 2.0 { return false; }
    let value = cloud_clip_position(rotation, point);
    let inside = all(value >= low.xyz) && all(value <= high.xyz);
    return inside != (low.w == 3.0);
}

fn cloud_source_age_parts(now: vec2<u32>, capture: vec2<u32>, offset: array<u32, 2>) -> vec3<u32> {
    let low = capture.x + offset[0];
    let sample = vec2(low, capture.y + offset[1] + select(0u, 1u, low < capture.x));
    let future = now.y < sample.y || (now.y == sample.y && now.x < sample.x);
    let high_value = select(now, sample, future);
    let low_value = select(sample, now, future);
    let difference_low = high_value.x - low_value.x;
    let difference_high = high_value.y - low_value.y - select(0u, 1u, high_value.x < low_value.x);
    return vec3(difference_low, difference_high, select(0u, 1u, future));
}

fn cloud_source_age_ns(now: vec2<u32>, capture: vec2<u32>, offset: array<u32, 2>) -> f32 {
    let parts = cloud_source_age_parts(now, capture, offset);
    let magnitude = f32(parts.y) * 4294967296.0 + f32(parts.x);
    return select(magnitude, -magnitude, parts.z != 0u);
}

fn cloud_seconds_parts(value: vec2<u32>) -> vec3<u32> {
    var whole = value.x / 1000000000u;
    var remainder = value.x % 1000000000u;
    if value.y != 0u {
        whole = 0u;
        remainder = value.y % 1000000000u;
        for (var bit = 32u; bit > 0u; bit -= 1u) {
            remainder = remainder * 2u + ((value.x >> (bit - 1u)) & 1u);
            let carry = remainder >= 1000000000u;
            remainder = select(remainder, remainder - 1000000000u, carry);
            whole = (whole << 1u) | select(0u, 1u, carry);
        }
    }
    return vec3(whole, value.y / 1000000000u, remainder);
}

fn cloud_seconds(value: vec2<u32>) -> f32 {
    let parts = cloud_seconds_parts(value);
    let fraction = f32(parts.z / 1953125u) * 0.001953125 + f32(parts.z % 1953125u) / 1000000000.0;
    return f32(parts.y) * 4294967296.0 + f32(parts.x) + fraction;
}

fn cloud_source_age_seconds(now: vec2<u32>, capture: vec2<u32>, offset: array<u32, 2>) -> f32 {
    let parts = cloud_source_age_parts(now, capture, offset);
    let magnitude = cloud_seconds(parts.xy);
    return select(magnitude, -magnitude, parts.z != 0u);
}
