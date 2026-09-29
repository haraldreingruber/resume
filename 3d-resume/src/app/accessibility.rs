//! The app's side of native screen readers: what the accessibility tree
//! shows, and what screen readers ask the app to do.

use accesskit_winit::WindowEvent as AccessEvent;

use super::*;
use crate::platform::accessibility::{self, Accessibility, Contents, Kind, Target};

impl App {
    /// Connects screen readers to the window, which must still be hidden.
    pub(super) fn connect_accessibility(&mut self, event_loop: &ActiveEventLoop, window: &Window) {
        self.accessibility = Some(Accessibility::new(event_loop, window, self.proxy.clone()));
    }

    /// Hands a window event to the screen-reader bridge first.
    pub(super) fn accessibility_window_event(&mut self, event: &WindowEvent) {
        if let (Some(accessibility), Some(window)) = (&mut self.accessibility, &self.window) {
            accessibility.process_event(window, event);
        }
    }

    /// Rebuilds the tree, if a screen reader is listening.
    pub(super) fn update_accessibility(&mut self) {
        let Some(mut accessibility) = self.accessibility.take() else {
            return;
        };
        accessibility.update(|| {
            let targets = self.accessibility_targets();
            let panel = self.about_open.then(|| about::announcement(&self.session));
            accessibility::tree(&Contents {
                title: &self.title,
                station: self
                    .shown_station
                    .and_then(|station| self.scene.summary(station))
                    .unwrap_or_default(),
                announcement: &self.announcement,
                panel: panel.as_deref(),
                targets: &targets,
                focus: self.focused,
            })
        });
        self.accessibility = Some(accessibility);
    }

    /// A screen reader's request.
    pub(super) fn accessibility_event(&mut self, event: AccessEvent) {
        match event {
            AccessEvent::InitialTreeRequested => self.update_accessibility(),
            AccessEvent::ActionRequested(request) => {
                use accesskit::Action as Request;
                match (accessibility::group(request.target_node), request.action) {
                    (Some(group), Request::Focus) => self.set_focus(Some(group)),
                    (Some(group), Request::Click) => self.activate(group),
                    // E.g. VoiceOver's three-finger swipes: through time.
                    (None, Request::ScrollDown) => self.step_station(1),
                    (None, Request::ScrollUp) => self.step_station(-1),
                    _ => {}
                }
            }
            AccessEvent::AccessibilityDeactivated => {}
        }
    }

    fn step_station(&mut self, direction: i32) {
        self.timeline.step(direction);
        self.request_redraw();
    }

    /// The keyboard's focus targets, then the timeline rail's markers.
    fn accessibility_targets(&self) -> Vec<Target> {
        let mut groups = self.targets();
        groups.extend(&self.rail);
        groups
            .into_iter()
            .filter_map(|group| {
                let label = self.scene.label(group)?.to_owned();
                let (kind, description) = match self.scene.action(group)? {
                    Action::Open(scene::Link::Url(_)) => (Kind::Link, String::new()),
                    Action::GoToStation(_) => match self.scene.related(group) {
                        skills if skills.is_empty() => (Kind::Link, String::new()),
                        skills => (Kind::Link, format!("Skills: {skills}")),
                    },
                    Action::Pin => (
                        Kind::Button,
                        format!("Used at {}", self.scene.related(group)),
                    ),
                    Action::ToggleStats => (Kind::Switch(self.stats.is_some()), String::new()),
                    Action::ToggleBloom => (Kind::Switch(self.glow), String::new()),
                    Action::ToggleXray => (Kind::Switch(self.xray), String::new()),
                    Action::Open(_) | Action::ToggleSkills | Action::ToggleAbout => {
                        (Kind::Button, String::new())
                    }
                };
                Some(Target {
                    group,
                    label,
                    description,
                    kind,
                    bounds: self.bounds(group),
                })
            })
            .collect()
    }

    /// Where `group` is on screen (physical pixels, y down): a screen-space
    /// control, or a link in the scene where the camera is now.
    fn bounds(&self, group: u32) -> Option<[f32; 4]> {
        let state = self.state.as_ref()?;
        let (width, height) = (
            state.gpu.config.width as f32,
            state.gpu.config.height as f32,
        );
        if let Some([x0, y0, x1, y1]) = state.ui.bounds(group) {
            return Some([x0, height - y1, x1, height - y0]);
        }
        let camera = self.scene.camera(self.timeline.position(), &state.lens);
        let [x0, y0, x1, y1] = self.scene.screen_bounds(&camera, group)?;
        let x = |ndc: f32| (ndc.clamp(-1.0, 1.0) + 1.0) / 2.0 * width;
        let y = |ndc: f32| (1.0 - ndc.clamp(-1.0, 1.0)) / 2.0 * height;
        Some([x(x0), y(y1), x(x1), y(y0)])
    }
}
