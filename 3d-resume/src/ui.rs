//! Screen-space layer in physical pixels (origin bottom-left, y up), drawn on
//! top of the scene with an orthographic projection and no fading. It holds
//! the About panel ("How this resume is built") and the performance overlay
//! on every platform, and in the native app the "ⓘ · Skills · Text version ·
//! PDF" buttons that the web page provides as HTML.

use glam::{Mat4, Vec3};
use resume_model::about::ABOUT;

use crate::scene::{Action, Link, Scene};
use crate::shapes::ShapeInstance;
use crate::text::{self, Font, GlyphInstance, Run, TextStyle, rgb};

const BUTTON_TEXT: [f32; 4] = rgb(0xC9D4DE);
const BUTTON_BORDER: [f32; 4] = rgb(0x3E5A73);
const BUTTON_FILL: [f32; 4] = [0.0005, 0.002, 0.004, 0.6];
/// Same as the web nav's focus outline.
const FOCUS: [f32; 4] = rgb(0x6DB3E8);
/// Opaque: blending is linear, so even a few percent of the scene's white
/// text would show through as readable grey.
const PANEL_FILL: [f32; 4] = rgb(0x0A121C);
const PANEL_TITLE: [f32; 4] = rgb(0xF2F6FA);
const SESSION: [f32; 4] = rgb(0x6FC2C9);
const MUTED: [f32; 4] = rgb(0x8AA0B4);

/// Margin around the controls and the panel, in logical pixels.
const MARGIN: f32 = 16.0;
/// Widths (logical pixels) the panel tries, narrowest first, before it
/// shrinks its text to fit the window's height.
const PANEL_WIDTHS: [f32; 3] = [460.0, 640.0, 820.0];

/// A screen-space button: its label and hover group.
pub type Button = (&'static str, u32);

/// The native "ⓘ" (About) / "Skills" / "Text version" / "PDF" buttons,
/// registered as scene actions. None on the web, where the page has an HTML
/// nav instead.
pub fn native_buttons(scene: &mut Scene) -> Vec<Button> {
    if cfg!(target_arch = "wasm32") {
        return Vec::new();
    }
    vec![
        ("i", scene.add_action(Action::ToggleAbout, ABOUT.title)),
        ("Skills", scene.add_action(Action::ToggleSkills, "Skills")),
        (
            "Text version",
            scene.add_action(Action::Open(Link::TextVersion), "Text version"),
        ),
        ("PDF", scene.add_action(Action::Open(Link::Pdf), "PDF")),
    ]
}

/// The About panel's link to the source code, registered as a scene action.
pub fn source_link(scene: &mut Scene) -> u32 {
    let action = Action::Open(Link::Url(ABOUT.source_url.to_owned()));
    scene.add_action(action, ABOUT.source_label)
}

/// The About panel's switch for the performance overlay, registered as a
/// scene action.
pub fn overlay_switch(scene: &mut Scene) -> u32 {
    scene.add_action(Action::ToggleStats, "Performance overlay")
}

/// How far (physical pixels) the controls stay in from the window's edges
/// beyond their margin, e.g. clear of a phone's notch and home indicator.
#[derive(Debug, Clone, Copy, Default)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// The open About panel: this session's graphics backend and GPU, the hover
/// groups of the source link and of the performance overlay's switch, and
/// whether the overlay is shown.
pub struct Panel<'a> {
    pub session: &'a str,
    pub source: u32,
    pub overlay: u32,
    pub overlay_shown: bool,
}

#[derive(Default)]
pub struct UiLayer {
    pub glyphs: Vec<GlyphInstance>,
    pub shapes: Vec<ShapeInstance>,
    hits: Vec<([f32; 4], u32)>,
    /// The panel's box: it hides the scene under it from the pointer.
    panel: Option<[f32; 4]>,
    /// The performance overlay's box: it hides the scene too.
    overlay: Option<[f32; 4]>,
}

