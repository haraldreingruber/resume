// Full-screen vertical gradient with a soft vignette (colors are linear).

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // One triangle covering the screen.
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.clip = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let top = vec3<f32>(0.010, 0.030, 0.062);
    let bottom = vec3<f32>(0.001, 0.004, 0.010);
    var color = mix(bottom, top, in.uv.y);
    let d = distance(in.uv, vec2<f32>(0.5, 0.6));
    color *= 1.0 - 0.5 * d * d;
    return vec4<f32>(color, 1.0);
}
