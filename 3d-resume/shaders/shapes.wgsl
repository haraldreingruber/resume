// Instanced rounded rectangles (filled or outlined), anti-aliased via a signed
// distance field. Shares globals, fading and hover highlighting with text.wgsl.

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // Camera distances: near fade start/end, far fade start/end.
    fade: vec4<f32>,
    // x: MSDF distance range in atlas pixels, y: 1 = fade by camera distance.
    params: vec4<f32>,
}

struct Groups {
    // Per group (group 0 is never highlighted): x = highlight (0..1),
    // y = keyboard focus (shows the group's focus rings).
    state: array<vec4<f32>, 64>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(3) var<uniform> groups: Groups;

// A shape whose group has this bit set is a focus ring: drawn only while its
// group has keyboard focus. Matches `shapes::FOCUS_RING`.
const FOCUS_RING: u32 = 0x80000000u;

struct Shape {
    // x0, y0 (bottom), x1, y1 (top).
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) z: f32,
    @location(3) radius: f32,
    @location(4) border: f32,
    @location(5) group: u32,
}

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    // Position relative to the shape's center.
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) world: vec3<f32>,
    // x: radius, y: border.
    @location(4) style: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, shape: Shape) -> VertexOutput {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let half_size = (shape.rect.zw - shape.rect.xy) * 0.5;
    // Grow the quad by a little so the anti-aliased edge isn't clipped.
    let pad = max(half_size.x, half_size.y) * 0.1;
    let center = (shape.rect.xy + shape.rect.zw) * 0.5;
    let local = (corner * 2.0 - 1.0) * (half_size + pad);
    let world = vec3<f32>(center + local, shape.z);

    let state = groups.state[min(shape.group & ~FOCUS_RING, 63u)];
    var alpha = shape.color.a;
    if (shape.group & FOCUS_RING) != 0u {
        alpha *= state.y;
    }
    var out: VertexOutput;
    out.clip = globals.view_proj * vec4<f32>(world, 1.0);
    out.local = local;
    out.half_size = half_size;
    out.color = vec4<f32>(mix(shape.color.rgb, vec3<f32>(1.0), state.x * 0.35), alpha);
    out.world = world;
    out.style = vec2<f32>(min(shape.radius, min(half_size.x, half_size.y)), shape.border);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Rounded-box signed distance (negative inside).
    let radius = in.style.x;
    let q = abs(in.local) - in.half_size + radius;
    var d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    if in.style.y > 0.0 {
        d = abs(d + in.style.y * 0.5) - in.style.y * 0.5;
    }
    let aa = max(fwidth(d), 1e-5);
    let coverage = clamp(0.5 - d / aa, 0.0, 1.0);

    var fade = 1.0;
    if globals.params.y > 0.5 {
        let camera_distance = distance(in.world, globals.camera_pos.xyz);
        let near = smoothstep(globals.fade.x, globals.fade.y, camera_distance);
        let far = 1.0 - smoothstep(globals.fade.z, globals.fade.w, camera_distance);
        fade = near * far;
    }
    let alpha = in.color.a * coverage * fade;
    if alpha < 0.002 {
        discard;
    }
    // Premultiplied alpha.
    return vec4<f32>(in.color.rgb * alpha, alpha);
}