impl UiLayer {
    /// The buttons in the bottom-right corner, the panel (if open) above
    /// them and the performance overlay's `lines` (if shown) in the top-left
    /// corner, sized in logical pixels times `scale` for a `width` × `height`
    /// window. The About button shows as pressed while the panel is open.
    pub fn new(
        [width, height]: [f32; 2],
        scale: f32,
        insets: Insets,
        buttons: &[Button],
        panel: Option<&Panel>,
        overlay: Option<&[String]>,
    ) -> Self {
        let mut layer = Self::default();
        let active = panel.map(|_| buttons.iter().find(|(label, _)| *label == "i"));
        let active = active.flatten().map(|&(_, group)| group);
        let row_top = layer.buttons(width, scale, insets, buttons, active);
        if let Some(panel) = panel {
            let gap = if buttons.is_empty() {
                0.0
            } else {
                10.0 * scale
            };
            let area = [
                MARGIN * scale + insets.left,
                row_top + gap,
                width - MARGIN * scale - insets.right,
                height - MARGIN * scale - insets.top,
            ];
            layer.panel(area, scale, panel);
        }
        if let Some(lines) = overlay {
            let top_left = [
                MARGIN * scale + insets.left,
                height - MARGIN * scale - insets.top,
            ];
            let max_width = width - 2.0 * MARGIN * scale - insets.left - insets.right;
            layer.overlay(top_left, max_width, scale, lines);
        }
        layer
    }

    /// The performance overlay: its heading and lines in a box, top-left.
    fn overlay(&mut self, [left, top]: [f32; 2], max_width: f32, scale: f32, lines: &[String]) {
        let pad = 10.0 * scale;
        let size = 12.5 * scale;
        let style = TextStyle::new(Font::Regular, size, BUTTON_TEXT).wrap(max_width - 2.0 * pad);
        let width = lines
            .iter()
            .map(|line| text::measure(line, style)[0])
            .fold(0.0, f32::max)
            + 2.0 * pad;
        let mut glyphs = Vec::new();
        let mut cursor = Vec3::new(left + pad, top - pad, 0.0);
        for (i, line) in lines.iter().enumerate() {
            let style = if i == 0 {
                TextStyle {
                    font: Font::Bold,
                    color: PANEL_TITLE,
                    ..style
                }
            } else {
                style
            };
            cursor.y -= text::layout(line, style, cursor, &mut glyphs) + 3.0 * scale;
        }
        let bottom = cursor.y + 3.0 * scale - pad;
        let rect = [left, bottom, left + width, top];
        let radius = 8.0 * scale;
        self.shapes
            .push(ShapeInstance::filled(rect, 0.0, radius, PANEL_FILL));
        self.shapes.push(ShapeInstance::outlined(
            rect,
            0.0,
            radius,
            scale,
            BUTTON_BORDER,
        ));
        self.glyphs.extend(glyphs);
        self.overlay = Some(rect);
    }

    /// Pill buttons in the bottom-right corner, right to left in the given
    /// order; a one-letter label gets a round button. Returns the row's top
    /// edge (or the bottom margin's, without buttons).
    fn buttons(
        &mut self,
        width: f32,
        scale: f32,
        insets: Insets,
        buttons: &[Button],
        active: Option<u32>,
    ) -> f32 {
        let (size, pad_x, pad_y, gap, margin) = (
            14.0 * scale,
            14.0 * scale,
            9.0 * scale,
            8.0 * scale,
            MARGIN * scale,
        );
        let height = size * 1.2 + 2.0 * pad_y;
        let bottom = margin + insets.bottom;
        let mut right = width - margin - insets.right;
        for &(label, group) in buttons.iter().rev() {
            let style = TextStyle::new(Font::Bold, size, BUTTON_TEXT).group(group);
            let text_width = text::width(Font::Bold, size, label);
            let w = (text_width + 2.0 * pad_x).max(height);
            let rect = [right - w, bottom, right, bottom + height];
            let radius = height / 2.0;
            let border = if active == Some(group) {
                FOCUS
            } else {
                BUTTON_BORDER
            };
            // The fill stays out of the hover group: highlighting would
            // lighten it and lower the label's contrast.
            self.shapes
                .push(ShapeInstance::filled(rect, 0.0, radius, BUTTON_FILL));
            self.shapes
                .push(ShapeInstance::outlined(rect, 0.0, radius, scale, border).group(group));
            let [x0, y0, x1, y1] = rect;
            let ring = 3.0 * scale;
            self.shapes.push(ShapeInstance::focus_ring(
                [x0 - ring, y0 - ring, x1 + ring, y1 + ring],
                0.0,
                radius + ring,
                2.0 * scale,
                FOCUS,
                group,
            ));
            let top = Vec3::new(
                right - (w + text_width) / 2.0,
                bottom + height - pad_y * 0.8,
                0.0,
            );
            text::layout(label, style, top, &mut self.glyphs);
            self.hits.push((rect, group));
            right -= w + gap;
        }
        if buttons.is_empty() {
            bottom
        } else {
            bottom + height
        }
    }

