//! winit application: creates the window, initializes the GPU (async on the
//! web), turns input into timeline movement and link clicks, and renders on
//! demand.

use std::sync::Arc;

use glam::Vec2;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{
    ElementState, KeyEvent, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::gpu::Gpu;
use crate::renderer::Renderer;
use crate::scene::{Action, Lens, Scene};
use crate::timeline::Timeline;
use crate::ui::UiLayer;

/// Timeline units per wheel line and per touch/trackpad pixel.
const SCROLL_PER_LINE: f32 = 0.35;
const SCROLL_PER_PIXEL: f32 = 1.0 / 400.0;
/// A touch that moves less than this (physical pixels) is a tap.
const TAP_SLOP: f64 = 12.0;

pub enum AppEvent {
    /// GPU initialization finished (asynchronous on the web).
    GpuReady(Result<Gpu, String>),
}

/// Start options from the URL (web) or command line (native).
pub struct Options {
    /// Deep link: a station id (`dedalus`) or index.
    pub station: Option<String>,
    /// Jump between stations instead of easing (prefers-reduced-motion).
    pub reduced_motion: bool,
}

pub struct App {
    proxy: EventLoopProxy<AppEvent>,
    title: String,
    scene: Scene,
    timeline: Timeline,
    /// Screen-space buttons (native only; the web page has an HTML nav).
    buttons: Vec<(&'static str, u32)>,
    window: Option<Arc<Window>>,
    state: Option<State>,
    last_frame: Instant,
    cursor: Option<PhysicalPosition<f64>>,
    /// Hover group under the cursor, and the one a mouse press started on.
    hovered: Option<u32>,
    pressed: Option<u32>,
    touch: Option<Touch>,
}

struct State {
    gpu: Gpu,
    renderer: Renderer,
    lens: Lens,
    ui: UiLayer,
}

struct Touch {
    start: PhysicalPosition<f64>,
    last_y: f64,
    /// Still within `TAP_SLOP` of the start.
    tap: bool,
}

impl App {
    pub fn new(event_loop: &EventLoop<AppEvent>, options: Options) -> Self {
        let resume = crate::content::resume();
        let mut scene = Scene::new(&resume);
        let buttons = if cfg!(target_arch = "wasm32") {
            Vec::new()
        } else {
            vec![
                ("Text version", scene.add_action(Action::TextVersion)),
                ("PDF", scene.add_action(Action::Pdf)),
            ]
        };
        let start = options
            .station
            .as_deref()
            .and_then(|key| scene.station_index(key))
            .unwrap_or(0);
        let mut timeline = Timeline::starting_at(scene.station_count(), start);
        timeline.set_instant(options.reduced_motion);
        Self {
            proxy: event_loop.create_proxy(),
            title: format!("{} – 3D Resume", resume.basics.name),
            scene,
            timeline,
            buttons,
            window: None,
            state: None,
            last_frame: Instant::now(),
            cursor: None,
            hovered: None,
            pressed: None,
            touch: None,
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
        let camera = self.scene.camera(self.timeline.position(), &state.lens);
        let (width, height) = (
            state.gpu.config.width as f32,
            state.gpu.config.height as f32,
        );
        let presented =
            state
                .renderer
                .render(&mut state.gpu, &camera, UiLayer::projection(width, height));
        // Keep redrawing while the timeline is animating, and retry a frame
        // the surface skipped (e.g. right after the first `configure()`) so a
        // skipped frame is never the last one drawn.
        if moving || !presented {
            self.request_redraw();
        }
        if moving {
            // The link under a resting cursor changes as the camera moves.
            self.update_hover();
        }
    }

    /// Rebuilds size-dependent state: camera lens and screen-space buttons.
    fn layout(&mut self) {
        let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
        let Some(state) = &mut self.state else { return };
        state.lens = Scene::lens(state.gpu.aspect());
        state.ui = UiLayer::buttons(state.gpu.config.width as f32, scale, &self.buttons);
        state.renderer.set_ui(&state.gpu, &state.ui);
    }

    /// The hover group at a cursor position: screen-space buttons first, then
    /// links in the scene.
    fn group_at(&self, position: PhysicalPosition<f64>) -> Option<u32> {
        let state = self.state.as_ref()?;
        let (width, height) = (
            state.gpu.config.width as f32,
            state.gpu.config.height as f32,
        );
        let (x, y) = (position.x as f32, position.y as f32);
        state.ui.pick(x, height - y).or_else(|| {
            let ndc = Vec2::new(2.0 * x / width - 1.0, 1.0 - 2.0 * y / height);
            let camera = self.scene.camera(self.timeline.position(), &state.lens);
            self.scene.pick(&camera, ndc)
        })
    }

    fn update_hover(&mut self) {
        let group = self.cursor.and_then(|cursor| self.group_at(cursor));
        if group == self.hovered {
            return;
        }
        self.hovered = group;
        if let Some(state) = &self.state {
            state.renderer.set_highlight(&state.gpu, group);
        }
        if let Some(window) = &self.window {
            window.set_cursor(if group.is_some() {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            });
        }
        self.request_redraw();
    }

    fn activate(&self, group: u32) {
        if let Some(action) = self.scene.action(group) {
            crate::links::open(action);
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

    fn touch(&mut self, phase: TouchPhase, location: PhysicalPosition<f64>) {
        match phase {
            TouchPhase::Started => {
                self.touch = Some(Touch {
                    start: location,
                    last_y: location.y,
                    tap: true,
                });
            }
            TouchPhase::Moved => {
                if let Some(touch) = &mut self.touch {
                    // Dragging up moves forward, like scrolling down.
                    let delta = (touch.last_y - location.y) as f32 * SCROLL_PER_PIXEL * 2.0;
                    touch.last_y = location.y;
                    let (dx, dy) = (location.x - touch.start.x, location.y - touch.start.y);
                    touch.tap &= dx.hypot(dy) < TAP_SLOP;
                    self.timeline.scroll(delta);
                }
            }
            TouchPhase::Ended => {
                if self.touch.take().is_some_and(|touch| touch.tap)
                    && let Some(group) = self.group_at(location)
                {
                    self.activate(group);
                }
            }
            TouchPhase::Cancelled => self.touch = None,
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
                let renderer = Renderer::new(&gpu, &self.scene);
                self.state = Some(State {
                    lens: Scene::lens(gpu.aspect()),
                    ui: UiLayer::default(),
                    gpu,
                    renderer,
                });
                self.layout();
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
                self.layout();
                self.update_hover();
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => self.layout(),
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
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some(position);
                self.update_hover();
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.update_hover();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => self.pressed = self.hovered,
                ElementState::Released => {
                    if let Some(group) = self.hovered
                        && self.pressed.take() == Some(group)
                    {
                        self.activate(group);
                    }
                }
            },
            WindowEvent::Touch(touch) => self.touch(touch.phase, touch.location),
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
