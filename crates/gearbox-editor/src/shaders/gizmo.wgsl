// transform-gizmo-style overlay shader.
//
// Input positions are already in Normalized Device Coordinates (NDC)
// — the CPU projected world-space gizmo points into screen space and
// packed them here directly. No view/proj matrices are applied.
//
// This matches the 2D painter vibe of urholaukkarinen/transform-gizmo:
// filled flat shapes that ignore perspective on the gizmo geometry
// itself, even though they sit inside a 3D scene.
//
// The companion Material (`gizmo::GizmoOverlayMaterial`) disables
// depth testing and enables alpha blending so these overlays always
// sit on top of world geometry.

struct VertexInput {
    @location(0) position: vec3<f32>,   // x,y in NDC [-1,1]; z in depth [0,1]
    @location(1) color:    vec4<f32>,   // per-vertex RGBA, linear-space
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color:               vec4<f32>,
};

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position.x, in.position.y, in.position.z, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