    /// The About panel, anchored to the bottom-right of `area` ([x0, y0, x1,
    /// y1]): as narrow as fits the window's height, else with smaller text.
    fn panel(&mut self, area: [f32; 4], scale: f32, panel: &Panel) {
        let [x0, y0, x1, y1] = area;
        let (max_width, max_height) = (x1 - x0, y1 - y0);
        let fit = |width: f32, text_scale: f32| {
            let content = PanelContent::layout(width, scale * text_scale, panel);
            (content.height <= max_height).then_some(content)
        };
        let content = PANEL_WIDTHS
            .iter()
            .map(|w| (w * scale).min(max_width))
            .find_map(|width| fit(width, 1.0))
            .or_else(|| {
                // Too little height even at full width: shrink the text.
                let width = (PANEL_WIDTHS[2] * scale).min(max_width);
                let full = PanelContent::layout(width, scale, panel);
                let text_scale = (max_height / full.height).clamp(0.5, 1.0);
                Some(PanelContent::layout(width, scale * text_scale, panel))
            })
            .expect("a layout");
        // Move the content, laid out from the origin (top-left), into place.
        let (left, top) = (x1 - content.width, y0 + content.height);
        let rect = [left, y0, x1, top];
        let radius = 12.0 * scale;
        self.shapes
            .push(ShapeInstance::filled(rect, 0.0, radius, PANEL_FILL));
        self.shapes.push(ShapeInstance::outlined(
            rect,
            0.0,
            radius,
            scale,
            BUTTON_BORDER,
        ));
        let offset = |[a, b, c, d]: [f32; 4]| [a + left, b + top, c + left, d + top];
        self.glyphs
            .extend(content.glyphs.into_iter().map(|mut glyph| {
                glyph.rect = offset(glyph.rect);
                glyph
            }));
        self.shapes
            .extend(content.shapes.into_iter().map(|mut shape| {
                shape.rect = offset(shape.rect);
                shape
            }));
        self.hits.extend(
            content
                .hits
                .into_iter()
                .map(|(rect, group)| (offset(rect), group)),
        );
        self.panel = Some(rect);
    }

    /// The hover group of the button or link at `(x, y)` (physical pixels,
    /// y up).
    pub fn pick(&self, x: f32, y: f32) -> Option<u32> {
        self.hits
            .iter()
            .find(|(rect, _)| contains(*rect, x, y))
            .map(|&(_, group)| group)
    }

    /// Whether `(x, y)` is on the open panel or the overlay (which hide the
    /// scene there).
    pub fn covers(&self, x: f32, y: f32) -> bool {
        [self.panel, self.overlay]
            .into_iter()
            .flatten()
            .any(|rect| contains(rect, x, y))
    }

    /// Maps physical pixels (origin bottom-left) to clip space.
    pub fn projection(width: f32, height: f32) -> Mat4 {
        glam::camera::rh::proj::directx::orthographic(0.0, width, 0.0, height, -1.0, 1.0)
    }
}

