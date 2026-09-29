//! Draws a frame: gradient background, the scene's lines (skill map) and
//! shapes, the intro particles and the MSDF glyphs, then the screen-space
//! layer (buttons, panels) on top. With bloom (where the GPU renders to
//! floating-point textures), the scene goes through an HDR texture and gets
//! a glow before the screen-space layer is drawn. Draws into any texture
//! view: the window surface or an offscreen screenshot target. With timing
//! on (performance overlay), it measures its passes on the GPU.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::bloom::{self, Bloom};
use crate::gpu::Context;
use crate::gpu_timer::{GpuTimer, Pass};
use crate::intro::Step;
use crate::lines::LineInstance;
use crate::particles::Particles;
use crate::scene::{
    Camera, FAR_FADE, MAP_DIM, MAX_GROUPS, NEAR_FADE, Relations, Scene, TITLE_GROUP,
};
use crate::shapes::ShapeInstance;
use crate::stats::{DrawCounts, GpuTimes};
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

    fn count(&self) -> u32 {
        self.0.as_ref().map_or(0, |(_, count)| *count)
    }

    fn bytes(&self) -> u64 {
        self.0.as_ref().map_or(0, |(buffer, _)| buffer.size())
    }

    /// Four vertices (a triangle strip quad) per instance.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some((buffer, count)) = &self.0 {
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..4, 0..*count);
        }
    }
}

/// The shader modules the layers draw with.
struct Shaders {
    text: wgpu::ShaderModule,
    shapes: wgpu::ShaderModule,
    lines: wgpu::ShaderModule,
    background: wgpu::ShaderModule,
}

/// The layers' pipelines for one target format.
struct Pipelines {
    background: wgpu::RenderPipeline,
    text: wgpu::RenderPipeline,
    shapes: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
}

impl Pipelines {
    fn new(
        ctx: &Context,
        layout: &wgpu::PipelineLayout,
        shaders: &Shaders,
        format: wgpu::TextureFormat,
    ) -> Self {
        let instanced = |label, shader, stride, attributes: &[wgpu::VertexAttribute]| {
            instanced_pipeline(ctx, label, layout, shader, stride, attributes, format)
        };
        let text = instanced(
            "text",
            &shaders.text,
            size_of::<GlyphInstance>(),
            &wgpu::vertex_attr_array![
                0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32, 4 => Uint32
            ],
        );
        let shapes = instanced(
            "shapes",
            &shaders.shapes,
            size_of::<ShapeInstance>(),
            &wgpu::vertex_attr_array![
                0 => Float32x4, 1 => Float32x4, 2 => Float32, 3 => Float32, 4 => Float32, 5 => Uint32
            ],
        );
        let lines = instanced(
            "lines",
            &shaders.lines,
            size_of::<LineInstance>(),
            &wgpu::vertex_attr_array![
                0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32x2
            ],
        );
        let background_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("background"),
                    bind_group_layouts: &[],
                    immediate_size: 0,
                });
        let background = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("background"),
                layout: Some(&background_layout),
                vertex: wgpu::VertexState {
                    module: &shaders.background,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shaders.background,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        Self {
            background,
            text,
            shapes,
            lines,
        }
    }

    /// A layer's lines, shapes, the particles (if given) and glyphs.
    fn draw_layer(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        layer: &Layer,
        particles: Option<&Particles>,
    ) {
        pass.set_bind_group(0, &layer.bind_group, &[]);
        pass.set_pipeline(&self.lines);
        layer.lines.draw(pass);
        pass.set_pipeline(&self.shapes);
        layer.shapes.draw(pass);
        // Under the text, so the settled particles hide behind the crisp
        // title.
        if let Some(particles) = particles {
            particles.draw(pass);
        }
        pass.set_pipeline(&self.text);
        layer.glyphs.draw(pass);
    }
}

