//! Keyboard focus, hover and what activating a link, button or skill-map
//! node does.

use super::*;

impl App {
    /// Keyboard focus targets: the performance overlay's switches (top-left,
    /// first), the open About panel's links, the links of the station in
    /// view, then the screen-space buttons.
    pub(super) fn targets(&self) -> Vec<u32> {
        let mut panels: Vec<u32> = self
            .overlay_switches()
            .iter()
            .map(|switch| switch.group)
            .collect();
        if self.about_open {
            panels.extend(&self.shaders);
            panels.extend([self.source, self.overlay_switch]);
        }
        focus::targets(&self.scene, self.station, &panels, &self.buttons)
    }

    pub(super) fn focus_index(&self, targets: &[u32]) -> Option<usize> {
        let focused = self.focused?;
        targets.iter().position(|&group| group == focused)
    }

    /// Tab (`forward`) or Shift+Tab.
    pub(super) fn move_focus(&mut self, forward: bool) {
        let targets = self.targets();
        let current = self.focus_index(&targets);
        match focus::step(current, targets.len(), forward, focus::WRAPS) {
            Step::To(index) => self.set_focus(index.map(|i| targets[i])),
            // The page's listener normally lets such a Tab through to the
            // browser before winit sees it.
            Step::Leave => self.set_focus(None),
        }
    }

    pub(super) fn set_focus(&mut self, group: Option<u32>) {
        self.focused = group;
        self.update_groups();
        self.publish_tab_leaves();
        if let Some(description) = group.and_then(|g| self.scene.describe(g)) {
            announce(&description);
        }
    }

    /// Tells the page whether the next Tab / Shift+Tab leaves the canvas.
    pub(super) fn publish_tab_leaves(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            let targets = self.targets();
            let current = self.focus_index(&targets);
            let leaves =
                |forward| focus::step(current, targets.len(), forward, focus::WRAPS) == Step::Leave;
            self.tab_leaves.set([leaves(true), leaves(false)]);
        }
    }

    /// Uploads the hovered and focused groups and the skill map's relations
    /// (of the hovered, else the focused, else the pinned node), and redraws.
    pub(super) fn update_groups(&mut self) {
        let relations = [self.hovered, self.focused, self.pinned]
            .into_iter()
            .find_map(|group| self.scene.relations(group));
        if let Some(state) = &mut self.state {
            state
                .renderer
                .set_groups(&state.gpu.context, self.hovered, self.focused, relations);
        }
        self.request_redraw();
    }

    /// The hover group at a cursor position: screen-space buttons and the
    /// panel's link first, then links in the scene (unless the panel hides
    /// them).
    pub(super) fn group_at(&self, position: PhysicalPosition<f64>) -> Option<u32> {
        let state = self.state.as_ref()?;
        let height = state.gpu.config.height as f32;
        let (x, y) = (position.x as f32, height - position.y as f32);
        state.ui.pick(x, y).or_else(|| {
            if state.ui.covers(x, y) {
                return None;
            }
            let camera = self.scene.camera(self.timeline.position(), &state.lens);
            self.scene.pick(&camera, ndc(position, &state.gpu))
        })
    }

    /// Whether a cursor position is on the open About panel.
    pub(super) fn on_panel(&self, position: PhysicalPosition<f64>) -> bool {
        self.state.as_ref().is_some_and(|state| {
            let height = state.gpu.config.height as f32;
            state
                .ui
                .covers(position.x as f32, height - position.y as f32)
        })
    }

    pub(super) fn update_hover(&mut self) {
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

    pub(super) fn activate(&mut self, group: u32) {
        match self.scene.action(group).cloned() {
            Some(Action::Open(link)) => crate::platform::links::open(&link),
            Some(Action::GoToStation(station)) => {
                self.timeline.go_to(station);
                self.request_redraw();
            }
            Some(Action::Pin) => {
                self.pinned = (self.pinned != Some(group)).then_some(group);
                self.update_groups();
            }
            Some(Action::ToggleSkills) => self.toggle_skills(),
            Some(Action::ToggleAbout) => self.toggle_about(),
            Some(Action::ToggleStats) => self.toggle_stats(),
            Some(Action::ToggleBloom) => self.toggle_glow(),
            Some(Action::ToggleXray) => self.toggle_xray(),
            None => {}
        }
    }

    /// S, the Skills button or link: flies to the skill map, or back.
    pub(super) fn toggle_skills(&mut self) {
        let (target, back) = skills_toggle(
            self.timeline.nearest(),
            self.scene.skills_station(),
            self.skills_return,
        );
        self.skills_return = back;
        self.timeline.go_to(target);
        self.request_redraw();
    }
}