fn contains([x0, y0, x1, y1]: [f32; 4], x: f32, y: f32) -> bool {
    (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
}

/// The panel's text, laid out with its top-left corner at the origin.
struct PanelContent {
    glyphs: Vec<GlyphInstance>,
    shapes: Vec<ShapeInstance>,
    hits: Vec<([f32; 4], u32)>,
    width: f32,
    height: f32,
}

impl PanelContent {
    /// `scale`: logical to physical pixels, times any text shrinking.
    fn layout(width: f32, scale: f32, panel: &Panel) -> Self {
        let pad = 18.0 * scale;
        let wrap = width - 2.0 * pad;
        let mut content = Self {
            glyphs: Vec::new(),
            shapes: Vec::new(),
            hits: Vec::new(),
            width,
            height: 0.0,
        };
        let mut cursor = Vec3::new(pad, -pad, 0.0);
        let mut text = |content: &mut Self, runs: &[Run], style: TextStyle, gap: f32| {
            let paragraph = text::layout_runs(runs, style, cursor, &mut content.glyphs);
            cursor.y -= paragraph.height + gap * scale;
            paragraph
        };
        let run = |text, font, color| Run {
            text,
            font,
            color,
            group: 0,
        };
        let title = TextStyle::new(Font::Bold, 17.0 * scale, PANEL_TITLE).wrap(wrap);
        text(
            &mut content,
            &[run(ABOUT.title, Font::Bold, PANEL_TITLE)],
            title,
            12.0,
        );
        let heading = TextStyle::new(Font::Bold, 13.0 * scale, FOCUS).wrap(wrap);
        let body = TextStyle::new(Font::Regular, 14.0 * scale, BUTTON_TEXT)
            .wrap(wrap)
            .line_spacing(1.2);
        for section in ABOUT.sections {
            text(
                &mut content,
                &[run(section.heading, Font::Bold, FOCUS)],
                heading,
                4.0,
            );
            text(
                &mut content,
                &[run(section.text, Font::Regular, BUTTON_TEXT)],
                body,
                12.0,
            );
        }
        text(
            &mut content,
            &[
                run("This session: ", Font::Bold, SESSION),
                run(panel.session, Font::Regular, SESSION),
            ],
            body,
            12.0,
        );
        let link = Run {
            group: panel.source,
            ..run(ABOUT.source_label, Font::Bold, FOCUS)
        };
        let size = body.size;
        let mut boxes = text(&mut content, &[link], body, 8.0).boxes;
        let switch = Run {
            group: panel.overlay,
            ..run(
                if panel.overlay_shown {
                    "Hide performance overlay"
                } else {
                    "Show performance overlay"
                },
                Font::Bold,
                FOCUS,
            )
        };
        let key = run(" (P)", Font::Regular, MUTED);
        boxes.extend(text(&mut content, &[switch, key], body, 0.0).boxes);
        for (group, [x0, y0, x1, y1]) in boxes {
            // Underline, focus ring and a slightly enlarged hit region, as
            // for links in the scene.
            let underline = y0 + (y1 - y0) * 0.17;
            content.shapes.push(
                ShapeInstance::filled([x0, underline - scale, x1, underline], 0.0, 0.0, FOCUS)
                    .group(group),
            );
            let ring = size * 0.25;
            let rect = [x0 - ring, y0 - ring, x1 + ring, y1 + ring];
            content.shapes.push(ShapeInstance::focus_ring(
                rect,
                0.0,
                size * 0.3,
                2.0 * scale,
                FOCUS,
                group,
            ));
            content.hits.push((rect, group));
        }
        content.height = -cursor.y + pad;
        content
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: [f32; 2] = [800.0, 600.0];

    #[test]
    fn picks_buttons_in_the_bottom_right_corner() {
        let buttons = [("Text version", 5), ("PDF", 6)];
        let layer = UiLayer::new(SIZE, 1.0, Insets::default(), &buttons, None, None);
        assert_eq!(layer.hits.len(), 2);
        // "PDF" is rightmost.
        let ([x0, y0, x1, y1], group) = layer.hits[0];
        assert_eq!(group, 6);
        assert!(x1 <= 800.0 && x0 < x1 && y0 < y1);
        assert_eq!(layer.pick((x0 + x1) / 2.0, (y0 + y1) / 2.0), Some(6));
        assert_eq!(layer.pick(10.0, 590.0), None);
    }

    #[test]
    fn every_control_gets_a_hover_group() {
        // Groups are shared with the scene's links and skill-map nodes.
        let mut scene = Scene::new(&crate::content::resume());
        let buttons = native_buttons(&mut scene);
        let source = source_link(&mut scene);
        let overlay = overlay_switch(&mut scene);
        assert!(source != 0 && overlay != 0);
        assert!(buttons.iter().all(|&(_, group)| group != 0));
    }

    #[test]
    fn the_overlay_sits_top_left_and_hides_the_scene() {
        let lines = ["Performance", "60 fps · frame 16.7 ms (max 18.2)"].map(String::from);
        let insets = Insets {
            top: 50.0,
            left: 20.0,
            ..Insets::default()
        };
        let layer = UiLayer::new(SIZE, 1.0, insets, &[], None, Some(&lines));
        let [x0, _, x1, y1] = layer.overlay.expect("shown");
        assert_eq!((x0, y1), (MARGIN + 20.0, 600.0 - MARGIN - 50.0));
        assert!(x1 < 400.0 && layer.covers(x0 + 5.0, y1 - 5.0));
        assert!(!layer.glyphs.is_empty());
    }

    #[test]
    fn a_one_letter_button_is_round() {
        let layer = UiLayer::new(SIZE, 1.0, Insets::default(), &[("i", 4)], None, None);
        let [x0, y0, x1, y1] = layer.hits[0].0;
        assert!((x1 - x0 - (y1 - y0)).abs() < 0.01);
    }

    #[test]
    fn keeps_buttons_inside_the_insets() {
        let insets = Insets {
            right: 30.0,
            bottom: 40.0,
            ..Insets::default()
        };
        let layer = UiLayer::new(SIZE, 1.0, insets, &[("PDF", 6)], None, None);
        let [_, y0, x1, _] = layer.hits[0].0;
        assert!(x1 <= 800.0 - 30.0 - MARGIN && y0 >= 40.0 + MARGIN);
    }

    /// The open panel with the given buttons in a `size` window.
    fn open(size: [f32; 2], insets: Insets, buttons: &[Button]) -> UiLayer {
        let panel = Panel {
            session: "Vulkan on NVIDIA GeForce GTX 960M",
            source: 9,
            overlay: 10,
            overlay_shown: false,
        };
        UiLayer::new(size, 1.0, insets, buttons, Some(&panel), None)
    }

    #[test]
    fn the_panel_sits_above_the_buttons_and_fits_the_window() {
        let buttons = [("i", 4), ("PDF", 6)];
        let insets = Insets {
            top: 50.0,
            bottom: 30.0,
            ..Insets::default()
        };
        // Desktop, phone (portrait and landscape), and a tiny window.
        for size in [
            [1280.0, 800.0],
            [390.0, 844.0],
            [844.0, 390.0],
            [300.0, 200.0],
        ] {
            let layer = open(size, insets, &buttons);
            let [x0, y0, x1, y1] = layer.panel.expect("open");
            let row_top = layer.hits[0].0[3];
            assert!(y0 > row_top, "{size:?}: panel overlaps the buttons");
            assert!(x0 >= MARGIN && x1 <= size[0] - MARGIN, "{size:?}: too wide");
            // In a tiny window, the panel sticks out: the text only shrinks
            // so far.
            if size[1] > 300.0 {
                assert!(y1 <= size[1] - insets.top, "{size:?}: under the top inset");
                // Every glyph above the buttons is inside the panel.
                let panel_glyphs = layer.glyphs.iter().filter(|g| g.rect[1] > row_top);
                assert!(
                    panel_glyphs.clone().count() > 100,
                    "{size:?}: panel text missing"
                );
                assert!(panel_glyphs.clone().all(|g| {
                    let [gx0, gy0, gx1, gy1] = g.rect;
                    gx0 >= x0 && gx1 <= x1 && gy0 >= y0 && gy1 <= y1
                }));
            }
        }
    }

    #[test]
    fn the_panel_link_is_clickable_and_its_box_covers_the_scene() {
        let layer = open([1280.0, 800.0], Insets::default(), &[("i", 4)]);
        for group in [9, 10] {
            let (rect, _) = *layer.hits.iter().find(|(_, g)| *g == group).expect("link");
            let [x0, y0, x1, y1] = rect;
            let (x, y) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            assert_eq!(layer.pick(x, y), Some(group));
        }
        let [px0, py0, _, py1] = layer.panel.expect("open");
        assert!(layer.covers(px0 + 2.0, (py0 + py1) / 2.0));
        assert!(!layer.covers(10.0, 790.0));
        // The web has no buttons: the panel sits in the corner.
        let web = open([1280.0, 800.0], Insets::default(), &[]);
        assert_eq!(web.panel.expect("open")[1], MARGIN);
    }
}
