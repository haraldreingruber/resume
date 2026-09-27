//! Device, queue and window surface.

use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

pub struct Gpu {
    instance: wgpu::Instance,
    window: Arc<Window>,
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    /// sRGB view of the surface format: shaders output linear colors.
    pub view_format: wgpu::TextureFormat,
}

impl Gpu {
    pub async fn new(display: OwnedDisplayHandle, window: Arc<Window>) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(display),
        ));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        log::info!("GPU adapter: {:?}", adapter.get_info());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("resume"),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;

        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities.formats[0];
        let view_format = format.add_srgb_suffix();
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![view_format],
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            desired_maximum_frame_latency: 2,
            present_mode: wgpu::PresentMode::AutoVsync,
        };
        let gpu = Self {
            instance,
            window,
            surface,
            device,
            queue,
            config,
            view_format,
        };
        gpu.configure();
        Ok(gpu)
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        let max = self.device.limits().max_texture_dimension_2d;
        self.config.width = size.width.clamp(1, max);
        self.config.height = size.height.clamp(1, max);
        self.configure();
    }

    fn configure(&self) {
        self.surface.configure(&self.device, &self.config);
    }

    /// The next frame to draw into, or `None` to skip this frame.
    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => Some(texture),
            wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout => None,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                drop(texture);
                self.configure();
                None
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.configure();
                None
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                match self.instance.create_surface(self.window.clone()) {
                    Ok(surface) => {
                        self.surface = surface;
                        self.configure();
                    }
                    Err(error) => log::error!("recreating surface failed: {error}"),
                }
                None
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("surface validation error");
                None
            }
        }
    }

    pub fn present(&self, frame: wgpu::SurfaceTexture) {
        self.window.pre_present_notify();
        self.queue.present(frame);
    }
}
