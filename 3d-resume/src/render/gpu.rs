//! Device, queue and window surface.

use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

use crate::debug::startup;

/// Graphics APIs to try first, and the ones to fall back to if none of those
/// has an adapter. Starting every API at once costs the sum of their startup
/// times (Vulkan and DirectX 12 together: ~1.2 s instead of ~0.8 s on a
/// Windows laptop), and DirectX 12 compiles shaders far slower than Vulkan.
/// `WGPU_BACKEND` overrides both, e.g. `WGPU_BACKEND=dx12`.
pub const FIRST_CHOICE: wgpu::Backends = wgpu::Backends::VULKAN
    .union(wgpu::Backends::METAL)
    .union(wgpu::Backends::BROWSER_WEBGPU);
pub const FALLBACK: wgpu::Backends = wgpu::Backends::DX12.union(wgpu::Backends::GL);

/// What the renderer needs: the device, its queue and the color format it
/// draws in. Shared by the window surface and headless screenshots.
pub struct Context {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// sRGB format of the render target: shaders output linear colors.
    pub view_format: wgpu::TextureFormat,
    /// Whether compute shaders run here (the intro particles need them;
    /// OpenGL ES 3.0 phones lack them).
    pub compute: bool,
    /// The backend and GPU (shown in the About panel).
    pub adapter: wgpu::AdapterInfo,
    /// Whether it renders to floating-point textures (for bloom).
    pub hdr: bool,
}

impl Context {
    /// Opens a device on `adapter`. It asks for WebGPU's default limits,
    /// lowered wherever the adapter offers less (the iOS Simulator, say,
    /// has 15 inter-stage variables, not 16): the app needs far less than
    /// either.
    pub async fn new(
        adapter: &wgpu::Adapter,
        label: &str,
        view_format: wgpu::TextureFormat,
    ) -> Result<Self, String> {
        startup::mark("GPU adapter");
        let info = adapter.get_info();
        log::info!("GPU adapter: {info:?}");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some(label),
                // For the performance overlay's GPU times, where available.
                required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
                required_limits: wgpu::Limits::default().or_worse_values_from(&adapter.limits()),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        startup::mark("GPU device");
        let compute = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS);
        if !compute {
            log::info!("no compute shaders: the intro shows the title without particles");
        }
        Ok(Self {
            device,
            queue,
            view_format,
            compute,
            adapter: info,
            hdr: crate::render::bloom::supported(adapter),
        })
    }
}

pub struct Gpu {
    instance: wgpu::Instance,
    window: Arc<Window>,
    /// `None` while the app is suspended (Android destroys the window's
    /// surface then).
    surface: Option<wgpu::Surface<'static>>,
    pub context: Context,
    pub config: wgpu::SurfaceConfiguration,
    /// Frames the surface skipped so far (the first few are logged).
    skipped: u32,
}

impl Gpu {
    pub async fn new(display: OwnedDisplayHandle, window: Arc<Window>) -> Result<Self, String> {
        let mut error = String::from("no graphics API");
        for backends in [FIRST_CHOICE, FALLBACK] {
            let descriptor = wgpu::InstanceDescriptor {
                backends,
                ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display.clone()))
            }
            .with_env();
            let instance = wgpu::Instance::new(descriptor);
            let surface = instance
                .create_surface(window.clone())
                .map_err(|e| e.to_string())?;
            let options = wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            };
            match instance.request_adapter(&options).await {
                Ok(adapter) => {
                    return Self::with_adapter(instance, surface, &adapter, window).await;
                }
                Err(e) => error = e.to_string(),
            }
        }
        Err(error)
    }

    async fn with_adapter(
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        window: Arc<Window>,
    ) -> Result<Self, String> {
        let capabilities = surface.get_capabilities(adapter);
        let format = capabilities.formats[0];
        let view_format = format.add_srgb_suffix();
        let context = Context::new(adapter, "resume", view_format).await?;
        let size = surface_size(&window);
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
            surface: Some(surface),
            context,
            config,
            skipped: 0,
        };
        gpu.configure();
        startup::mark("surface configured");
        log::info!(
            "surface: {:?} (drawn as {:?}), {}x{}, {:?}",
            gpu.config.format,
            gpu.context.view_format,
            gpu.config.width,
            gpu.config.height,
            gpu.config.present_mode
        );
        Ok(gpu)
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        let max = self.context.device.limits().max_texture_dimension_2d;
        self.config.width = size.width.clamp(1, max);
        self.config.height = size.height.clamp(1, max);
        self.configure();
    }

    fn configure(&self) {
        if let Some(surface) = &self.surface {
            surface.configure(&self.context.device, &self.config);
        }
    }

    /// The app went to the background: drop the surface (Android destroys
    /// the native window).
    pub fn suspend(&mut self) {
        self.surface = None;
    }

    /// Back in the foreground: a new surface for the (possibly new) native
    /// window.
    pub fn resume(&mut self) {
        if self.surface.is_some() {
            return;
        }
        match self.instance.create_surface(self.window.clone()) {
            Ok(surface) => {
                self.surface = Some(surface);
                self.resize(surface_size(&self.window));
            }
            Err(error) => log::error!("creating surface failed: {error}"),
        }
    }

    /// Draws a frame with `draw` and presents it. Returns `false` if the
    /// surface skipped this frame (the caller should request another redraw
    /// so the skipped frame isn't the last one).
    pub fn render(&mut self, draw: impl FnOnce(&Context, &wgpu::TextureView)) -> bool {
        let Some(frame) = self.acquire() else {
            return false;
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.context.view_format),
            ..Default::default()
        });
        draw(&self.context, &view);
        self.window.pre_present_notify();
        self.context.queue.present(frame);
        true
    }

    /// The next frame to draw into, or `None` to skip this frame.
    fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        let status = self.surface.as_ref()?.get_current_texture();
        if !matches!(status, wgpu::CurrentSurfaceTexture::Success(_)) && self.skipped < 5 {
            self.skipped += 1;
            log::info!("frame skipped: {status:?}");
        }
        match status {
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
                        self.surface = Some(surface);
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
}

/// The size of the area the surface covers: the window's content. On iOS
/// that's the whole screen, while winit's `inner_size` is only its safe area
/// (without the notch and home indicator), so the image would be stretched.
pub fn surface_size(window: &Window) -> PhysicalSize<u32> {
    if cfg!(target_os = "ios") {
        window.outer_size()
    } else {
        window.inner_size()
    }
}
