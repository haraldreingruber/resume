//! winit application: creates the window, initializes the GPU (async on the
//! web), turns input into timeline movement, link clicks, keyboard focus and
//! fullscreen toggles, and renders on demand.

use std::sync::Arc;
use std::time::Duration;

use glam::Vec2;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{
    ElementState, KeyEvent, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::focus::{self, Step};
use crate::gpu::Gpu;
use crate::renderer::Renderer;
use crate::scene::{Lens, Scene};
use crate::timeline::Timeline;
use crate::ui::{self, Button, UiLayer};

/// Timeline units per wheel line and per touch/trackpad pixel.
const SCROLL_PER_LINE: f32 = 0.35;
const SCROLL_PER_PIXEL: f32 = 1.0 / 400.0;
/// A touch that moves less than this (physical pixels) is a tap.
const TAP_SLOP: f64 = 12.0;
/// Two clicks or taps on empty space within this time and distance (physical
/// pixels) are a double-click, which toggles fullscreen.
const DOUBLE_PRESS_TIME: Duration = Duration::from_millis(400);
const DOUBLE_PRESS_SLOP: f64 = 24.0;

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
    buttons: Vec<Button>,
    window: Option<Arc<Window>>,
    state: Option<State>,
    last_frame: Instant,
    cursor: Option<PhysicalPosition<f64>>,
    /// Hover group under the cursor, and the one a mouse press started on.
    hovered: Option<u32>,
    pressed: Option<u32>,
    touch: Option<Touch>,
    /// Hover group with keyboard focus (Tab / Shift+Tab, Enter opens it).
    focused: Option<u32>,
    /// The last click or tap on empty space (half of a double-click).
    empty_press: Option<Press>,
    modifiers: ModifiersState,
    /// The station in view, and the one last shown in the address bar.
    station: usize,
    shown_station: usize,
    /// Whether Tab / Shift+Tab leaves the canvas, read by the page's listener.
    #[cfg(target_arch = "wasm32")]
    tab_leaves: std::rc::Rc<std::cell::Cell<[bool; 2]>>,
}

struct State {
    gpu: Gpu,
    renderer: Renderer,
    lens: Lens,
    ui: UiLayer,
}

/// When and where a click or tap happened.
type Press = (Instant, PhysicalPosition<f64>);

