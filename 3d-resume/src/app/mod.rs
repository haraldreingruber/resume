//! winit application: creates the window, initializes the GPU (async on the
//! web), turns input into timeline movement, link clicks, keyboard focus,
//! the About panel, the performance overlay and fullscreen toggles, runs the
//! particle intro, and renders on demand.

mod frame;
mod input;
mod navigation;
mod panels;

use std::sync::Arc;
use std::time::Duration;

use glam::{Vec2, Vec3};
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::StartCause;
use winit::event::{
    ElementState, KeyEvent, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::debug::stats::{self, GraphFrame, Stats};
use crate::render::gpu::Gpu;
use crate::render::particles;
use crate::render::renderer::Renderer;
use crate::scene::intro::{self, Intro};
use crate::scene::timeline::Timeline;
use crate::scene::{self, Action, Input, Lens, Metrics, Scene};
use crate::ui::about;
use crate::ui::focus::{self, Step};
use crate::ui::{self, Button, Insets, Overlay, Panel, Rail, Switch, Switches, UiLayer};

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
    GpuReady(Result<Box<Gpu>, String>),
    /// The page's Skills link was clicked.
    #[cfg(target_arch = "wasm32")]
    ToggleSkills,
    /// The page's About button was clicked.
    #[cfg(target_arch = "wasm32")]
    ToggleAbout,
}

/// Start options from the URL (web) or command line (native).
pub struct Options {
    /// Deep link: a station id (`dedalus`) or index.
    pub station: Option<String>,
    /// Jump between stations instead of easing (prefers-reduced-motion).
    pub reduced_motion: bool,
    /// Start with the performance overlay shown.
    pub stats: bool,
    /// A touch screen (phones, tablets): hints talk about taps, not keys.
    pub touch: bool,
}

