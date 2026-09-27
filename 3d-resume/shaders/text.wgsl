// Multi-channel signed distance field (MSDF) text, one instanced quad per glyph.

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // Camera distances: near fade start/end, far fade start/end.
    fade: vec4<f32>,
    // x: MSDF distance range in atlas pixels.
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Glyph {
    // World-space quad: x0, y0 (bottom), x1, y1 (top).
    @location(0) rect: vec4<f32>,
    // Atlas UVs: left, top, right, bottom.
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) z: f32,
}

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, glyph: Glyph) -> VertexOutput {
    // Triangle strip corners: (0,0) (1,0) (0,1) (1,1), y up.
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let world = vec3<f32>(mix(glyph.rect.xy, glyph.rect.zw, corner), glyph.z);
    var out: VertexOutput;
    out.clip = globals.view_proj * vec4<f32>(world, 1.0);
    out.uv = vec2<f32>(mix(glyph.uv.x, glyph.uv.z, corner.x), mix(glyph.uv.w, glyph.uv.y, corner.y));
    out.color = glyph.color;
    out.world = world;
    return out;
}

fn median(v: vec3<f32>) -> f32 {
    return max(min(v.r, v.g), min(max(v.r, v.g), v.b));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let distance_sample = median(textureSample(atlas, atlas_sampler, in.uv).rgb);
    // Distance range expressed in screen pixels keeps edges crisp at any scale.
    let unit_range = vec2<f32>(globals.params.x) / vec2<f32>(textureDimensions(atlas, 0));
    let screen_texture_size = vec2<f32>(1.0) / fwidth(in.uv);
    let screen_px_range = max(0.5 * dot(unit_range, screen_texture_size), 1.0);
    let coverage = clamp(screen_px_range * (distance_sample - 0.5) + 0.5, 0.0, 1.0);

    let camera_distance = distance(in.world, globals.camera_pos.xyz);
    let near = smoothstep(globals.fade.x, globals.fade.y, camera_distance);
    let far = 1.0 - smoothstep(globals.fade.z, globals.fade.w, camera_distance);
    let alpha = in.color.a * coverage * near * far;
    if alpha < 0.002 {
        discard;
    }
    // Premultiplied alpha.
    return vec4<f32>(in.color.rgb * alpha, alpha);
}
