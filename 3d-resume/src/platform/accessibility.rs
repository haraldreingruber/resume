//! Screen readers in the native apps (Windows, macOS, Linux and iOS)
//! through AccessKit. The canvas is exposed as a small accessibility tree:
//! the window; two live regions that speak what the web page's `#live`
//! region does: the station reached, and other announcements (the About
//! panel, switches); the About panel's text while it's open; and a node per
//! link, button, switch and timeline marker in reach, which screen readers
//! can focus and activate. Scrolling the
//! window (e.g. VoiceOver's three-finger swipe) moves through time.
//!
//! Android waits for a GameActivity build (AccessKit's Android adapter needs
//! it); the web has its own live region.

use accesskit::{Action, Live, Node, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate};
use accesskit_winit::Adapter;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::Window;

use crate::app::AppEvent;

/// Hover groups are their own node ids; these lie above any group.
const ROOT: NodeId = NodeId(1 << 32);
const STATION: NodeId = NodeId((1 << 32) + 1);
const LIVE: NodeId = NodeId((1 << 32) + 2);
const PANEL: NodeId = NodeId((1 << 32) + 3);

/// Something a screen reader can reach: its hover group, what it's called
/// and what else to say about it, what it is, and where (window pixels, y
/// down), if known.
pub struct Target {
    pub group: u32,
    pub label: String,
    pub description: String,
    pub kind: Kind,
    pub bounds: Option<[f32; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Link,
    Button,
    /// A switch, on or off.
    Switch(bool),
}

/// What the tree shows: the window's title, the station reached (its
/// summary), the last other announcement, the open About panel's text, the
/// targets, and the one with keyboard focus.
pub struct Contents<'a> {
    pub title: &'a str,
    pub station: &'a str,
    pub announcement: &'a str,
    pub panel: Option<&'a str>,
    pub targets: &'a [Target],
    pub focus: Option<u32>,
}

pub struct Accessibility {
    adapter: Adapter,
}

impl Accessibility {
    /// For `window`, which must still be hidden.
    pub fn new(
        event_loop: &ActiveEventLoop,
        window: &Window,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Self {
        Self {
            adapter: Adapter::with_event_loop_proxy(event_loop, window, proxy),
        }
    }

    /// Hands a window event to the platform adapter (before the app sees it).
    pub fn process_event(&mut self, window: &Window, event: &WindowEvent) {
        self.adapter.process_event(window, event);
    }

    /// Replaces the tree (it's small) if a screen reader is listening; only
    /// then is it built.
    pub fn update(&mut self, build: impl FnOnce() -> TreeUpdate) {
        self.adapter.update_if_active(build);
    }
}

/// The hover group a node stands for (none for the window and the text
/// nodes).
pub fn group(node: NodeId) -> Option<u32> {
    u32::try_from(node.0).ok().filter(|&group| group != 0)
}

/// The whole tree: the window with the station, the live region and the
/// targets.
pub fn tree(contents: &Contents) -> TreeUpdate {
    let mut root = Node::new(Role::Window);
    root.set_label(contents.title);
    root.add_action(Action::ScrollDown);
    root.add_action(Action::ScrollUp);
    let mut children = Vec::with_capacity(contents.targets.len() + 3);
    let mut nodes = Vec::with_capacity(contents.targets.len() + 4);
    // Text nodes, left out while empty. Live ones are spoken when they
    // change, or appear.
    let texts = [
        (STATION, contents.station, Live::Polite),
        (LIVE, contents.announcement, Live::Polite),
        (PANEL, contents.panel.unwrap_or_default(), Live::Off),
    ];
    for (id, text, live) in texts {
        if !text.is_empty() {
            let mut node = Node::new(Role::Label);
            node.set_value(text);
            if live != Live::Off {
                node.set_live(live);
            }
            children.push(id);
            nodes.push((id, node));
        }
    }
    for target in contents.targets {
        let id = NodeId(target.group.into());
        let mut node = Node::new(match target.kind {
            Kind::Link => Role::Link,
            Kind::Button => Role::Button,
            Kind::Switch(_) => Role::Switch,
        });
        node.set_label(target.label.as_str());
        if !target.description.is_empty() {
            node.set_description(target.description.as_str());
        }
        if let Kind::Switch(on) = target.kind {
            node.set_toggled(if on { Toggled::True } else { Toggled::False });
        }
        node.add_action(Action::Focus);
        node.add_action(Action::Click);
        if let Some([x0, y0, x1, y1]) = target.bounds {
            node.set_bounds(Rect {
                x0: x0.into(),
                y0: y0.into(),
                x1: x1.into(),
                y1: y1.into(),
            });
        }
        children.push(id);
        nodes.push((id, node));
    }
    root.set_children(children);
    nodes.push((ROOT, root));
    let focus = contents
        .focus
        .filter(|group| contents.targets.iter().any(|t| t.group == *group))
        .map_or(ROOT, |group| NodeId(group.into()));
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(ROOT)),
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// The same text again still has to change the live region to be spoken:
/// it alternates with a trailing no-break space.
pub fn vary(previous: &str, text: &str) -> String {
    if previous == text {
        format!("{text}\u{a0}")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(group: u32, kind: Kind) -> Target {
        Target {
            group,
            label: format!("target {group}"),
            description: String::new(),
            kind,
            bounds: None,
        }
    }

    fn contents<'a>(targets: &'a [Target], focus: Option<u32>) -> Contents<'a> {
        Contents {
            title: "Resume",
            station: "Dedalus",
            announcement: "Bloom off",
            panel: None,
            targets,
            focus,
        }
    }

