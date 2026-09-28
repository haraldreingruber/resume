// Instanced line segments: the edges of the skill map. A segment is faint
// by default, lights up while one of its two nodes is the active one, and
// dims with the nodes when another node is active.

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // Camera distances: near fade start/end, far fade start/end.
    fade: vec4<f32>,
    // x: MSDF distance range in atlas pixels, y: 1 = fade by camera distance.
    params: vec4<f32>,
}

struct Groups {
    // Per group: x = highlight, y = keyboard focus, z = fade-out (dimmed),
    // w = the active node of the skill map.
    state: array<vec4<f32>, 64>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(3) var<uniform> groups: Groups;

struct Line {
    // xyz: start, w: width.
    @location(0) start: vec4<f32>,
    @location(1) end: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) groups: vec2<u32>,
}

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    // -1..1 across the line.
    @location(0) across: f32,
    @location(1) world: vec3<f32>,
    @location(2) color: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, line: Line) -> VertexOutput {
    // Triangle strip: along = 0/1, across = -1/+1.
    let along = f32(index >> 1u);
    let across = f32(index & 1u) * 2.0 - 1.0;
    let direction = line.end.xy - line.start.xy;
    let normal = normalize(vec2<f32>(-direction.y, direction.x));
    let center = mix(line.start.xyz, line.end.xyz, along);
    let world = center + vec3<f32>(normal * across * line.start.w * 0.5, 0.0);

    let a = groups.state[min(line.groups.x, 63u)];
    let b = groups.state[min(line.groups.y, 63u)];
    let lit = max(a.w, b.w);
    let dimmed = max(a.z, b.z) * (1.0 - lit);
    var out: VertexOutput;
    out.clip = globals.view_proj * vec4<f32>(world, 1.0);
    out.across = across;
    out.world = world;
    out.color = vec4<f32>(
        mix(line.color.rgb, vec3<f32>(1.0), lit * 0.3),
        min(line.color.a * (1.0 - dimmed) * (1.0 + 2.5 * lit), 1.0),
    );
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let coverage = 1.0 - smoothstep(0.4, 1.0, abs(in.across));
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
