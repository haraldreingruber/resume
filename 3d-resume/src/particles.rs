//! GPU particles for the intro. A compute pass moves them towards goals
//! sampled inside the name's glyphs (`shaders/particles_sim.wgsl`); the
//! render pass draws them as soft additive dots straight from the same
//! buffer (`shaders/particles.wgsl`). The choreography lives in `intro.rs`.

use std::f32::consts::TAU;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::gpu::Context;
use crate::intro::Step;
use crate::scene::Title;
use crate::text::{self, GlyphInstance};

/// Particles in the name (fewer only if the title were tiny).
pub const COUNT: usize = 16_384;
/// Radius around the pointer that pushes particles away (world units).
pub const POINTER_RADIUS: f32 = 0.35;
/// Dot radius (world units; the title is 0.42 units per em). Goals keep this
/// far inside the letters, so settled dots hide behind the crisp title.
const RADIUS: f32 = 0.011;
const WORKGROUP: u32 = 64;
/// Fixed, so every run and every screenshot starts from the same particles.
const SEED: u64 = 0x5EED_2026;

/// Matches `Particle` in the shaders; also the instance vertex layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Particle {
    /// w: brightness (0..1).
    position: [f32; 4],
    velocity: [f32; 4],
}

/// Matches `Home` in `shaders/particles_sim.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Home {
    goal: [f32; 4],
    /// Offset at full scatter.
    scatter: [f32; 4],
}

/// Matches `Sim` in the shaders.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Sim {
    /// xyz: title center, w: dot radius.
    center: [f32; 4],
    /// xyz: pointer, w: push radius (0 = no pointer).
    pointer: [f32; 4],
    /// Time, substep, substeps, scatter.
    params: [f32; 4],
}

pub struct Particles {
    count: u32,
    center: Vec3,
    particles: wgpu::Buffer,
    sim: wgpu::Buffer,
    simulate: wgpu::ComputePipeline,
    simulate_group: wgpu::BindGroup,
    draw: wgpu::RenderPipeline,
    draw_group: wgpu::BindGroup,
    /// False once fully scattered (nothing to draw).
    visible: bool,
}

impl Particles {
    /// `scene_layout`: the bind group layout of the world layer (group 0 of
    /// the draw pipeline). `None` without title glyphs.
    pub fn new(
        ctx: &Context,
        scene_layout: &wgpu::BindGroupLayout,
        title: &Title,
        atlas: &[u8],
    ) -> Option<Self> {
        let mut rng = Rng(SEED);
        let goals = sample_title(&title.glyphs, atlas, COUNT, RADIUS, &mut rng);
        if goals.is_empty() {
            return None;
        }
        let center = title.center();
        let (initial, homes) = initial_state(&goals, center, &mut rng);
        let device = &ctx.device;
        let particles = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particles"),
            contents: bytemuck::cast_slice(&initial),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
        });
        let homes = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particle homes"),
            contents: bytemuck::cast_slice(&homes),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let sim = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle sim"),
            size: size_of::<Sim>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let uniform = wgpu::BufferBindingType::Uniform;
        let storage = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let compute = wgpu::ShaderStages::COMPUTE;
        let simulate_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle sim"),
            entries: &[
                entry(0, compute, uniform),
                entry(1, compute, storage(false)),
                entry(2, compute, storage(true)),
            ],
        });
        let simulate_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle sim"),
            layout: &simulate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: sim.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: particles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: homes.as_entire_binding(),
                },
            ],
        });
        let simulate_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/particles_sim.wgsl"));
        let simulate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle sim"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("particle sim"),
                    bind_group_layouts: &[Some(&simulate_layout)],
                    immediate_size: 0,
                }),
            ),
            module: &simulate_shader,
            entry_point: Some("simulate"),
            compilation_options: Default::default(),
            cache: None,
        });

        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle draw"),
            entries: &[entry(0, wgpu::ShaderStages::VERTEX, uniform)],
        });
        let draw_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle draw"),
            layout: &draw_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: sim.as_entire_binding(),
            }],
        });
        let draw_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/particles.wgsl"));
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            // Leave the frame's alpha alone.
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let draw = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle draw"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("particle draw"),
                    bind_group_layouts: &[Some(scene_layout), Some(&draw_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &draw_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Particle>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &draw_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.view_format,
                    blend: Some(additive),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        Some(Self {
            count: initial.len() as u32,
            center,
            particles,
            sim,
            simulate,
            simulate_group,
            draw,
            draw_group,
            visible: true,
        })
    }

    /// Runs one frame of simulation.
    pub fn step(&mut self, ctx: &Context, step: &Step) {
        let (pointer, radius) = step
            .pointer
            .map_or((Vec3::ZERO, 0.0), |p| (p, POINTER_RADIUS));
        let sim = Sim {
            center: self.center.extend(RADIUS).to_array(),
            pointer: pointer.extend(radius).to_array(),
            params: [step.time, step.substep, step.substeps as f32, step.scatter],
        };
        ctx.queue
            .write_buffer(&self.sim, 0, bytemuck::bytes_of(&sim));
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.simulate);
            pass.set_bind_group(0, &self.simulate_group, &[]);
            pass.dispatch_workgroups(self.count.div_ceil(WORKGROUP), 1, 1);
        }
        ctx.queue.submit([encoder.finish()]);
        self.visible = step.scatter < 1.0;
    }

    /// Draws the particles; expects the world layer's bind group at group 0.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if !self.visible {
            return;
        }
        pass.set_pipeline(&self.draw);
        pass.set_bind_group(1, &self.draw_group, &[]);
        pass.set_vertex_buffer(0, self.particles.slice(..));
        pass.draw(0..4, 0..self.count);
    }
}

