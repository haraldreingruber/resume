//! Keyboard focus: Tab and Shift+Tab move through the links of the station
//! in view, then the screen-space buttons; Enter opens the focused one.
//!
//! On the web, Tab leaves the canvas past either end so the page's own links
//! (Text version, PDF) stay reachable; natively, focus wraps around.

use crate::scene::Scene;

/// Whether focus wraps around (native) instead of leaving the canvas (web).
pub const WRAPS: bool = !cfg!(target_arch = "wasm32");

/// What a Tab press does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Focus the target at this index (`None`: nothing focused).
    To(Option<usize>),
    /// Leave the canvas (web): the browser moves focus to the next element.
    Leave,
}

/// Where Tab (`forward`) or Shift+Tab moves the focus among `count` targets.
/// With nothing focused, Tab starts at the first target and Shift+Tab at the
/// last; past either end, focus wraps around or leaves.
pub fn step(current: Option<usize>, count: usize, forward: bool, wrap: bool) -> Step {
    let next = match (current, forward) {
        _ if count == 0 => None,
        (None, true) => Some(0),
        (None, false) => Some(count - 1),
        (Some(i), true) => Some(i + 1).filter(|&n| n < count),
        (Some(i), false) => i.checked_sub(1),
    };
    match next {
        Some(i) => Step::To(Some(i)),
        None if !wrap => Step::Leave,
        None if count == 0 => Step::To(None),
        None => Step::To(Some(if forward { 0 } else { count - 1 })),
    }
}

/// The focus targets at a station: its links, then the screen-space buttons.
pub fn targets(scene: &Scene, station: usize, buttons: &[(&str, u32)]) -> Vec<u32> {
    scene
        .links(station)
        .chain(buttons.iter().map(|&(_, group)| group))
        .filter(|&group| group != 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_the_canvas_past_either_end_on_the_web() {
        let step = |current, forward| step(current, 3, forward, false);
        assert_eq!(step(None, true), Step::To(Some(0)));
        assert_eq!(step(None, false), Step::To(Some(2)));
        assert_eq!(step(Some(0), true), Step::To(Some(1)));
        assert_eq!(step(Some(2), true), Step::Leave);
        assert_eq!(step(Some(1), false), Step::To(Some(0)));
        assert_eq!(step(Some(0), false), Step::Leave);
        assert_eq!(super::step(None, 0, true, false), Step::Leave);
    }

    #[test]
    fn wraps_around_natively() {
        let step = |current, forward| step(current, 3, forward, true);
        assert_eq!(step(Some(2), true), Step::To(Some(0)));
        assert_eq!(step(Some(0), false), Step::To(Some(2)));
        assert_eq!(super::step(None, 0, true, true), Step::To(None));
    }

    #[test]
    fn targets_are_the_station_links_then_the_buttons() {
        let mut scene = Scene::new(&crate::content::resume());
        let outro = scene.station_count() - 1;
        let pdf = scene.add_action(crate::scene::Action::Open(crate::scene::Link::Pdf));
        let targets = targets(&scene, outro, &[("PDF", pdf)]);
        assert_eq!(targets.len(), scene.links(outro).len() + 1);
        assert_eq!(targets.last(), Some(&pdf));
        // The buttons are targets at every station.
        let intro = super::targets(&scene, 0, &[("PDF", pdf)]);
        assert_eq!(intro.len(), scene.links(0).len() + 1);
    }
}
