//! The screen-space layer's panels: the About panel and the performance
//! overlay with its switches (bloom, x-ray).

use super::*;

impl App {
    /// The performance overlay's switches, while it's shown: bloom (where
    /// the GPU has it) and the x-ray view.
    pub(super) fn overlay_switches(&self) -> Vec<Switch> {
        if self.stats.is_none() {
            return Vec::new();
        }
        let bloom = self
            .state
            .as_ref()
            .is_some_and(|state| state.renderer.has_bloom());
        let mut switches = Vec::new();
        if bloom {
            switches.push(Switch {
                label: "Bloom",
                key: "[B]",
                on: self.glow,
                group: self.switches.bloom,
            });
        }
        switches.push(Switch {
            label: "X-ray",
            key: "[X]",
            on: self.xray,
            group: self.switches.xray,
        });
        switches
    }

    /// Rebuilds the screen-space layer: the buttons, and the About panel if
    /// it's open.
    pub(super) fn update_ui(&mut self) {
        let switches = self.overlay_switches();
        let Some(state) = &mut self.state else { return };
        let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
        let insets = self
            .window
            .as_deref()
            .map_or_else(Insets::default, safe_insets);
        let size = [
            state.gpu.config.width as f32,
            state.gpu.config.height as f32,
        ];
        let panel = self.about_open.then(|| Panel {
            session: &self.session,
            shaders: &self.shaders,
            source: self.source,
            overlay: self.overlay_switch,
            overlay_shown: self.stats.is_some(),
            keys: self.input == Input::Keyboard,
        });
        let overlay = (!self.overlay.is_empty()).then(|| Overlay {
            lines: &self.overlay,
            graph: &self.graph,
            switches: &switches,
            keys: self.input == Input::Keyboard,
        });
        state.ui = UiLayer::new(
            size,
            scale,
            insets,
            &self.buttons,
            panel.as_ref(),
            overlay.as_ref(),
        );
        state.renderer.set_ui(&state.gpu.context, &state.ui);
    }

    /// Recomputes the performance overlay's text (twice a second while it's
    /// shown, or empty) and the screen-space layer showing it.
    pub(super) fn refresh_overlay(&mut self, now: Instant) {
        self.overlay = match (&self.stats, &self.state) {
            (Some(stats), Some(state)) => {
                let config = &state.gpu.config;
                let images = u64::from(config.desired_maximum_frame_latency) + 1;
                let info = stats::Info {
                    animating: self.animating,
                    gpu_timing: self.gpu_timing,
                    counts: state.renderer.counts(),
                    gpu_bytes: state.renderer.gpu_bytes(),
                    surface_bytes: u64::from(config.width) * u64::from(config.height) * 4 * images,
                    size: [config.width, config.height],
                    session: &self.session,
                };
                self.graph = stats.graph(now);
                stats::lines(&stats.summary(now), &info)
            }
            _ => Vec::new(),
        };
        self.update_ui();
    }

    /// B or the overlay's switch: bloom's glow on or off.
    pub(super) fn toggle_glow(&mut self) {
        self.set_glow(!self.glow);
        announce(if self.glow { "Bloom on" } else { "Bloom off" });
    }

    pub(super) fn set_glow(&mut self, on: bool) {
        self.glow = on;
        if let Some(state) = &mut self.state {
            state.renderer.set_glow(on);
        }
        self.update_ui();
        self.request_redraw();
    }

    /// X or the overlay's switch: the x-ray view on or off.
    pub(super) fn toggle_xray(&mut self) {
        self.set_xray(!self.xray);
        announce(if self.xray {
            "X-ray view on: outlines of every glyph, shape and click area"
        } else {
            "X-ray view off"
        });
    }

    pub(super) fn set_xray(&mut self, on: bool) {
        self.xray = on;
        if let Some(state) = &mut self.state {
            let outlines = if on { self.scene.xray() } else { Vec::new() };
            state.renderer.set_xray(&state.gpu.context, &outlines);
        }
        self.update_ui();
        self.request_redraw();
    }

    /// P or the About panel's switch: shows or hides the performance
    /// overlay (and times the GPU while it's shown).
    pub(super) fn toggle_stats(&mut self) {
        let show = self.stats.is_none();
        self.stats = show.then(Stats::default);
        if !show {
            // The overlay's switches go with it: back to the normal view.
            self.set_glow(true);
            self.set_xray(false);
        }
        if let Some(state) = &mut self.state {
            self.gpu_timing = state.renderer.set_timing(&state.gpu.context, show);
        }
        self.refresh_overlay(Instant::now());
        announce(if show {
            "Performance overlay shown"
        } else {
            "Performance overlay hidden"
        });
        self.update_hover();
        self.request_redraw();
    }

    /// I, the ⓘ button or the page's About button: opens or closes the
    /// About panel.
    pub(super) fn toggle_about(&mut self) {
        self.set_about(!self.about_open);
    }

    pub(super) fn set_about(&mut self, open: bool) {
        if open == self.about_open {
            return;
        }
        self.about_open = open;
        self.update_ui();
        if !open && self.focused == Some(self.source) {
            self.set_focus(None);
        } else {
            self.publish_tab_leaves();
        }
        #[cfg(target_arch = "wasm32")]
        crate::platform::web::show_about_expanded(open);
        if open {
            announce(&about::announcement(&self.session));
        }
        // The panel may now be under (or gone from under) the cursor.
        self.update_hover();
        self.request_redraw();
    }
}