    #[test]
    fn exposes_the_window_the_station_the_live_region_and_the_targets() {
        let targets = [
            target(3, Kind::Link),
            target(9, Kind::Button),
            target(11, Kind::Switch(true)),
        ];
        let update = tree(&contents(&targets, Some(9)));
        let ids: Vec<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [STATION, LIVE, NodeId(3), NodeId(9), NodeId(11), ROOT]);
        let node = |id| &update.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
        assert_eq!(
            node(ROOT).children(),
            [STATION, LIVE, NodeId(3), NodeId(9), NodeId(11)]
        );
        assert_eq!(node(NodeId(3)).role(), Role::Link);
        assert_eq!(node(NodeId(9)).role(), Role::Button);
        assert_eq!(node(NodeId(11)).role(), Role::Switch);
        assert_eq!(node(NodeId(11)).toggled(), Some(Toggled::True));
        assert_eq!(node(STATION).value(), Some("Dedalus"));
        assert_eq!(node(STATION).live(), Some(Live::Polite));
        assert_eq!(node(LIVE).value(), Some("Bloom off"));
        assert_eq!(node(LIVE).live(), Some(Live::Polite));
        assert_eq!(update.focus, NodeId(9));
    }

    #[test]
    fn the_open_panel_reads_before_the_targets() {
        let targets = [target(3, Kind::Link)];
        let update = tree(&Contents {
            panel: Some("How this resume is built"),
            ..contents(&targets, None)
        });
        let root = &update.nodes.iter().find(|(id, _)| *id == ROOT).unwrap().1;
        assert_eq!(root.children(), [STATION, LIVE, PANEL, NodeId(3)]);
        let panel = &update.nodes.iter().find(|(id, _)| *id == PANEL).unwrap().1;
        assert_eq!(panel.value(), Some("How this resume is built"));
    }

    #[test]
    fn empty_texts_are_left_out() {
        let targets = [target(3, Kind::Link)];
        let update = tree(&Contents {
            station: "",
            announcement: "",
            ..contents(&targets, None)
        });
        let root = &update.nodes.iter().find(|(id, _)| *id == ROOT).unwrap().1;
        assert_eq!(root.children(), [NodeId(3)]);
        assert_eq!(update.nodes.len(), 2);
    }

    #[test]
    fn focus_falls_back_to_the_window() {
        // No focus, or focus on something that's not a target (any more).
        let targets = [target(3, Kind::Link)];
        assert_eq!(tree(&contents(&targets, None)).focus, ROOT);
        assert_eq!(tree(&contents(&targets, Some(7))).focus, ROOT);
    }

    #[test]
    fn maps_nodes_back_to_groups() {
        assert_eq!(group(NodeId(12)), Some(12));
        assert_eq!(group(ROOT), None);
        assert_eq!(group(STATION), None);
        assert_eq!(group(LIVE), None);
        assert_eq!(group(PANEL), None);
    }

    #[test]
    fn repeated_announcements_still_change_the_live_region() {
        let first = vary("", "Bloom off");
        let second = vary(&first, "Bloom off");
        let third = vary(&second, "Bloom off");
        assert_ne!(first, second);
        assert_ne!(second, third);
        assert_eq!(second.trim_end_matches('\u{a0}'), "Bloom off");
    }
}