pub struct App {
    proxy: EventLoopProxy<AppEvent>,
    title: String,
    scene: Scene,
    timeline: Timeline,
    /// Screen-space buttons (native only; the web page has an HTML nav).
    buttons: Vec<Button>,
    /// The About panel: whether it's open, the hover group of its source
    /// link, and its session line (backend and GPU, known once the GPU is).
    about_open: bool,
    source: u32,
    session: String,
    /// Hover groups of the About panel's links to shaders.
    shaders: Vec<u32>,
    /// The performance overlay: frame statistics while it's shown, its text
    /// and graph, the About panel's switch for it and its own switches,
    /// whether the GPU can time its passes, and whether the last frame
    /// animated (or the app idles).
    stats: Option<Stats>,
    overlay: Vec<String>,
    graph: Vec<GraphFrame>,
    /// Keys and a mouse, or touch: what the hints talk about.
    input: Input,
    overlay_switch: u32,
    switches: Switches,
    /// Hover groups of the timeline rail's markers, in station order.
    rail: Vec<u32>,
    gpu_timing: bool,
    animating: bool,
    /// The overlay's switches: bloom's glow, and the x-ray view.
    glow: bool,
    xray: bool,
    window: Option<Arc<Window>>,
    state: Option<State>,
    last_frame: Instant,
    /// Whether a frame is being drawn, and whether another one is due once
    /// this event-loop iteration is over (see `about_to_wait`).
    drawing: bool,
    redraw_after: bool,
    /// Frames presented so far, and when the app started (frame counts at
    /// powers of two are logged, to see how fast a device draws).
    presented: u64,
    started: Instant,
    cursor: Option<PhysicalPosition<f64>>,
    /// Hover group under the cursor, and the one a mouse press started on.
    hovered: Option<u32>,
    pressed: Option<u32>,
    touch: Option<Touch>,
    /// Hover group with keyboard focus (Tab / Shift+Tab, Enter opens it).
    focused: Option<u32>,
    /// The last click or tap on empty space (half of a double-click).
    empty_press: Option<Press>,
    /// A skill pinned on the skill map (by a click or tap) until unpinned.
    pinned: Option<u32>,
    /// Where S / the Skills button flies back to from the skill map.
    skills_return: Option<usize>,
    modifiers: ModifiersState,
    /// The station in view, and the one last shown in the address bar and
    /// announced to screen readers (none yet at startup).
    station: usize,
    shown_station: Option<usize>,
    /// The particle intro's clock; `None` with reduced motion.
    intro: Option<Intro>,
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

/// A window position (physical pixels) in normalized device coordinates
/// (-1..1, y up).
fn ndc(position: PhysicalPosition<f64>, gpu: &Gpu) -> Vec2 {
    let (width, height) = (gpu.config.width as f32, gpu.config.height as f32);
    Vec2::new(
        2.0 * position.x as f32 / width - 1.0,
        1.0 - 2.0 * position.y as f32 / height,
    )
}

/// Where S goes from `current`, and where it goes back to next time: to the
/// skill map (remembering `current`), or from it back to where you came from
/// (staying if you arrived another way, e.g. by scrolling or a deep link).
fn skills_toggle(current: usize, skills: usize, back: Option<usize>) -> (usize, Option<usize>) {
    if current == skills {
        (back.unwrap_or(skills), None)
    } else {
        (skills, Some(current))
    }
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
        let input = if options.touch {
            Input::Touch
        } else {
            Input::Keyboard
        };
        let mut scene = Scene::build(&resume, scene::WIDE, input);
        let buttons = ui::native_buttons(&mut scene);
        let source = ui::source_link(&mut scene);
        let overlay_switch = ui::overlay_switch(&mut scene);
        let shaders = ui::shader_links(&mut scene);
        let switches = ui::overlay_switches(&mut scene);
        let rail = ui::rail_links(&mut scene);
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
            about_open: false,
            source,
            session: String::new(),
            shaders,
            stats: options.stats.then(Stats::default),
            input,
            overlay: Vec::new(),
            graph: Vec::new(),
            overlay_switch,
            switches,
            rail,
            gpu_timing: false,
            animating: false,
            glow: true,
            xray: false,
            window: None,
            state: None,
            last_frame: Instant::now(),
            drawing: false,
            redraw_after: false,
            presented: 0,
            started: Instant::now(),
            cursor: None,
            hovered: None,
            pressed: None,
            touch: None,
            focused: None,
            empty_press: None,
            pinned: None,
            skills_return: None,
            modifiers: ModifiersState::empty(),
            station: start,
            shown_station: None,
            // A deep link past the intro skips the assembly.
            intro: (!options.reduced_motion).then(|| Intro::new(start != 0)),
            #[cfg(target_arch = "wasm32")]
            tab_leaves: Default::default(),
        };
        #[cfg(target_arch = "wasm32")]
        {
            crate::platform::web::release_tab_at_edges(app.tab_leaves.clone());
            crate::platform::web::forward_nav_clicks(app.proxy.clone());
        }
        app.publish_tab_leaves();
        crate::debug::startup::mark("scene built");
        app
    }

    /// Asks for a frame. During a frame (e.g. focus clearing as the
    /// timeline settles), only once the frame is over: iOS ignores requests
    /// made while it draws (see `about_to_wait`).
    fn request_redraw(&mut self) {
        if self.drawing {
            self.redraw_after = true;
        } else if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            // Back from the background (phones): a new surface.
            if let Some(state) = &mut self.state {
                state.gpu.resume();
            }
            self.layout();
            self.request_redraw();
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
        crate::debug::startup::mark("window created");

        let display = event_loop.owned_display_handle();
        let proxy = self.proxy.clone();
        let init = async move {
            let gpu = Gpu::new(display, window).await.map(Box::new);
            let _ = proxy.send_event(AppEvent::GpuReady(gpu));
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(init);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(init);
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        // The performance overlay's refresh, due while nothing else redraws.
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.request_redraw();
        }
    }

    /// Requests the next frame of an animation. Not from within the frame
    /// itself: on iOS, winit redraws via `setNeedsDisplay`, which UIKit
    /// ignores while it's drawing, so the animation would stop after one
    /// frame. With the performance overlay shown, it also wakes up for the
    /// overlay's next refresh.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if std::mem::take(&mut self.redraw_after)
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
        event_loop.set_control_flow(match &self.stats {
            Some(stats) => ControlFlow::WaitUntil(stats.next_refresh(Instant::now())),
            None => ControlFlow::Wait,
        });
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // Android destroys the window's surface while in the background.
        if let Some(state) = &mut self.state {
            state.gpu.suspend();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::GpuReady(Ok(gpu)) => {
                self.session = about::session(&gpu.context.adapter);
                if !gpu.context.compute {
                    // No particles: the crisp title from the start.
                    self.intro = None;
                }
                self.fit_layout(gpu.aspect());
                let renderer = Renderer::new(&gpu.context, &self.scene, self.intro.is_some());
                let mut state = State {
                    lens: self.scene.lens(gpu.aspect()),
                    ui: UiLayer::default(),
                    gpu: *gpu,
                    renderer,
                };
                if self.stats.is_some() {
                    self.gpu_timing = state.renderer.set_timing(&state.gpu.context, true);
                }
                self.state = Some(state);
                self.layout();
                self.last_frame = Instant::now();
                // The first frame right away, not via a redraw request: a
                // hidden desktop window gets none (`redraw` shows it).
                self.drawing = true;
                self.redraw();
                self.drawing = false;
            }
            #[cfg(target_arch = "wasm32")]
            AppEvent::ToggleSkills => self.toggle_skills(),
            #[cfg(target_arch = "wasm32")]
            AppEvent::ToggleAbout => self.toggle_about(),
            AppEvent::GpuReady(Err(error)) => {
                log::error!("WebGPU initialization failed: {error}");
                #[cfg(target_arch = "wasm32")]
                crate::platform::web::show_plain_version();
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
            WindowEvent::RedrawRequested => {
                self.drawing = true;
                self.redraw();
                self.drawing = false;
            }
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
                self.wake_intro();
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.update_hover();
                self.wake_intro();
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

/// Tells screen readers (web: the page's live region). The native app has
/// no screen-reader bridge yet.
fn announce(text: &str) {
    #[cfg(target_arch = "wasm32")]
    crate::platform::web::announce(text);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = text;
}

#[cfg(not(target_arch = "wasm32"))]
fn window_attributes(title: &str) -> winit::window::WindowAttributes {
    let attributes = Window::default_attributes()
        .with_title(title)
        .with_window_icon(crate::platform::icon::window_icon());
    // Phones: the whole screen (iOS would otherwise make the view this size).
    if cfg!(any(target_os = "android", target_os = "ios")) {
        attributes
    } else {
        // Hidden until its first frame (see `reveal`), instead of an empty
        // white window while the GPU starts.
        attributes
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0))
            .with_visible(false)
    }
}