pub struct Renderer {
    /// The scene's pipelines (in `scene_format`), and the screen-space
    /// layer's when that differs (with bloom).
    scene_pipelines: Pipelines,
    ui_pipelines: Option<Pipelines>,
    scene_format: wgpu::TextureFormat,
    bloom: Option<Bloom>,
    /// Per hover group: `x` = highlight, `y` = keyboard focus (focus ring),
    /// `z` = fade-out (the title while the particles form it; skill-map
    /// nodes unrelated to the active one), `w` = the skill map's active node.
    groups: wgpu::Buffer,
    group_state: GroupState,
    world: Layer,
    ui: Layer,
    /// The intro particles; `None` with reduced motion.
    particles: Option<Particles>,
    /// Layout of the layers' bind groups (for re-creating the particles).
    bind_layout: wgpu::BindGroupLayout,
    /// GPU timing for the performance overlay, while it's shown (and the
    /// GPU has timestamp queries).
    timer: Option<GpuTimer>,
    /// Whether bloom adds its glow (switchable in the performance overlay).
    glow: bool,
    /// The x-ray view's outlines over the world (performance overlay).
    xray: Instances,
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
        let shaders = Shaders {
            text: device.create_shader_module(wgpu::include_wgsl!("../shaders/text.wgsl")),
            shapes: device.create_shader_module(wgpu::include_wgsl!("../shaders/shapes.wgsl")),
            lines: device.create_shader_module(wgpu::include_wgsl!("../shaders/lines.wgsl")),
            background: device
                .create_shader_module(wgpu::include_wgsl!("../shaders/background.wgsl")),
        };
        // With bloom, the scene renders in HDR and the screen-space layer
        // on top of the result, in the window's format.
        let bloom = ctx.hdr.then(|| Bloom::new(ctx));
        let scene_format = if bloom.is_some() {
            bloom::HDR
        } else {
            ctx.view_format
        };
        let scene_pipelines = Pipelines::new(ctx, &scene_layout, &shaders, scene_format);
        let ui_pipelines = bloom
            .is_some()
            .then(|| Pipelines::new(ctx, &scene_layout, &shaders, ctx.view_format));

        let particles = particles
            .then(|| Particles::new(ctx, &layout, &scene.title, &atlas_pixels, scene_format))
            .flatten();

