//! winit application: creates the window, initializes the GPU (async on the
//! web), turns input into timeline movement and renders on demand.

use std::sync::Arc;

use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::gpu::Gpu;
use crate::renderer::Renderer;
use crate::scene::Scene;
use crate::timeline::Timeline;

/// Timeline units per wheel line and per touch/trackpad pixel.
const SCROLL_PER_LINE: f32 = 0.35;
const SCROLL_PER_PIXEL: f32 = 1.0 / 400.0;

pub enum AppEvent {
    /// GPU initialization finished (asynchronous on the web).
    GpuReady(Result<Gpu, String>),
}

pub struct App {
    proxy: EventLoopProxy<AppEvent>,
    title: String,
    scene: Scene,
    timeline: Timeline,
    window: Option<Arc<Window>>,
    state: Option<State>,
    touch_y: Option<f64>,
    last_frame: Instant,
}

struct State {
    gpu: Gpu,
    renderer: Renderer,
}

impl App {
    /// `start_station` deep-links into the timeline (0 = intro).
    pub fn new(event_loop: &EventLoop<AppEvent>, start_station: usize) -> Self {
        let resume = crate::content::resume();
        let scene = Scene::new(&resume);
        Self {
            proxy: event_loop.create_proxy(),
            title: format!("{} – 3D Resume", resume.basics.name),
            timeline: Timeline::starting_at(scene.station_count(), start_station),
            scene,
            window: None,
            state: None,
            touch_y: None,
            last_frame: Instant::now(),
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let moving = self.timeline.update(dt);

        let Some(state) = &mut self.state else { return };
        let camera = self
            .scene
            .camera(self.timeline.position(), state.gpu.aspect());
        let presented = state.renderer.render(&mut state.gpu, &camera);
        // Keep redrawing while the timeline is animating, and retry a frame
        // the surface skipped (e.g. right after the first `configure()`) so a
        // skipped frame is never the last one drawn.
        if moving || !presented {
            self.request_redraw();
        }
    }

    fn keyboard(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        match &event.logical_key {
            Key::Named(
                NamedKey::ArrowDown | NamedKey::ArrowRight | NamedKey::PageDown | NamedKey::Space,
            ) => self.timeline.step(1),
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowLeft | NamedKey::PageUp) => {
                self.timeline.step(-1)
            }
            Key::Named(NamedKey::Home) => self.timeline.go_to_start(),
            Key::Named(NamedKey::End) => self.timeline.go_to_end(),
            _ => return,
        }
        self.request_redraw();
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(window_attributes(&self.title)) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                log::error!("creating window failed: {error}");
                event_loop.exit();
                return;
            }
        };
        self.window = Some(window.clone());

        let display = event_loop.owned_display_handle();
        let proxy = self.proxy.clone();
        let init = async move {
            let gpu = Gpu::new(display, window).await;
            let _ = proxy.send_event(AppEvent::GpuReady(gpu));
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(init);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(init);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::GpuReady(Ok(gpu)) => {
                let renderer = Renderer::new(&gpu, &self.scene.glyphs);
                self.state = Some(State { gpu, renderer });
                self.last_frame = Instant::now();
                self.request_redraw();
            }
            AppEvent::GpuReady(Err(error)) => {
                log::error!("WebGPU initialization failed: {error}");
                #[cfg(target_arch = "wasm32")]
                crate::web::show_plain_version();
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    state.gpu.resize(size);
                }
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::KeyboardInput { event, .. } => self.keyboard(&event),
            WindowEvent::MouseWheel { delta, .. } => {
                let forward = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * SCROLL_PER_LINE,
                    MouseScrollDelta::PixelDelta(position) => -position.y as f32 * SCROLL_PER_PIXEL,
                };
                self.timeline.scroll(forward);
                self.request_redraw();
            }
            WindowEvent::Touch(touch) => {
                match touch.phase {
                    TouchPhase::Started => self.touch_y = Some(touch.location.y),
                    TouchPhase::Moved => {
                        if let Some(last) = self.touch_y.replace(touch.location.y) {
                            // Dragging up moves forward, like scrolling down.
                            self.timeline
                                .scroll((last - touch.location.y) as f32 * SCROLL_PER_PIXEL * 2.0);
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => self.touch_y = None,
                }
                self.request_redraw();
            }
            _ => {}
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn window_attributes(title: &str) -> winit::window::WindowAttributes {
    Window::default_attributes()
        .with_title(title)
        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0))
}

#[cfg(target_arch = "wasm32")]
fn window_attributes(title: &str) -> winit::window::WindowAttributes {
    use winit::platform::web::WindowAttributesExtWebSys;
    Window::default_attributes()
        .with_title(title)
        .with_canvas(crate::web::canvas())
}
