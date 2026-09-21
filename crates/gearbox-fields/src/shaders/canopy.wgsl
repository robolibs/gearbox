#import bevy_pbr::mesh_view_bindings::{view, globals}
#import "embedded://gearbox_fields/shaders/wind.wgsl"::{plant_lean}
#import "embedded://gearbox_fields/shaders/interaction.wgsl"::wheel_roll

struct CanopyVertex {
    position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec3<f32>,
}

fn canopy_vertex(local: vec3<f32>, ground: vec3<f32>, normal: vec3<f32>, seed: f32,
    distance: f32, alive: f32, pressed: vec3<f32>, bend: f32, straw: bool, wind: vec4<f32>) -> CanopyVertex {
    let direction = view.world_position.xz - ground.xz;
    let yaw = seed * 6.2831853 + (local.z + 1.0) * 1.5707963;
    let right = vec3<f32>(cos(yaw), 0.0, sin(yaw));
    let grazing = smoothstep(0.4, 0.9, length(direction) / max(distance, 0.001));
    let growth = smoothstep(8.0, 20.0, distance) * alive * select(1.0, grazing, straw);
    let width = mix(0.10, 0.17, seed) * mix(1.0, 1.5, smoothstep(20.0, 96.0, distance));
    let height = select(mix(0.065, 0.105, seed), mix(0.08, 0.14, seed), straw);
    let flat = clamp(pressed.x, 0.0, 1.0);
    let forward = vec3<f32>(-right.z, 0.0, right.x);
    let arch = select(0.65, 0.0, straw) * local.y * local.y * height * (1.0 - flat * bend);
    let lateral = right * local.x * width * 0.5 + forward * arch;
    let ground_offset = -dot(normal.xz, lateral.xz) / max(normal.y, 0.1);
    let lean = plant_lean(ground.xz, globals.time, wind, seed, local.y) * select(0.9, 0.09, straw);
    let tip = local.y * height * growth * (1.0 - select(0.2, 0.0, straw) * local.y);
    let position = ground + lateral * growth + vec3<f32>(0.0, ground_offset * growth + tip * (1.0 - flat * bend), 0.0)
        + (wheel_roll(pressed) * flat + lean * (1.0 - flat)) * tip * bend;
    return CanopyVertex(position, normal, vec3<f32>(local.x * 0.5 + 0.5, local.y, seed - local.z));
}

fn canopy_alpha(uv: vec3<f32>, straw: bool) -> f32 {
    let x = uv.x * 6.0;
    let blade = floor(x);
    let random = fract(sin(blade * 37.7 + uv.z * 91.3) * 4375.3);
    let top = mix(0.55, 1.0, random);
    let width = select(0.36 * (1.0 - uv.y / top), 0.24, straw);
    let side = abs(fract(x) - 0.5 + select(sin(uv.y * 2.0 + random) * uv.y * 0.15, 0.0, straw));
    let dx = max(fwidth(x), 0.001);
    let dy = max(fwidth(uv.y), 0.001);
    let shape = (1.0 - smoothstep(width - dx * 0.5, width + dx * 0.5, side))
        * (1.0 - smoothstep(top - dy * 0.5, top + dy * 0.5, uv.y));
    let mean = select(0.36 * pow(1.0 - uv.y, 1.4), 0.48 * (1.0 - smoothstep(0.55, 1.0, uv.y)), straw);
    return mix(shape, mean, smoothstep(0.5, 1.5, dx));
}
