// Intro particles, simulation: each particle springs towards its goal on the
// name (shifted by the scroll scatter), circles the title early in the
// assembly, and is pushed away by the pointer. Particles don't interact, so
// every invocation runs all of a frame's substeps for its particle.

// Matches `Sim` in particles.rs and particles.wgsl.
struct Sim {
    // xyz: center of the title, w: dot radius (for drawing).
    center: vec4<f32>,
    // xyz: pointer on the title's plane, w: push radius (0 = no pointer).
    pointer: vec4<f32>,
    // x: time at the frame start, y: substep, z: substeps, w: scatter (0..1).
    params: vec4<f32>,
}

// Matches `Particle` in particles.rs; position.w is the brightness.
struct Particle {
    position: vec4<f32>,
    velocity: vec4<f32>,
}

// Matches `Home` in particles.rs.
struct Home {
    goal: vec4<f32>,
    // Offset at full scatter.
    scatter: vec4<f32>,
}

@group(0) @binding(0) var<uniform> sim: Sim;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(2) var<storage, read> homes: array<Home>;

// Stiffness of the spring towards the goal once assembled (1/s²).
const PULL: f32 = 40.0;
const PUSH: f32 = 160.0;

@compute @workgroup_size(64)
fn simulate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&particles) {
        return;
    }
    var p = particles[i].position.xyz;
    var v = particles[i].velocity.xyz;
    let home = homes[i];
    let goal = home.goal.xyz + home.scatter.xyz * pow(sim.params.w, 1.5);
    let h = sim.params.y;
    let steps = u32(sim.params.z);
    for (var s = 0u; s < steps; s += 1u) {
        let t = sim.params.x + f32(s) * h;
        // The pull grows during the assembly; until then the particles
        // mostly circle the title. Slightly under-damped, so they overshoot
        // a little as they settle.
        let pull = 0.6 + PULL * smoothstep(0.1, 1.3, t);
        var a = (goal - p) * pull - v * (1.7 * sqrt(pull));
        let r = p.xy - sim.center.xy;
        a += vec3<f32>(-r.y, r.x, 0.0) * 1.6 * (1.0 - smoothstep(0.2, 1.1, t));
        if sim.pointer.w > 0.0 {
            let d = p.xy - sim.pointer.xy;
            let dist = length(d);
            if dist < sim.pointer.w && dist > 1e-4 {
                let push = 1.0 - dist / sim.pointer.w;
                a += vec3<f32>(d / dist, 0.0) * PUSH * push * push;
            }
        }
        // Semi-implicit Euler.
        v += a * h;
        p += v * h;
    }
    particles[i].position = vec4<f32>(p, particles[i].position.w);
    particles[i].velocity = vec4<f32>(v, 0.0);
}
