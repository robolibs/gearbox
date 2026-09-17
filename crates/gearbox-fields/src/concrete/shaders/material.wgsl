// Cast concrete yard: 10 m slabs of scanned concrete (Poly Haven, CC0).
// Every slab is its own pour — the scan is turned and shifted per slab and
// the seams fall in the joints — and mossy concrete creeps in from the
// joints where the yard has gone to seed.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::lights,
}

#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::{SurfaceGeometryParams, surface_geometry_normal}
#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels}
#import "embedded://gearbox_fields/concrete/shaders/yard.wgsl"::{SLAB_M, JOINT_M, slab_rand, yard_noise, slab_cell, joint_distances, yard_weedy}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var clean_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var yard_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var clean_normal: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var clean_arm: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var trample: texture_2d<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var<uniform> trample_params: WheelMapParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var surface_heightmap: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var<uniform> geometry: SurfaceGeometryParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var moss_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var moss_normal: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var moss_arm: texture_2d<f32>;

// Ground the scans cover: the clean floor is 3 m across, the mossy one 2 m.
const CLEAN_M: f32 = 3.0;
const MOSS_M: f32 = 2.0;

// A vector turned by `quarter` right angles.
fn turn(v: vec2<f32>, quarter: u32) -> vec2<f32> {
    switch (quarter & 3u) {
        case 1u: { return vec2<f32>(-v.y, v.x); }
        case 2u: { return -v; }
        case 3u: { return vec2<f32>(v.y, -v.x); }
        default: { return v; }
    }
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let world_xz = in.world_position.xz;
    let ground = surface_geometry_normal(surface_heightmap, geometry, world_xz, in.world_normal);

    // This slab's pour: the scan turned and shifted, gradients kept
    // continuous so mips do not seam at the slab edge.
    let cell = slab_cell(world_xz);
    let quarter = u32(slab_rand(cell, 1u) * 4.0);
    let shift = vec2<f32>(slab_rand(cell, 2u), slab_rand(cell, 3u));
    let local = turn(world_xz - (vec2<f32>(cell) + vec2<f32>(0.5)) * SLAB_M, quarter);
    // Once a pixel spans several texels, read one mip coarser and flatten the
    // relief: sub-pixel bumps lit by a raking sun are grain, not texture.
    let footprint = max(length(dpdx(world_xz)), length(dpdy(world_xz)));
    let coarse = smoothstep(0.003, 0.012, footprint);
    let ddx = turn(dpdx(world_xz), quarter) * mix(1.0, 1.8, coarse);
    let ddy = turn(dpdy(world_xz), quarter) * mix(1.0, 1.8, coarse);
    var sun_height = 1.0;
    if (lights.n_directional_lights > 0u) {
        sun_height = max(lights.directional_lights[0].direction_to_light.y, 0.0);
    }
    let relief_strength = mix(1.0, 0.3, smoothstep(0.003, 0.02, footprint))
        * mix(0.45, 1.0, smoothstep(0.1, 0.6, sun_height));
    let uv = local / CLEAN_M + shift;
    // A second, larger reading of the same scan breaks the repeat inside
    // one slab.
    let uv_wide = local / (CLEAN_M * 2.3) + shift.yx;
    let wide = smoothstep(0.3, 0.7, yard_noise(world_xz * 0.45));
    var albedo = mix(textureSampleGrad(clean_albedo, yard_sampler, uv, ddx / CLEAN_M, ddy / CLEAN_M).rgb,
        textureSampleGrad(clean_albedo, yard_sampler, uv_wide, ddx / (CLEAN_M * 2.3), ddy / (CLEAN_M * 2.3)).rgb, wide * 0.6);
    var bump = textureSampleGrad(clean_normal, yard_sampler, uv, ddx / CLEAN_M, ddy / CLEAN_M).xyz * 2.0 - 1.0;
    var arm = textureSampleGrad(clean_arm, yard_sampler, uv, ddx / CLEAN_M, ddy / CLEAN_M).rgb;

    // Joints: the open gap, and how near this point is to one.
    let reach = joint_distances(world_xz);
    let to_joint = min(reach.x, reach.y);
    let soft = max(fwidth(to_joint), 0.002);
    let gap = 1.0 - smoothstep(JOINT_M * 0.5 - soft, JOINT_M * 0.5 + soft, to_joint);

    // Moss and staining where the yard is weedy: patches on the slab,
    // thickest along the joints it spreads from.
    let weedy = yard_weedy(world_xz);
    let patches = smoothstep(0.45, 0.75, yard_noise(world_xz * 0.6 + vec2<f32>(5.0, 11.0)));
    let creep = (1.0 - smoothstep(0.0, 1.6, to_joint)) * smoothstep(0.25, 0.6, yard_noise(world_xz * 1.7));
    let moss = clamp(weedy * max(patches * 0.8, creep), 0.0, 1.0);
    if (moss > 0.004) {
        let uv_moss = local / MOSS_M + shift.yx * 3.0;
        albedo = mix(albedo, textureSampleGrad(moss_albedo, yard_sampler, uv_moss, ddx / MOSS_M, ddy / MOSS_M).rgb, moss);
        bump = mix(bump, textureSampleGrad(moss_normal, yard_sampler, uv_moss, ddx / MOSS_M, ddy / MOSS_M).xyz * 2.0 - 1.0, moss);
        arm = mix(arm, textureSampleGrad(moss_arm, yard_sampler, uv_moss, ddx / MOSS_M, ddy / MOSS_M).rgb, moss);
    }

    // No two pours cure to the same grey, and dirt gathers along an edge.
    let pour = slab_rand(cell, 4u);
    albedo *= mix(0.86, 1.06, pour) * mix(vec3<f32>(1.0, 0.99, 0.965), vec3<f32>(0.975, 0.99, 1.0), slab_rand(cell, 5u));
    albedo *= 1.0 - (1.0 - smoothstep(0.0, 0.3, to_joint)) * mix(0.1, 0.28, weedy);

    // Half of the slabs carry a saw-cut control joint down the middle.
    let middle = abs(select(local.x, local.y, slab_rand(cell, 6u) > 0.5));
    let cut_soft = max(fwidth(middle), 0.001);
    let cut = (1.0 - smoothstep(0.004 - cut_soft, 0.004 + cut_soft, middle)) * step(0.5, slab_rand(cell, 7u));
    albedo *= 1.0 - cut * 0.55;

    // The gap itself: dark sealant where kept, soil where weeds root.
    let fill = mix(vec3<f32>(0.035, 0.035, 0.036), vec3<f32>(0.075, 0.058, 0.04), weedy)
        * mix(0.7, 1.2, yard_noise(world_xz * 23.0));
    albedo = mix(albedo, fill, gap);

    // Tyres leave a faint dark polish rather than a rut.
    let pressed = sample_wheels(trample, trample_params, world_xz).x;
    albedo *= 1.0 - trample_params.darkening * pressed;

    // Scan relief back in world space, each slab settled a touch out of
    // level, its edge chamfered into the joint.
    let tangent = normalize(vec3<f32>(1.0, 0.0, 0.0) - ground * ground.x);
    let bitangent = cross(tangent, ground);
    let relief = turn(bump.xy, (4u - (quarter & 3u)) & 3u);
    let settle = (vec2<f32>(slab_rand(cell, 8u), slab_rand(cell, 9u)) - vec2<f32>(0.5)) * 0.02;
    let toward = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), reach.x < reach.y)
        * sign(fract(world_xz / SLAB_M) - vec2<f32>(0.5));
    let chamfer = (1.0 - smoothstep(JOINT_M * 0.5, JOINT_M * 0.5 + 0.03, to_joint)) * (1.0 - gap);
    let lean = (relief * relief_strength + settle) * (1.0 - gap) + toward * chamfer * 0.7;
    let normal = normalize(tangent * lean.x + bitangent * lean.y + ground * max(bump.z, 0.2));

    pbr_input.material.base_color = alpha_discard(pbr_input.material, vec4<f32>(albedo, 1.0));
    pbr_input.material.perceptual_roughness = mix(clamp(arm.g, 0.3, 1.0), 0.97, gap);
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3<f32>(0.04);
    pbr_input.diffuse_occlusion = vec3<f32>(mix(arm.r, 0.45, gap));
    pbr_input.specular_occlusion = mix(arm.r, 0.2, gap);
    pbr_input.N = normal;
    pbr_input.world_normal = ground;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
