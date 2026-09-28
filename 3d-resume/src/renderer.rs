//! Draws a frame: gradient background, the scene's lines (skill map) and
//! shapes, the intro particles and the MSDF glyphs, then the screen-space
//! layer (native buttons) on top. Draws into any texture view: the window surface or an
//! offscreen screenshot target.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::gpu::Context;
use crate::intro::Step;
use crate::lines::LineInstance;
use crate::particles::Particles;
use crate::scene::{
    Camera, FAR_FADE, MAP_DIM, MAX_GROUPS, NEAR_FADE, Relations, Scene, TITLE_GROUP,
};
use crate::shapes::ShapeInstance;
use crate::text::{self, GlyphInstance};
use crate::ui::UiLayer;

/// Matches `Globals` in `shaders/text.wgsl` and `shaders/shapes.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    /// Camera distances: near fade start/end, far fade start/end.
    fade: [f32; 4],
    /// x: MSDF distance range in atlas pixels, y: 1 = fade by camera distance.
    params: [f32; 4],
}

/// Instances drawn with one set of globals (projection, fading).
struct Layer {
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    glyphs: Instances,
    shapes: Instances,
    lines: Instances,
}

/// An instance buffer, `None` when empty (wgpu buffers can't be empty).
struct Instances(Option<(wgpu::Buffer, u32)>);

impl Instances {
    fn new<T: Pod>(device: &wgpu::Device, label: &str, items: &[T]) -> Self {
        Self((!items.is_empty()).then(|| {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(items),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (buffer, items.len() as u32)
        }))
    }

    /// Four vertices (a triangle strip quad) per instance.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some((buffer, count)) = &self.0 {
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..4, 0..*count);
        }
    }
}

pub struct Renderer {
    background: wgpu::RenderPipeline,
    text: wgpu::RenderPipeline,
    shapes: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    /// Per hover group: `x` = highlight, `y` = keyboard focus (focus ring),
    /// `z` = fade-out (the title while the particles form it; skill-map
    /// nodes unrelated to the active one), `w` = the skill map's active node.
    groups: wgpu::Buffer,
    group_state: GroupState,
    world: Layer,
    ui: Layer,
    /// The intro particles; `None` with reduced motion.
    particles: Option<Particles>,
}

/// What `groups` holds.
struct GroupState {
    hovered: Option<u32>,
    focused: Option<u32>,
    title_opacity: f32,
    relations: Option<Relations>,
}

