// Bloom: the scene's bright parts, blurred over a chain of ever smaller
// textures, added back as a soft glow. All passes draw one full-screen
// triangle.
// - fs_prefilter: scene (HDR) -> first mip: downsample, keep what's bright.
// - fs_down: mip -> next smaller mip (dual-filter downsample).
// - fs_up: smaller mip -> larger one, added on top (tent upsample), so the
//   glow mixes several radii.
// - fs_composite: scene + glow -> the window's surface.

// Brightness (max channel, linear) where the glow starts, the width of the
// soft transition around it, and how strongly the glow is added back.
const THRESHOLD: f32 = 0.72;
const KNEE: f32 = 0.2;
const INTENSITY: f32 = 0.45;

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var bilinear: sampler;
// The scene (composite only; other passes bind it but don't read it).
@group(0) @binding(2) var scene: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // One triangle covering the screen; uv (0, 0) is the top-left.
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.clip = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

fn texel() -> vec2<f32> {
    return 1.0 / vec2<f32>(textureDimensions(source));
}

// Dual-filter downsample: the center and four diagonal neighbors.
fn down(uv: vec2<f32>) -> vec3<f32> {
    let t = texel();
    var sum = textureSampleLevel(source, bilinear, uv, 0.0).rgb * 4.0;
    sum += textureSampleLevel(source, bilinear, uv - t, 0.0).rgb;
    sum += textureSampleLevel(source, bilinear, uv + t, 0.0).rgb;
    sum += textureSampleLevel(source, bilinear, uv + vec2<f32>(t.x, -t.y), 0.0).rgb;
    sum += textureSampleLevel(source, bilinear, uv - vec2<f32>(t.x, -t.y), 0.0).rgb;
    return sum / 8.0;
}

@fragment
fn fs_prefilter(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = down(in.uv);
    let brightness = max(color.r, max(color.g, color.b));
    // Soft knee: a smooth ramp instead of a hard cut at the threshold.
    var soft = clamp(brightness - THRESHOLD + KNEE, 0.0, 2.0 * KNEE);
    soft = soft * soft / (4.0 * KNEE + 1e-4);
    let contribution = max(soft, brightness - THRESHOLD) / max(brightness, 1e-4);
    return vec4<f32>(color * contribution, 1.0);
}

@fragment
fn fs_down(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(down(in.uv), 1.0);
}

// Tent upsample from the smaller mip (`source`).
@fragment
fn fs_up(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = texel();
    let uv = in.uv;
    var sum = textureSampleLevel(source, bilinear, uv + vec2<f32>(-2.0 * t.x, 0.0), 0.0).rgb;
    sum += textureSampleLevel(source, bilinear, uv + vec2<f32>(2.0 * t.x, 0.0), 0.0).rgb;
    sum += textureSampleLevel(source, bilinear, uv + vec2<f32>(0.0, -2.0 * t.y), 0.0).rgb;
    sum += textureSampleLevel(source, bilinear, uv + vec2<f32>(0.0, 2.0 * t.y), 0.0).rgb;
    sum += textureSampleLevel(source, bilinear, uv + vec2<f32>(-t.x, -t.y), 0.0).rgb * 2.0;
    sum += textureSampleLevel(source, bilinear, uv + vec2<f32>(t.x, -t.y), 0.0).rgb * 2.0;
    sum += textureSampleLevel(source, bilinear, uv + vec2<f32>(-t.x, t.y), 0.0).rgb * 2.0;
    sum += textureSampleLevel(source, bilinear, uv + vec2<f32>(t.x, t.y), 0.0).rgb * 2.0;
    return vec4<f32>(sum / 12.0, 1.0);
}

@fragment
fn fs_composite(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(scene, bilinear, in.uv, 0.0).rgb;
    let glow = textureSampleLevel(source, bilinear, in.uv, 0.0).rgb;
    return vec4<f32>(color + glow * INTENSITY, 1.0);
}
