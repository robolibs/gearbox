fn deform_contact(point: vec3<f32>, envelope: vec4<f32>, ground: vec4<f32>) -> vec3<f32> {
    let radius = envelope.x;
    let bead = envelope.y;
    let width = envelope.z;
    var out = point;
    if radius > 0.0 && bead > 0.0 && bead < radius && width > 0.0 {
        let loaded = clamp(envelope.w, bead, radius);
        let compression = radius - loaded;
        let radial = length(point.xy);
        if compression > 0.0 && radial > bead && point.y < 0.0 {
            let sidewall = clamp((radial - bead) / (radius - bead), 0.0, 1.0);
            let shoulder = sin(3.141592653589793 * sidewall);
            let downward = clamp(-point.y / radial, 0.0, 1.0);
            let lower_sector = downward * downward * downward * downward;
            let lateral = clamp(point.z / (0.25 * width), -1.0, 1.0);
            let radial_weight = sidewall * sidewall * (3.0 - 2.0 * sidewall);
            let tread_lift = max(radius * downward - loaded, 0.0);
            out.y = max(point.y + radial_weight * tread_lift, -loaded);
            out.z += 0.65 * compression * shoulder * lower_sector * lateral;
        }
    }
    if ground.y > 0.0 {
        out.y = max(out.y, -ground.x);
    }
    return out;
}

fn safe_normal(vector: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let squared = dot(vector, vector);
    if squared > 1e-16 { return vector * inverseSqrt(squared); }
    return fallback;
}