/// Whether `second` completes a double-click (or double-tap) with `first`.
fn is_double_press((t0, p0): Press, (t1, p1): Press) -> bool {
    t1.saturating_duration_since(t0) <= DOUBLE_PRESS_TIME
        && (p1.x - p0.x).hypot(p1.y - p0.y) <= DOUBLE_PRESS_SLOP
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
        let buttons = ui::native_buttons(&mut scene);
        let start = options
            .station
            .as_deref()
            .and_then(|key| scene.station_index(key))
            .unwrap_or(0);
        let mut timeline = Timeline::starting_at(scene.station_count(), start);
        timeline.set_instant(options.reduced_motion);
        let app = Self {
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
            focused: None,
            empty_press: None,
            modifiers: ModifiersState::empty(),
            station: start,
            shown_station: start,
            #[cfg(target_arch = "wasm32")]
            tab_leaves: Default::default(),
        };
        #[cfg(target_arch = "wasm32")]
        crate::web::release_tab_at_edges(app.tab_leaves.clone());
        app.publish_tab_leaves();
        app
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

        let Some(State {
            gpu,
            renderer,
            lens,
            ..
        }) = &mut self.state
        else {
            return;
        };
        let camera = self.scene.camera(self.timeline.position(), lens);
        let projection = UiLayer::projection(gpu.config.width as f32, gpu.config.height as f32);
        let presented = gpu.render(|ctx, view| renderer.draw(ctx, view, &camera, projection));
        // Keep redrawing while the timeline is animating, and retry a frame
        // the surface skipped (e.g. right after the first `configure()`) so a
        // skipped frame is never the last one drawn.
        if moving || !presented {
            self.request_redraw();
        }
        self.follow_station(!moving);
        if moving {
            // The link under a resting cursor changes as the camera moves.
            self.update_hover();
        }
    }

    /// Keeps station-dependent state in step with the timeline: keyboard
    /// focus stays within the station in view, and once the timeline has
    /// `settled`, the web address names that station.
    fn follow_station(&mut self, settled: bool) {
        let station = self.timeline.nearest();
        if station != self.station {
            self.station = station;
            if self.focused.is_some_and(|g| !self.targets().contains(&g)) {
                self.set_focus(None);
            } else {
                self.publish_tab_leaves();
            }
        }
        if settled && station != self.shown_station {
            self.shown_station = station;
            #[cfg(target_arch = "wasm32")]
            crate::web::show_station(
                Some(station)
                    .filter(|&s| s > 0)
                    .and_then(|s| self.scene.station_id(s)),
            );
        }
    }

    /// Keyboard focus targets: the links of the station in view, then the
    /// screen-space buttons.
    fn targets(&self) -> Vec<u32> {
        focus::targets(&self.scene, self.station, &self.buttons)
    }

    fn focus_index(&self, targets: &[u32]) -> Option<usize> {
        let focused = self.focused?;
        targets.iter().position(|&group| group == focused)
    }

    /// Tab (`forward`) or Shift+Tab.
    fn move_focus(&mut self, forward: bool) {
        let targets = self.targets();
        let current = self.focus_index(&targets);
        match focus::step(current, targets.len(), forward, focus::WRAPS) {
            Step::To(index) => self.set_focus(index.map(|i| targets[i])),
            // The page's listener normally lets such a Tab through to the
            // browser before winit sees it.
            Step::Leave => self.set_focus(None),
        }
    }

    fn set_focus(&mut self, group: Option<u32>) {
        self.focused = group;
        self.update_groups();
        self.publish_tab_leaves();
    }

    /// Tells the page whether the next Tab / Shift+Tab leaves the canvas.
    fn publish_tab_leaves(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            let targets = self.targets();
            let current = self.focus_index(&targets);
            let leaves =
                |forward| focus::step(current, targets.len(), forward, focus::WRAPS) == Step::Leave;
            self.tab_leaves.set([leaves(true), leaves(false)]);
        }
    }

    /// Uploads the hovered and focused groups and redraws.
    fn update_groups(&self) {
        if let Some(state) = &self.state {
            state
                .renderer
                .set_groups(&state.gpu.context, self.hovered, self.focused);
        }
        self.request_redraw();
    }

    /// Rebuilds size-dependent state: camera lens and screen-space buttons.
    fn layout(&mut self) {
        let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
        let Some(state) = &mut self.state else { return };
        state.lens = Scene::lens(state.gpu.aspect());
        state.ui = UiLayer::buttons(state.gpu.config.width as f32, scale, &self.buttons);
        state.renderer.set_ui(&state.gpu.context, &state.ui);
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
        self.update_groups();
        if let Some(window) = &self.window {
            window.set_cursor(if group.is_some() {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            });
        }
    }

    fn activate(&self, group: u32) {
        if let Some(action) = self.scene.action(group) {
            crate::links::open(action);
        }
    }

    /// A click or tap on empty space; the second of a quick pair toggles
    /// fullscreen.
    fn press_empty(&mut self, position: PhysicalPosition<f64>) {
        let press = (Instant::now(), position);
        if self
            .empty_press
            .take()
            .is_some_and(|first| is_double_press(first, press))
        {
            self.toggle_fullscreen();
        } else {
            self.empty_press = Some(press);
        }
    }

    /// Web: the whole page (keeps the HTML nav); native: borderless on the
    /// current monitor.
    fn toggle_fullscreen(&self) {
        #[cfg(target_arch = "wasm32")]
        crate::web::toggle_fullscreen();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(window) = &self.window {
            window.set_fullscreen(match window.fullscreen() {
                Some(_) => None,
                None => Some(winit::window::Fullscreen::Borderless(None)),
            });
        }
    }

    /// Native Esc; browsers handle Esc in fullscreen themselves.
    fn leave_fullscreen(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(window) = &self.window
            && window.fullscreen().is_some()
        {
            window.set_fullscreen(None);
        }
    }

    fn keyboard(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Tab) => return self.move_focus(!self.modifiers.shift_key()),
            Key::Named(NamedKey::Enter) => {
                if let Some(group) = self.focused {
                    self.activate(group);
                }
                return;
            }
            Key::Named(NamedKey::Escape) => {
                if self.focused.is_some() {
                    self.set_focus(None);
                } else {
                    self.leave_fullscreen();
                }
                return;
            }
            // Browsers use F11 for their own fullscreen.
            #[cfg(not(target_arch = "wasm32"))]
            Key::Named(NamedKey::F11) => return self.toggle_fullscreen(),
            Key::Character(c)
                if c.eq_ignore_ascii_case("f")
                    && !event.repeat
                    && !(self.modifiers.control_key()
                        || self.modifiers.alt_key()
                        || self.modifiers.super_key()) =>
            {
                return self.toggle_fullscreen();
            }
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
                if self.touch.take().is_some_and(|touch| touch.tap) {
                    match self.group_at(location) {
                        Some(group) => self.activate(group),
                        None => self.press_empty(location),
                    }
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
                let renderer = Renderer::new(&gpu.context, &self.scene);
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
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            // The window lost focus, or on the web, Tab moved on from the canvas.
            WindowEvent::Focused(false) => self.set_focus(None),
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
                ElementState::Pressed => {
                    self.pressed = self.hovered;
                    if self.hovered.is_none()
                        && let Some(cursor) = self.cursor
                    {
                        self.press_empty(cursor);
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_press_needs_two_quick_nearby_presses() {
        let t0 = Instant::now();
        let at = |x| PhysicalPosition::new(x, 100.0);
        let first = (t0, at(100.0));
        assert!(is_double_press(
            first,
            (t0 + Duration::from_millis(250), at(110.0))
        ));
        assert!(!is_double_press(
            first,
            (t0 + Duration::from_millis(600), at(100.0))
        ));
        assert!(!is_double_press(
            first,
            (t0 + Duration::from_millis(250), at(200.0))
        ));
    }
}
