//! Draws a frame: gradient background, the scene's shapes and MSDF glyphs,
//! then the screen-space layer (native buttons) on top.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::gpu::Gpu;
use crate::scene::{Camera, FAR_FADE, MAX_GROUPS, NEAR_FADE, Scene};
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
    /// Per hover group: `x` = highlight amount.
    groups: wgpu::Buffer,
    world: Layer,
    ui: Layer,
}

impl Renderer {
    pub fn new(gpu: &Gpu, scene: &Scene) -> Self {
        let device = &gpu.device;
        let atlas = atlas_texture(gpu).create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let groups = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hover groups"),
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
        let layer = |label: &str, glyphs: &[GlyphInstance], shapes: &[ShapeInstance]| {
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
            }
        };
        let world = layer("world", &scene.glyphs, &scene.shapes);
        let ui = layer("ui", &[], &[]);

        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let text_shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/text.wgsl"));
        let text = instanced_pipeline(
            gpu,
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
            gpu,
            "shapes",
            &scene_layout,
            &shapes_shader,
            size_of::<ShapeInstance>(),
            &wgpu::vertex_attr_array![
                0 => Float32x4, 1 => Float32x4, 2 => Float32, 3 => Float32, 4 => Float32, 5 => Uint32
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
                targets: &[Some(gpu.view_format.into())],
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
            groups,
            world,
            ui,
        }
    }

    /// Replaces the screen-space layer (e.g. after a resize).
    pub fn set_ui(&mut self, gpu: &Gpu, ui: &UiLayer) {
        self.ui.glyphs = Instances::new(&gpu.device, "ui glyphs", &ui.glyphs);
        self.ui.shapes = Instances::new(&gpu.device, "ui shapes", &ui.shapes);
    }

    /// Highlights one hover group (a link or button), or none.
    pub fn set_highlight(&self, gpu: &Gpu, group: Option<u32>) {
        let mut highlight = [[0.0f32; 4]; MAX_GROUPS];
        if let Some(entry) = group
            .filter(|&g| g != 0)
            .and_then(|g| highlight.get_mut(g as usize))
        {
            entry[0] = 1.0;
        }
        gpu.queue
            .write_buffer(&self.groups, 0, bytemuck::cast_slice(&highlight));
    }

    /// Draws a frame; returns `false` if the frame was skipped (the caller
    /// should request another redraw so the skipped frame isn't the last one).
    pub fn render(&self, gpu: &mut Gpu, camera: &Camera, ui_projection: Mat4) -> bool {
        let Some(frame) = gpu.acquire() else {
            return false;
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(gpu.view_format),
            ..Default::default()
        });

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
        gpu.queue
            .write_buffer(&self.world.globals, 0, bytemuck::bytes_of(&world));
        gpu.queue
            .write_buffer(&self.ui.globals, 0, bytemuck::bytes_of(&ui));

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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

            for layer in [&self.world, &self.ui] {
                pass.set_bind_group(0, &layer.bind_group, &[]);
                pass.set_pipeline(&self.shapes);
                layer.shapes.draw(&mut pass);
                pass.set_pipeline(&self.text);
                layer.glyphs.draw(&mut pass);
            }
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.present(frame);
        true
    }
}

/// A pipeline drawing one premultiplied-alpha quad (triangle strip) per instance.
fn instanced_pipeline(
    gpu: &Gpu,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    stride: usize,
    attributes: &[wgpu::VertexAttribute],
) -> wgpu::RenderPipeline {
    gpu.device
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
                    format: gpu.view_format,
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

/// Uploads the baked MSDF atlas (PNG, RGBA8, linear data).
fn atlas_texture(gpu: &Gpu) -> wgpu::Texture {
    let decoder = png::Decoder::new(std::io::Cursor::new(text::ATLAS_PNG));
    let mut reader = decoder.read_info().expect("baked atlas is a valid PNG");
    let mut pixels = vec![0; reader.output_buffer_size().expect("atlas size")];
    let info = reader.next_frame(&mut pixels).expect("decode atlas");
    let [width, height] = text::ATLAS_SIZE;
    assert_eq!(
        (info.width, info.height, info.color_type),
        (width, height, png::ColorType::Rgba)
    );

    gpu.device.create_texture_with_data(
        &gpu.queue,
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
        &pixels,
    )
}
