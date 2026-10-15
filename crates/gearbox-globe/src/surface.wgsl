#define_import_path gearbox_globe::surface

fn geology_hash(cell: vec3<i32>) -> f32 {
    let c = bitcast<vec3<u32>>(cell);
    var h = (c.x * 1597334677u) ^ (c.y * 3812015801u) ^ (c.z * 2798796415u);
    h ^= h >> 16u;
    h *= 2246822519u;
    h ^= h >> 13u;
    h *= 3266489917u;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

fn geology_noise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f*f*(3.0-2.0*f);
    let x00 = mix(geology_hash(i), geology_hash(i+vec3(1,0,0)), u.x);
    let x10 = mix(geology_hash(i+vec3(0,1,0)), geology_hash(i+vec3(1,1,0)), u.x);
    let x01 = mix(geology_hash(i+vec3(0,0,1)), geology_hash(i+vec3(1,0,1)), u.x);
    let x11 = mix(geology_hash(i+vec3(0,1,1)), geology_hash(i+vec3(1,1,1)), u.x);
    return mix(mix(x00,x10,u.y),mix(x01,x11,u.y),u.z);
}

fn planet_albedo(dir: vec3<f32>, kind: u32) -> vec3<f32> {
    var colour = vec3<f32>(0.058, 0.108, 0.015);
    if kind == 1u {
        let province = geology_noise(dir*7.0 + vec3(1.7,2.1,5.3));
        let weathering = geology_noise(dir*41.0)*0.20 + geology_noise(dir*193.0)*0.08;
        let basalt = smoothstep(0.42,0.73,province+weathering);
        colour = mix(vec3<f32>(0.24,0.105,0.047),vec3<f32>(0.065,0.049,0.038),basalt);
        colour *= 0.85 + 0.3*geology_noise(dir*470.0);
        let ice = smoothstep(0.978,0.995,abs(dir.z)+0.006*geology_noise(dir*75.0));
        colour = mix(colour,vec3<f32>(0.65,0.63,0.59),ice);
    } else if kind == 2u {
        let province = geology_noise(dir*5.0+vec3(3.1,8.4,1.7));
        let detail = geology_noise(dir*31.0)*0.20 + geology_noise(dir*155.0)*0.08;
        let mare = smoothstep(0.51,0.68,province+detail);
        colour = mix(vec3<f32>(0.16,0.155,0.15),vec3<f32>(0.045,0.044,0.043),mare);
        colour *= 0.8+0.4*geology_noise(dir*370.0);
    }
    return colour;
}