/// Where the particles start (a flat, circling band left and right of the
/// title), and where they go: their goal on the name, and their offset once
/// scattered (outwards and along the flight into the screen).
fn initial_state(goals: &[Vec3], center: Vec3, rng: &mut Rng) -> (Vec<Particle>, Vec<Home>) {
    goals
        .iter()
        .map(|goal| {
            let angle = rng.range(0.0, TAU);
            let radius = rng.range(3.0, 6.5);
            let (sin, cos) = angle.sin_cos();
            let start = center + Vec3::new(cos * radius, sin * radius * 0.3, rng.range(-2.5, 1.0));
            let swirl = Vec3::new(-sin, cos * 0.3, 0.0) * rng.range(1.0, 2.5);
            let brightness = rng.range(0.45, 1.0);
            let away = rng.range(0.0, TAU);
            let scatter = Vec3::new(
                away.cos() * rng.range(1.5, 4.0),
                away.sin() * rng.range(1.0, 2.5),
                -rng.range(0.5, 4.0),
            );
            let particle = Particle {
                position: start.extend(brightness).to_array(),
                velocity: swirl.extend(0.0).to_array(),
            };
            let home = Home {
                goal: goal.extend(0.0).to_array(),
                scatter: scatter.extend(0.0).to_array(),
            };
            (particle, home)
        })
        .unzip()
}

/// Up to `count` points inside the title's glyphs, in their plane: random
/// points in the glyph quads (glyphs picked in proportion to their area),
/// kept where the atlas's distance field says they are at least `inset`
/// (world units) inside a letter.
fn sample_title(
    title: &[GlyphInstance],
    atlas: &[u8],
    count: usize,
    inset: f32,
    rng: &mut Rng,
) -> Vec<Vec3> {
    let [width, height] = text::ATLAS_SIZE;
    let areas: Vec<f32> = title
        .iter()
        .map(|g| (g.rect[2] - g.rect[0]) * (g.rect[3] - g.rect[1]))
        .collect();
    let total: f32 = areas.iter().sum();
    let mut points = Vec::with_capacity(count);
    if total <= 0.0 {
        return points;
    }
    for _ in 0..count * 64 {
        if points.len() == count {
            break;
        }
        let mut pick = rng.unit() * total;
        let glyph = title
            .iter()
            .zip(&areas)
            .find(|&(_, &area)| {
                pick -= area;
                pick < 0.0
            })
            .map_or(&title[title.len() - 1], |(glyph, _)| glyph);
        let (u, v) = (rng.unit(), rng.unit());
        // Quad v runs up; atlas rows run down (uv: left, top, right, bottom).
        let [left, top, right, bottom] = glyph.uv;
        let x = ((left + (right - left) * u) * width as f32) as usize;
        let y = ((bottom + (top - bottom) * v) * height as f32) as usize;
        let texel = (y.min(height as usize - 1) * width as usize + x.min(width as usize - 1)) * 4;
        let [r, g, b] = [atlas[texel], atlas[texel + 1], atlas[texel + 2]];
        // Distance inside the letter, in atlas pixels (as in text.wgsl).
        let distance = (median(r, g, b) as f32 / 255.0 - 0.5) * text::DISTANCE_RANGE_PX;
        let [x0, y0, x1, y1] = glyph.rect;
        let atlas_px_per_unit = (right - left) * width as f32 / (x1 - x0);
        if distance >= inset * atlas_px_per_unit {
            points.push(Vec3::new(x0 + (x1 - x0) * u, y0 + (y1 - y0) * v, glyph.z));
        }
    }
    points
}

/// The signed distance an MSDF texel encodes (above 127: inside).
fn median(r: u8, g: u8, b: u8) -> u8 {
    r.min(g).max(r.max(g).min(b))
}

/// Deterministic pseudo-random numbers (SplitMix64).
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;

    #[test]
    fn samples_points_inside_the_title_deterministically() {
        let scene = Scene::new(&crate::content::resume());
        let atlas = text::atlas_rgba();
        let title = &scene.title.glyphs;
        let points = sample_title(title, &atlas, 4096, RADIUS, &mut Rng(SEED));
        assert_eq!(points.len(), 4096);
        for p in &points {
            assert!(
                title.iter().any(|g| {
                    let [x0, y0, x1, y1] = g.rect;
                    (x0..=x1).contains(&p.x) && (y0..=y1).contains(&p.y) && p.z == g.z
                }),
                "{p} is outside the title"
            );
        }
        let again = sample_title(title, &atlas, 4096, RADIUS, &mut Rng(SEED));
        assert_eq!(points, again);
        // Spread over the whole name, not clumped in one glyph.
        let [x0, _, x1, _] = scene.title.bounds;
        let (min, max) = points.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p.x), hi.max(p.x))
        });
        assert!(min < x0 + 0.1 * (x1 - x0) && max > x1 - 0.1 * (x1 - x0));
    }

    #[test]
    fn random_numbers_stay_in_range() {
        let mut rng = Rng(1);
        for _ in 0..10_000 {
            let x = rng.unit();
            assert!((0.0..1.0).contains(&x));
            assert!((2.0..3.0).contains(&rng.range(2.0, 3.0)));
        }
    }
}
