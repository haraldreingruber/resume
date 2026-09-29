//! Drawing frames and fitting the layout to the window: the frame
//! loop, the station in view, and the scene rebuilt for the other layout.

use super::*;

impl App {
    pub(super) fn redraw(&mut self) {
        let now = Instant::now();
        if self
            .stats
            .as_mut()
            .is_some_and(|stats| stats.refresh_due(now))
        {
            self.refresh_overlay(now);
        }
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let moving = self.timeline.update(dt);
        let position = self.timeline.position();
        let pointer = self.intro_pointer();
        let step = self
            .intro
            .as_mut()
            .and_then(|intro| intro.advance(dt, intro::scatter(position), pointer));
        let title_opacity = self.intro.as_ref().map_or(1.0, Intro::title_opacity);
        let intro_active = self.intro.as_ref().is_some_and(Intro::active);

        let Some(State {
            gpu,
            renderer,
            lens,
            ..
        }) = &mut self.state
        else {
            return;
        };
        if let Some(step) = &step {
            renderer.step_particles(&gpu.context, step);
        }
        renderer.set_title_opacity(&gpu.context, title_opacity);
        let camera = self.scene.camera(position, lens);
        let size = [gpu.config.width, gpu.config.height];
        let projection = UiLayer::projection(size[0] as f32, size[1] as f32);
        // CPU time: updating and encoding, not waiting for the surface.
        let prepared = now.elapsed();
        let mut encoded = Duration::ZERO;
        let rendering = Instant::now();
        let presented = gpu.render(|ctx, view| {
            let start = Instant::now();
            renderer.draw(ctx, view, size, &camera, projection);
            encoded = start.elapsed();
        });
        // The rest of it: getting the surface texture and presenting it.
        let waited = rendering.elapsed().saturating_sub(encoded);
        // The first frame is ready: show the window. (Also when a frame was
        // skipped, in case a hidden window's surface can't present.)
        if let Some(window) = &self.window {
            reveal(window);
        }
        if presented {
            self.presented += 1;
            if self.presented == 1 {
                crate::debug::startup::mark("first frame");
            }
            if self.presented.is_power_of_two() {
                let (frames, seconds) = (self.presented, self.started.elapsed().as_secs_f32());
                log::info!("{frames} frames presented after {seconds:.1} s");
            }
            if let Some(stats) = &mut self.stats {
                let ms = |time: Duration| time.as_secs_f32() * 1000.0;
                // `animating` is still the previous frame's: did it ask for this one?
                stats.frame(now, ms(prepared + encoded), ms(waited), self.animating);
            }
        }
        if let Some(stats) = &mut self.stats
            && let Some(times) = renderer.gpu_times(&gpu.context)
        {
            stats.gpu(times);
        }
        self.animating = moving || intro_active;
        // Keep redrawing while the timeline or the particles move, and retry
        // a frame the surface skipped (e.g. right after the first
        // `configure()`) so a skipped frame is never the last one drawn.
        self.redraw_after |= moving || intro_active || !presented;
        self.follow_station(!moving);
        if moving {
            // The link under a resting cursor changes as the camera moves.
            self.update_hover();
        }
    }

    /// Keeps station-dependent state in step with the timeline: keyboard
    /// focus stays within the station in view, and once the timeline has
    /// `settled`, the web address names that station.
    pub(super) fn follow_station(&mut self, settled: bool) {
        let station = self.timeline.nearest();
        if station != self.station {
            self.station = station;
            // A pinned skill only stays pinned while the map is in view.
            if self.pinned.take().is_some() {
                self.update_groups();
            }
            if self.focused.is_some_and(|g| !self.targets().contains(&g)) {
                self.set_focus(None);
            } else {
                self.publish_tab_leaves();
            }
            // The rail highlights the new station.
            self.update_ui();
        }
        if settled && Some(station) != self.shown_station {
            self.shown_station = Some(station);
            #[cfg(target_arch = "wasm32")]
            crate::platform::web::show_station(
                Some(station)
                    .filter(|&s| s > 0)
                    .and_then(|s| self.scene.station_id(s)),
            );
            // Native screen readers hear the tree's station node change (with
            // the links' final positions).
            #[cfg(not(accessibility))]
            if let Some(summary) = self.scene.summary(station).map(str::to_owned) {
                self.announce(&summary);
            }
            #[cfg(accessibility)]
            self.update_accessibility();
        }
    }

    /// Rebuilds size-dependent state: camera lens and screen-space layer.
    pub(super) fn layout(&mut self) {
        let Some(aspect) = self.state.as_ref().map(|state| state.gpu.aspect()) else {
            return;
        };
        let rebuilt = self.fit_layout(aspect);
        let Some(state) = &mut self.state else { return };
        if rebuilt {
            state.renderer.set_scene(&state.gpu.context, &self.scene);
            if self.xray {
                state
                    .renderer
                    .set_xray(&state.gpu.context, &self.scene.xray());
            }
        }
        state.lens = self.scene.lens(aspect);
        self.update_ui();
        if rebuilt {
            self.publish_tab_leaves();
            self.request_redraw();
        }
    }

    /// Rebuilds the scene when the screen's shape calls for the other layout
    /// (wide or compact, e.g. after rotating a phone); returns whether it
    /// did. Hover groups stay the same (same content, same order); hover,
    /// focus and pins reset.
    pub(super) fn fit_layout(&mut self, aspect: f32) -> bool {
        let metrics = Metrics::for_aspect(aspect);
        if metrics == self.scene.metrics() {
            return false;
        }
        let mut scene = Scene::build(&crate::content::resume(), metrics, self.input);
        self.buttons = ui::native_buttons(&mut scene);
        self.source = ui::source_link(&mut scene);
        self.overlay_switch = ui::overlay_switch(&mut scene);
        self.shaders = ui::shader_links(&mut scene);
        self.switches = ui::overlay_switches(&mut scene);
        self.rail = ui::rail_links(&mut scene);
        self.scene = scene;
        (self.hovered, self.pressed, self.focused, self.pinned) = (None, None, None, None);
        // Swapping a running scene restarts the particles from their cloud:
        // let them fly straight to the new title (no second assembly). At
        // startup, before the first frame, the assembly plays as usual.
        if self.intro.is_some() && self.state.is_some() {
            self.intro = Some(Intro::new(true));
        }
        true
    }
}
