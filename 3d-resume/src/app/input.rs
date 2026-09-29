//! Keyboard, mouse and touch input, and fullscreen.

use super::*;

impl App {
    /// The mouse on the title's plane, while it is near the title on the
    /// intro (the particles make way for it).
    pub(super) fn intro_pointer(&self) -> Option<Vec3> {
        let state = self.state.as_ref()?;
        let cursor = self.cursor?;
        let position = self.timeline.position();
        if intro::scatter(position) > 0.3 {
            return None;
        }
        let camera = self.scene.camera(position, &state.lens);
        let title = &self.scene.title;
        let (_, point) = camera.ray_to_plane(ndc(cursor, &state.gpu), title.z)?;
        let [x0, y0, x1, y1] = title.bounds;
        let r = particles::POINTER_RADIUS;
        let near = (x0 - r..=x1 + r).contains(&point.x) && (y0 - r..=y1 + r).contains(&point.y);
        near.then_some(point)
    }

    /// Redraws when the mouse moved the particles' pointer, so they react.
    pub(super) fn wake_intro(&mut self) {
        if self
            .intro
            .as_ref()
            .is_some_and(|intro| intro.pointer() != self.intro_pointer())
        {
            self.request_redraw();
        }
    }

    /// A click or tap on empty space: closes the About panel (unless it's
    /// on the panel), or unpins a skill; the second of a quick pair toggles
    /// fullscreen.
    pub(super) fn press_empty(&mut self, position: PhysicalPosition<f64>) {
        if self.on_panel(position) {
            return;
        }
        if self.about_open {
            return self.set_about(false);
        }
        if self.pinned.take().is_some() {
            self.update_groups();
        }
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
    pub(super) fn toggle_fullscreen(&self) {
        #[cfg(target_arch = "wasm32")]
        crate::platform::web::toggle_fullscreen();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(window) = &self.window {
            window.set_fullscreen(match window.fullscreen() {
                Some(_) => None,
                None => Some(winit::window::Fullscreen::Borderless(None)),
            });
        }
    }

    /// Native Esc; browsers handle Esc in fullscreen themselves.
    pub(super) fn leave_fullscreen(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(window) = &self.window
            && window.fullscreen().is_some()
        {
            window.set_fullscreen(None);
        }
    }

    pub(super) fn keyboard(&mut self, event: &KeyEvent) {
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
                if self.about_open {
                    self.set_about(false);
                } else if self.focused.is_some() {
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
                if !event.repeat
                    && !(self.modifiers.control_key()
                        || self.modifiers.alt_key()
                        || self.modifiers.super_key()) =>
            {
                if c.eq_ignore_ascii_case("f") {
                    self.toggle_fullscreen();
                } else if c.eq_ignore_ascii_case("s") {
                    self.toggle_skills();
                } else if c.eq_ignore_ascii_case("i") {
                    self.toggle_about();
                } else if c.eq_ignore_ascii_case("p") {
                    self.toggle_stats();
                } else if self.stats.is_some() && c.eq_ignore_ascii_case("x") {
                    self.toggle_xray();
                } else if self.stats.is_some()
                    && c.eq_ignore_ascii_case("b")
                    && self
                        .state
                        .as_ref()
                        .is_some_and(|state| state.renderer.has_bloom())
                {
                    self.toggle_glow();
                }
                return;
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

    pub(super) fn touch(&mut self, phase: TouchPhase, location: PhysicalPosition<f64>) {
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