/// Shows the desktop window, created hidden, once it has something to show.
fn reveal(window: &Window) {
    if !cfg!(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios"
    )) && window.is_visible() == Some(false)
    {
        window.set_visible(true);
    }
}

/// How far screen-space controls stay in from the window's edges: on the
/// web, the page's nav covers the bottom-right corner.
#[cfg(target_arch = "wasm32")]
fn safe_insets(_window: &Window) -> Insets {
    Insets {
        bottom: crate::platform::web::nav_height(),
        ..Insets::default()
    }
}

/// How far screen-space controls stay in from the window's edges: iOS draws
/// the scene under the notch, the home indicator and rounded corners,
/// outside winit's safe area (`inner_*`).
#[cfg(not(target_arch = "wasm32"))]
fn safe_insets(window: &Window) -> Insets {
    if !cfg!(target_os = "ios") {
        return Insets::default();
    }
    let (Ok(safe), Ok(screen)) = (window.inner_position(), window.outer_position()) else {
        return Insets::default();
    };
    let (safe_size, size) = (window.inner_size(), window.outer_size());
    let (left, top) = (safe.x - screen.x, safe.y - screen.y);
    let right = size.width as i32 - left - safe_size.width as i32;
    let bottom = size.height as i32 - top - safe_size.height as i32;
    Insets {
        top: top.max(0) as f32,
        right: right.max(0) as f32,
        bottom: bottom.max(0) as f32,
        left: left.max(0) as f32,
    }
}

#[cfg(target_arch = "wasm32")]
fn window_attributes(title: &str) -> winit::window::WindowAttributes {
    use winit::platform::web::WindowAttributesExtWebSys;
    Window::default_attributes()
        .with_title(title)
        .with_canvas(crate::platform::web::canvas())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s_flies_to_the_skill_map_and_back() {
        let skills = 12;
        assert_eq!(skills_toggle(3, skills, None), (skills, Some(3)));
        assert_eq!(skills_toggle(skills, skills, Some(3)), (3, None));
        // Arrived by scrolling: stays.
        assert_eq!(skills_toggle(skills, skills, None), (skills, None));
    }

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
