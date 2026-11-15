// Ground height on the tile mesh's two triangles per cell (split from +x to
// +z), slope bilinear; texels hold (height, normal.x, normal.z), rows along +z.
fn ground_at(heightmap: texture_2d<f32>, origin: vec2<f32>, texels_per_metre: f32,
    texel_count: f32, world_xz: vec2<f32>) -> vec3<f32> {
    let t = (world_xz - origin) * texels_per_metre;
    let max_index = i32(texel_count) - 1;
    let i = clamp(vec2<i32>(floor(t)), vec2<i32>(0), vec2<i32>(max_index - 1));
    let f = clamp(t - vec2<f32>(i), vec2<f32>(0.0), vec2<f32>(1.0));
    let s00 = textureLoad(heightmap, i, 0).xyz;
    let s10 = textureLoad(heightmap, i + vec2<i32>(1, 0), 0).xyz;
    let s01 = textureLoad(heightmap, i + vec2<i32>(0, 1), 0).xyz;
    let s11 = textureLoad(heightmap, i + vec2<i32>(1, 1), 0).xyz;
    let slope = mix(mix(s00.yz, s10.yz, f.x), mix(s01.yz, s11.yz, f.x), f.y);
    let near = s00.x + f.x * (s10.x - s00.x) + f.y * (s01.x - s00.x);
    let far = s11.x + (1.0 - f.x) * (s01.x - s11.x) + (1.0 - f.y) * (s10.x - s11.x);
    return vec3<f32>(select(far, near, f.x + f.y <= 1.0), slope);
}