        Self {
            scene_pipelines,
            ui_pipelines,
            scene_format,
            bloom,
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
            bind_layout: layout,
            timer: None,
            glow: true,
            xray: Instances(None),
        }
    }

    /// Swaps in a rebuilt scene (the other layout, e.g. after rotating a
    /// phone): its instances, and the particles' goals on the new title.
    pub fn set_scene(&mut self, ctx: &Context, scene: &Scene) {
        let device = &ctx.device;
        self.world.glyphs = Instances::new(device, "glyphs", &scene.glyphs);
        self.world.shapes = Instances::new(device, "shapes", &scene.shapes);
        self.world.lines = Instances::new(device, "lines", &scene.lines);
        if self.particles.is_some() {
            self.particles = Particles::new(
                ctx,
                &self.bind_layout,
                &scene.title,
                &text::atlas_rgba(),
                self.scene_format,
            );
        }
        self.set_groups(ctx, None, None, None);
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
            let timestamps = self.timer.as_mut().and_then(GpuTimer::compute_writes);
            particles.step(ctx, step, timestamps);
        }
    }

    /// Turns GPU timing on or off; returns whether the GPU can time.
    /// Whether this GPU renders bloom at all.
    pub fn has_bloom(&self) -> bool {
        self.bloom.is_some()
    }

    /// Turns bloom's glow on or off.
    pub fn set_glow(&mut self, on: bool) {
        self.glow = on;
    }

    /// Shows the x-ray view's `outlines` over the world (none: hides it).
    pub fn set_xray(&mut self, ctx: &Context, outlines: &[ShapeInstance]) {
        self.xray = Instances::new(&ctx.device, "x-ray", outlines);
    }

    pub fn set_timing(&mut self, ctx: &Context, on: bool) -> bool {
        self.timer = if on { GpuTimer::new(ctx) } else { None };
        self.timer.is_some()
    }

    /// GPU times of the newest timed frame that finished since last asked.
    pub fn gpu_times(&mut self, ctx: &Context) -> Option<GpuTimes> {
        self.timer.as_mut()?.collect(ctx)
    }

    /// Instances drawn per frame.
    pub fn counts(&self) -> DrawCounts {
        let layers = [&self.world, &self.ui];
        let sum = |count: fn(&Layer) -> u32| layers.iter().map(|l| count(l)).sum();
        DrawCounts {
            glyphs: sum(|l| l.glyphs.count()),
            shapes: sum(|l| l.shapes.count()) + self.xray.count(),
            lines: sum(|l| l.lines.count()),
            particles: self.particles.as_ref().map_or(0, Particles::drawn),
        }
    }

    /// The GPU memory of its buffers and textures (not the window's
    /// surface).
    pub fn gpu_bytes(&self) -> u64 {
        let [width, height] = text::ATLAS_SIZE;
        let atlas = u64::from(width) * u64::from(height) * 4;
        let layers: u64 = [&self.world, &self.ui]
            .iter()
            .map(|l| l.globals.size() + l.glyphs.bytes() + l.shapes.bytes() + l.lines.bytes())
            .sum();
        atlas
            + layers
            + self.groups.size()
            + self.particles.as_ref().map_or(0, Particles::bytes)
            + self.timer.as_ref().map_or(0, GpuTimer::bytes)
            + self.bloom.as_ref().map_or(0, Bloom::bytes)
            + self.xray.bytes()
    }

    /// Draws a frame into `target` (a `size` view in `ctx.view_format`).
    pub fn draw(
        &mut self,
        ctx: &Context,
        target: &wgpu::TextureView,
        size: [u32; 2],
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

        if let Some(bloom) = &mut self.bloom {
            bloom.prepare(ctx, size);
        }
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        {
            // The scene: into the HDR texture with bloom, else straight into
            // the target, the screen-space layer included.
            let view = self.bloom.as_ref().map_or(target, Bloom::scene);
            let timestamps = self
                .timer
                .as_mut()
                .and_then(|timer| timer.render_writes(Pass::Draw, true, true));
            let mut pass = begin_pass(&mut encoder, "scene", view, clear, timestamps);
            let pipelines = &self.scene_pipelines;
            pass.set_pipeline(&pipelines.background);
            pass.draw(0..3, 0..1);
            pipelines.draw_layer(&mut pass, &self.world, self.particles.as_ref());
            if self.xray.count() > 0 {
                pass.set_bind_group(0, &self.world.bind_group, &[]);
                pass.set_pipeline(&pipelines.shapes);
                self.xray.draw(&mut pass);
            }
            if self.bloom.is_none() {
                pipelines.draw_layer(&mut pass, &self.ui, None);
            }
        }
        if let (Some(bloom), Some(pipelines)) = (&self.bloom, &self.ui_pipelines) {
            bloom.apply(&mut encoder, target, self.timer.as_mut(), self.glow);
            let mut pass = begin_pass(&mut encoder, "ui", target, wgpu::LoadOp::Load, None);
            pipelines.draw_layer(&mut pass, &self.ui, None);
        }
        if let Some(timer) = &mut self.timer {
            timer.resolve(&mut encoder);
        }
        ctx.queue.submit([encoder.finish()]);
        if let Some(timer) = &mut self.timer {
            timer.map();
        }
    }
}

/// A render pass with one color attachment.
fn begin_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &str,
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: timestamps,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// A pipeline drawing one premultiplied-alpha quad (triangle strip) per
/// instance into `format`.
fn instanced_pipeline(
    ctx: &Context,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    stride: usize,
    attributes: &[wgpu::VertexAttribute],
    format: wgpu::TextureFormat,
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
                    format,
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
            ("bloom.wgsl", include_str!("../shaders/bloom.wgsl")),
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
