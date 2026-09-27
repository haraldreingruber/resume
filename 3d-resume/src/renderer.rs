//! Draws a frame: gradient background, then the scene's MSDF glyphs.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu::Gpu;
use crate::scene::Camera;
use crate::text::{self, GlyphInstance};

/// Text fades in/out relative to the focus distance: stations the camera
/// passes fade out, the next station fades in as the camera approaches.
const NEAR_FADE: (f32, f32) = (-2.8, -1.2);
const FAR_FADE: (f32, f32) = (1.8, 4.5);

/// Matches `Globals` in `shaders/text.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    /// Camera distances: near fade start/end, far fade start/end.
    fade: [f32; 4],
    /// x: MSDF distance range in atlas pixels.
    params: [f32; 4],
}

pub struct Renderer {
    background: wgpu::RenderPipeline,
    text: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    glyphs: wgpu::Buffer,
    glyph_count: u32,
}

impl Renderer {
    pub fn new(gpu: &Gpu, glyphs: &[GlyphInstance]) -> Self {
        let device = &gpu.device;

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let atlas = atlas_texture(gpu);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
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
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &atlas.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let text_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let text_shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/text.wgsl"));
        let text = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text"),
            layout: Some(&text_layout),
            vertex: wgpu::VertexState {
                module: &text_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<GlyphInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &text_shader,
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
        });

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

        let glyph_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("glyphs"),
            contents: bytemuck::cast_slice(glyphs),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            background,
            text,
            globals,
            bind_group,
            glyphs: glyph_buffer,
            glyph_count: glyphs.len() as u32,
        }
    }

    /// Draws a frame; returns `false` if the frame was skipped (the caller
    /// should request another redraw so the skipped frame isn't the last one).
    pub fn render(&self, gpu: &mut Gpu, camera: &Camera) -> bool {
        let Some(frame) = gpu.acquire() else {
            return false;
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(gpu.view_format),
            ..Default::default()
        });

        let focus = camera.focus_distance;
        let globals = Globals {
            view_proj: camera.view_proj.to_cols_array_2d(),
            camera_pos: camera.eye.extend(1.0).to_array(),
            fade: [
                focus + NEAR_FADE.0,
                focus + NEAR_FADE.1,
                focus + FAR_FADE.0,
                focus + FAR_FADE.1,
            ],
            params: [text::DISTANCE_RANGE_PX, 0.0, 0.0, 0.0],
        };
        gpu.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

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

            pass.set_pipeline(&self.text);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.glyphs.slice(..));
            pass.draw(0..4, 0..self.glyph_count);
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.present(frame);
        true
    }
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