impl Renderer {
    /// `particles`: whether to set up the intro particles.
    pub fn new(ctx: &Context, scene: &Scene, particles: bool) -> Self {
        let device = &ctx.device;
        let atlas_pixels = text::atlas_rgba();
        let atlas = atlas_texture(ctx, &atlas_pixels).create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let groups = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("group state"),
            contents: bytemuck::cast_slice(&[[0.0f32; 4]; MAX_GROUPS]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                uniform(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform(3),
            ],
        });
        let layer = |label: &str,
                     glyphs: &[GlyphInstance],
                     shapes: &[ShapeInstance],
                     lines: &[LineInstance]| {
            let globals = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size_of::<Globals>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: globals.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&atlas),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: groups.as_entire_binding(),
                    },
                ],
            });
            Layer {
                globals,
                bind_group,
                glyphs: Instances::new(device, "glyphs", glyphs),
                shapes: Instances::new(device, "shapes", shapes),
                lines: Instances::new(device, "lines", lines),
            }
        };
        let world = layer("world", &scene.glyphs, &scene.shapes, &scene.lines);
        let ui = layer("ui", &[], &[], &[]);

        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let text_shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/text.wgsl"));
        let text = instanced_pipeline(
            ctx,
            "text",
            &scene_layout,
            &text_shader,
            size_of::<GlyphInstance>(),
            &wgpu::vertex_attr_array![
                0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32, 4 => Uint32
            ],
        );
        let shapes_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/shapes.wgsl"));
        let shapes = instanced_pipeline(
            ctx,
            "shapes",
            &scene_layout,
            &shapes_shader,
            size_of::<ShapeInstance>(),
            &wgpu::vertex_attr_array![
                0 => Float32x4, 1 => Float32x4, 2 => Float32, 3 => Float32, 4 => Float32, 5 => Uint32
            ],
        );
        let lines_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/lines.wgsl"));
        let lines = instanced_pipeline(
            ctx,
            "lines",
            &scene_layout,
            &lines_shader,
            size_of::<LineInstance>(),
            &wgpu::vertex_attr_array![
                0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32x2
            ],
        );

        let background_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("background"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let background_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/background.wgsl"));
        let background = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("background"),
            layout: Some(&background_layout),
            vertex: wgpu::VertexState {
                module: &background_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &background_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(ctx.view_format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let particles = particles
            .then(|| Particles::new(ctx, &layout, &scene.title, &atlas_pixels))
            .flatten();

        Self {
            background,
            text,
            shapes,
            lines,
            groups,
            group_state: GroupState {
                hovered: None,
                focused: None,
                title_opacity: 1.0,
                relations: None,
            },
            world,
            ui,
            particles,
        }
    }

    /// Replaces the screen-space layer (e.g. after a resize).
    pub fn set_ui(&mut self, ctx: &Context, ui: &UiLayer) {
        self.ui.glyphs = Instances::new(&ctx.device, "ui glyphs", &ui.glyphs);
        self.ui.shapes = Instances::new(&ctx.device, "ui shapes", &ui.shapes);
    }

    /// Highlights the hovered and the keyboard-focused group (a link or
    /// button), shows the focused one's focus ring, and the skill map's
    /// `relations` (if a map node is active).
    pub fn set_groups(
        &mut self,
        ctx: &Context,
        hovered: Option<u32>,
        focused: Option<u32>,
        relations: Option<Relations>,
    ) {
        self.group_state.hovered = hovered;
        self.group_state.focused = focused;
        self.group_state.relations = relations;
        self.write_groups(ctx);
    }

    /// Fades the crisp title on the intro (while the particles form it).
    pub fn set_title_opacity(&mut self, ctx: &Context, opacity: f32) {
        if opacity != self.group_state.title_opacity {
            self.group_state.title_opacity = opacity;
            self.write_groups(ctx);
        }
    }

    fn write_groups(&self, ctx: &Context) {
        let GroupState {
            hovered,
            focused,
            title_opacity,
            ref relations,
        } = self.group_state;
        let mut state = [[0.0f32; 4]; MAX_GROUPS];
        if let Some(Relations {
            active,
            related,
            nodes,
        }) = relations
        {
            for group in nodes.clone().filter(|&g| (g as usize) < MAX_GROUPS) {
                let entry = &mut state[group as usize];
                if group == *active || related.contains(&group) {
                    entry[0] = 1.0;
                } else {
                    entry[2] = MAP_DIM;
                }
            }
            if let Some(entry) = state.get_mut(*active as usize) {
                entry[3] = 1.0;
            }
        }
        for (group, focus) in [(hovered, 0.0), (focused, 1.0)] {
            if let Some(entry) = group
                .filter(|&g| g != 0)
                .and_then(|g| state.get_mut(g as usize))
            {
                entry[0] = 1.0;
                entry[1] = f32::max(entry[1], focus);
            }
        }
        state[TITLE_GROUP as usize][2] = 1.0 - title_opacity;
        ctx.queue
            .write_buffer(&self.groups, 0, bytemuck::cast_slice(&state));
    }

    /// Runs one frame of the intro particles' simulation.
    pub fn step_particles(&mut self, ctx: &Context, step: &Step) {
        if let Some(particles) = &mut self.particles {
            particles.step(ctx, step);
        }
    }

    /// Draws a frame into `target` (a view in `ctx.view_format`).
    pub fn draw(
        &self,
        ctx: &Context,
        target: &wgpu::TextureView,
        camera: &Camera,
        ui_projection: Mat4,
    ) {
        let focus = camera.focus_distance;
        let world = Globals {
            view_proj: camera.view_proj.to_cols_array_2d(),
            camera_pos: camera.eye.extend(1.0).to_array(),
            fade: [
                focus + NEAR_FADE.0,
                focus + NEAR_FADE.1,
                focus + FAR_FADE.0,
                focus + FAR_FADE.1,
            ],
            params: [text::DISTANCE_RANGE_PX, 1.0, 0.0, 0.0],
        };
        let ui = Globals {
            view_proj: ui_projection.to_cols_array_2d(),
            params: [text::DISTANCE_RANGE_PX, 0.0, 0.0, 0.0],
            ..world
        };
        ctx.queue
            .write_buffer(&self.world.globals, 0, bytemuck::bytes_of(&world));
        ctx.queue
            .write_buffer(&self.ui.globals, 0, bytemuck::bytes_of(&ui));

        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.background);
            pass.draw(0..3, 0..1);

            for (layer, world) in [(&self.world, true), (&self.ui, false)] {
                pass.set_bind_group(0, &layer.bind_group, &[]);
                pass.set_pipeline(&self.lines);
                layer.lines.draw(&mut pass);
                pass.set_pipeline(&self.shapes);
                layer.shapes.draw(&mut pass);
                // Under the text, so the settled particles hide behind the
                // crisp title.
                if world && let Some(particles) = &self.particles {
                    particles.draw(&mut pass);
                }
                pass.set_pipeline(&self.text);
                layer.glyphs.draw(&mut pass);
            }
        }
        ctx.queue.submit([encoder.finish()]);
    }
}

/// A pipeline drawing one premultiplied-alpha quad (triangle strip) per instance.
fn instanced_pipeline(
    ctx: &Context,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    stride: usize,
    attributes: &[wgpu::VertexAttribute],
) -> wgpu::RenderPipeline {
    ctx.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: stride as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.view_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
        })
}

/// Uploads the baked MSDF atlas (RGBA8, linear data).
fn atlas_texture(ctx: &Context, pixels: &[u8]) -> wgpu::Texture {
    let [width, height] = text::ATLAS_SIZE;

    ctx.device.create_texture_with_data(
        &ctx.queue,
        &wgpu::TextureDescriptor {
            label: Some("msdf atlas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        pixels,
    )
}

#[cfg(test)]
mod tests {
    /// Every shader parses and validates. A broken shader otherwise only
    /// shows when the app creates its pipelines on a GPU.
    #[test]
    fn shaders_are_valid() {
        let shaders = [
            (
                "background.wgsl",
                include_str!("../shaders/background.wgsl"),
            ),
            ("text.wgsl", include_str!("../shaders/text.wgsl")),
            ("shapes.wgsl", include_str!("../shaders/shapes.wgsl")),
            ("lines.wgsl", include_str!("../shaders/lines.wgsl")),
            ("particles.wgsl", include_str!("../shaders/particles.wgsl")),
            (
                "particles_sim.wgsl",
                include_str!("../shaders/particles_sim.wgsl"),
            ),
        ];
        for (name, source) in shaders {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::default(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }
}
