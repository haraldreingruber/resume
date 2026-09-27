// Intro particles, drawing: soft additive dots in the accent blue, whiter
// while fast, fading out as they scatter. Reads the particles as instance
// data straight from the buffer the simulation writes.

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // Camera distances: near fade start/end, far fade start/end.
    fade: vec4<f32>,
    // x: MSDF distance range in atlas pixels, y: 1 = fade by camera distance.
    params: vec4<f32>,
}

// Matches `Sim` in particles.rs and particles_sim.wgsl.
struct Sim {
    // w: dot radius (world units).
    center: vec4<f32>,
    pointer: vec4<f32>,
    // w: scatter (0..1).
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> sim: Sim;

// The accent #6DB3E8 in linear RGB.
const BLUE: vec3<f32> = vec3<f32>(0.161, 0.463, 0.813);

struct Particle {
    // w: brightness.
    @location(0) position: vec4<f32>,
    @location(1) velocity: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    // -1..1 across the dot.
    @location(0) local: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) color: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, particle: Particle) -> VertexOutput {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u)) * 2.0 - 1.0;
    let world = particle.position.xyz + vec3<f32>(corner * sim.center.w, 0.0);
    let speed = length(particle.velocity.xyz);
    let visible = 1.0 - smoothstep(0.55, 0.95, sim.params.w);
    var out: VertexOutput;
    out.clip = globals.view_proj * vec4<f32>(world, 1.0);
    out.local = corner;
    out.world = world;
    out.color = vec4<f32>(
        mix(BLUE, vec3<f32>(1.0), clamp(speed * 0.08, 0.0, 0.7)),
        particle.position.w * visible * 0.55,
    );
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let falloff = 1.0 - smoothstep(0.3, 1.0, length(in.local));
    let camera_distance = distance(in.world, globals.camera_pos.xyz);
    let near = smoothstep(globals.fade.x, globals.fade.y, camera_distance);
    let far = 1.0 - smoothstep(globals.fade.z, globals.fade.w, camera_distance);
    let alpha = in.color.a * falloff * near * far;
    if alpha < 0.002 {
        discard;
    }
    // Additive (the pipeline blends One + One).
    return vec4<f32>(in.color.rgb * alpha, alpha);
}
