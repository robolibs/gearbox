@if((DRAW_FROST) && (VIEW_TRANSMISSION_TEXTURE)) import bevy_pbr::render::mesh_view_bindings::{view_transmission_texture, view_transmission_sampler};

// Frost needs the view's transmission texture; without it the coat is returned as is.
fn background_frost(coat: vec4<f32>, pixel: vec2<f32>, strength: f32) -> vec4<f32> {
    @if((DRAW_FROST) && (VIEW_TRANSMISSION_TEXTURE))
    let size = vec2<f32>(textureDimensions(view_transmission_texture));
    @if((DRAW_FROST) && (VIEW_TRANSMISSION_TEXTURE))
    let amount = clamp(strength, 0.0, 1.0) * clamp(coat.a * 3.0, 0.0, 1.0) * (1.0 - coat.a);
    @if((DRAW_FROST) && (VIEW_TRANSMISSION_TEXTURE))
    if amount <= 0.0001 || min(size.x, size.y) <= 1.0 { return coat; }
    @if((DRAW_FROST) && (VIEW_TRANSMISSION_TEXTURE))
    let uv = pixel / size;
    @if((DRAW_FROST) && (VIEW_TRANSMISSION_TEXTURE))
    var transmitted = textureSampleLevel(view_transmission_texture, view_transmission_sampler, uv, 0.0).rgb * 0.25;
    @if((DRAW_FROST) && (VIEW_TRANSMISSION_TEXTURE))
    let offsets = array<vec2<f32>, 8>(vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, -1.0), vec2(0.0, 1.0),
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(1.0, 1.0));
    @if((DRAW_FROST) && (VIEW_TRANSMISSION_TEXTURE))
    for (var i = 0u; i < 8u; i += 1u) {
        let weight = select(0.0625, 0.125, i < 4u);
        transmitted += textureSampleLevel(view_transmission_texture, view_transmission_sampler,
            clamp(uv + offsets[i] * 5.0 / size, 0.5 / size, 1.0 - 0.5 / size), 0.0).rgb * weight;
    }
    @if((DRAW_FROST) && (VIEW_TRANSMISSION_TEXTURE))
    return vec4(coat.rgb + transmitted * amount, coat.a + amount);
    @if((!(DRAW_FROST)))
    return coat;
    @if((DRAW_FROST) && (!(VIEW_TRANSMISSION_TEXTURE)))
    return coat;
}
