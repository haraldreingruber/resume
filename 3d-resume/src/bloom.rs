//! Bloom: the scene renders into a floating-point (HDR) texture; its bright
//! parts are blurred over a chain of ever smaller textures (half size, then
//! halving) and added back as a soft glow when copying the scene to the
//! window. Overlapping particles add up beyond 1 there, so they glow most.
//! See `shaders/bloom.wgsl`.

use crate::gpu::Context;
use crate::gpu_timer::{GpuTimer, Pass};

/// Format of the scene and the glow: floating point, so bright parts can
/// exceed 1 and blending stays precise.
pub const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Mips in the chain, at most (the first at half resolution).
const MAX_LEVELS: usize = 6;
/// The chain stops before a mip gets smaller than this (pixels) either way.
const MIN_SIZE: u32 = 8;

/// Whether the GPU can render, blend and filter `HDR` (all but some
/// OpenGL ES phones can).
pub fn supported(adapter: &wgpu::Adapter) -> bool {
    let features = adapter.get_texture_format_features(HDR);
    features
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING)
        && features.flags.contains(
            wgpu::TextureFormatFeatureFlags::FILTERABLE
                | wgpu::TextureFormatFeatureFlags::BLENDABLE,
        )
}

pub struct Bloom {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    prefilter: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    /// Textures for the current window size.
    targets: Option<Targets>,
}

struct Targets {
    size: [u32; 2],
    scene: wgpu::TextureView,
    /// Reads the scene (the prefilter's source).
    scene_group: wgpu::BindGroup,
    /// Largest first.
    mips: Vec<Mip>,
    bytes: u64,
}

struct Mip {
    view: wgpu::TextureView,
    /// Reads this mip (as the source of the next pass).
    group: wgpu::BindGroup,
}

impl Bloom {
    pub fn new(ctx: &Context) -> Self {
        let device = &ctx.device;
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[
                texture(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(2),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/bloom.wgsl"));
        let pipeline = |entry: &str, format, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        Self {
            prefilter: pipeline("fs_prefilter", HDR, None),
            down: pipeline("fs_down", HDR, None),
            up: pipeline("fs_up", HDR, Some(add)),
            composite: pipeline("fs_composite", ctx.view_format, None),
            layout,
            sampler,
            targets: None,
        }
    }

    /// Makes sure the textures fit a `size` window.
    pub fn prepare(&mut self, ctx: &Context, size: [u32; 2]) {
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            self.targets = Some(self.create_targets(ctx, size));
        }
    }

    fn create_targets(&self, ctx: &Context, size: [u32; 2]) -> Targets {
        let device = &ctx.device;
        let mut bytes = 0;
        let mut texture = |label, [width, height]: [u32; 2]| {
            bytes += u64::from(width) * u64::from(height) * 8;
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: HDR,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let scene = texture("hdr scene", size);
        let views: Vec<wgpu::TextureView> = levels(size)
            .into_iter()
            .map(|mip_size| texture("bloom mip", mip_size))
            .collect();
        let group = |source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&scene),
                    },
                ],
            })
        };
        let scene_group = group(&scene);
        let mips = views
            .into_iter()
            .map(|view| Mip {
                group: group(&view),
                view,
            })
            .collect();
        Targets {
            size,
            scene_group,
            scene,
            mips,
            bytes,
        }
    }

    /// Where the scene renders to (after `prepare`).
    pub fn scene(&self) -> &wgpu::TextureView {
        &self.targets.as_ref().expect("prepared").scene
    }

    /// Blurs the scene's bright parts and writes scene + glow to `target`
    /// (a view in `ctx.view_format`).
    pub fn apply(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        mut timer: Option<&mut GpuTimer>,
    ) {
        let targets = self.targets.as_ref().expect("prepared");
        let mips = &targets.mips;
        let mut pass =
            |label,
             view: &wgpu::TextureView,
             load,
             pipeline: &wgpu::RenderPipeline,
             source: &wgpu::BindGroup,
             timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>| {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, source, &[]);
                pass.draw(0..3, 0..1);
            };
        let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        let begin = timer
            .as_deref_mut()
            .and_then(|t| t.render_writes(Pass::Bloom, true, false));
        pass(
            "bloom prefilter",
            &mips[0].view,
            clear,
            &self.prefilter,
            &targets.scene_group,
            begin,
        );
        for i in 1..mips.len() {
            pass(
                "bloom down",
                &mips[i].view,
                clear,
                &self.down,
                &mips[i - 1].group,
                None,
            );
        }
        for i in (0..mips.len() - 1).rev() {
            pass(
                "bloom up",
                &mips[i].view,
                wgpu::LoadOp::Load,
                &self.up,
                &mips[i + 1].group,
                None,
            );
        }
        let end = timer.and_then(|t| t.render_writes(Pass::Bloom, false, true));
        pass(
            "bloom composite",
            target,
            clear,
            &self.composite,
            &mips[0].group,
            end,
        );
    }

    /// The GPU memory its textures take.
    pub fn bytes(&self) -> u64 {
        self.targets.as_ref().map_or(0, |t| t.bytes)
    }
}

/// Sizes of the mip chain for a `size` window: half size, then halving,
/// while both sides stay at least `MIN_SIZE`; at least the first.
fn levels([width, height]: [u32; 2]) -> Vec<[u32; 2]> {
    let mut levels: Vec<[u32; 2]> = (1..=MAX_LEVELS as u32)
        .map(|level| [width >> level, height >> level])
        .take_while(|&[w, h]| w >= MIN_SIZE && h >= MIN_SIZE)
        .collect();
    if levels.is_empty() {
        levels.push([(width / 2).max(1), (height / 2).max(1)]);
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_down_to_a_minimum() {
        assert_eq!(
            levels([1280, 800]),
            [
                [640, 400],
                [320, 200],
                [160, 100],
                [80, 50],
                [40, 25],
                [20, 12]
            ]
        );
        // A tall phone stops when the narrow side gets too small.
        assert_eq!(levels([100, 2000]).len(), 3);
        // A tiny window still gets one.
        assert_eq!(levels([10, 10]), [[5, 5]]);
    }
}
