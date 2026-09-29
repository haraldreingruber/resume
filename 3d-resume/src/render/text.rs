//! MSDF text: glyph metrics baked by `build.rs`, word-wrapped layout of styled
//! runs, and the per-glyph GPU instances drawn by `shaders/text.wgsl`.
//!
//! Text lies in a plane facing +z (world units), or in screen pixels for the
//! screen-space layer.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::render::atlas_codec;

// Generated metrics can happen to resemble constants like 1/π.
#[allow(clippy::approx_constant)]
mod baked {
    use super::{FontMetrics, GlyphInfo};
    include!(concat!(env!("OUT_DIR"), "/glyphs.rs"));
}

pub use baked::{ATLAS_SIZE, DISTANCE_RANGE_PX};

/// The baked MSDF atlas, stored as `atlas_codec` describes.
const ATLAS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/atlas.bin"));

/// The baked MSDF atlas as RGBA8 pixels (distance data, top row first).
pub fn atlas_rgba() -> Vec<u8> {
    let [width, height] = ATLAS_SIZE;
    atlas_codec::decode(ATLAS, width as usize, height as usize)
}

/// Index into `build.rs`'s font list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Regular = 0,
    Bold = 1,
}

pub struct FontMetrics {
    pub ascender: f32,
    pub descender: f32,
    pub line_height: f32,
}

pub struct GlyphInfo {
    font: u8,
    ch: char,
    advance: f32,
    /// Quad relative to the pen, in em: left, bottom, right, top.
    plane: [f32; 4],
    /// Atlas UVs: left, top, right, bottom.
    uv: [f32; 4],
    visible: bool,
}

fn glyph(font: Font, ch: char) -> Option<&'static GlyphInfo> {
    let key = (font as u8, ch);
    baked::GLYPHS
        .binary_search_by_key(&key, |g| (g.font, g.ch))
        .ok()
        .map(|i| &baked::GLYPHS[i])
}

fn metrics(font: Font) -> &'static FontMetrics {
    &baked::FONT_METRICS[font as usize]
}

/// One glyph quad; matches the vertex layout in `shaders/text.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GlyphInstance {
    /// Quad: x0, y0 (bottom), x1, y1 (top).
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    /// Linear RGBA.
    pub color: [f32; 4],
    pub z: f32,
    /// Hover group (0 = not interactive), see `Renderer::set_highlight`.
    pub group: u32,
}

/// Horizontal alignment relative to the layout position's x.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy)]
pub struct TextStyle {
    pub font: Font,
    /// Em size in world units (or pixels on the screen-space layer).
    pub size: f32,
    pub color: [f32; 4],
    pub max_width: Option<f32>,
    pub align: Align,
    /// Multiple of the font's line height.
    pub line_spacing: f32,
    pub group: u32,
}

impl TextStyle {
    pub fn new(font: Font, size: f32, color: [f32; 4]) -> Self {
        Self {
            font,
            size,
            color,
            max_width: None,
            align: Align::Left,
            line_spacing: 1.0,
            group: 0,
        }
    }

    pub fn wrap(mut self, max_width: f32) -> Self {
        self.max_width = Some(max_width);
        self
    }

    pub fn centered(mut self) -> Self {
        self.align = Align::Center;
        self
    }

    pub fn right_aligned(mut self) -> Self {
        self.align = Align::Right;
        self
    }

    pub fn line_spacing(mut self, factor: f32) -> Self {
        self.line_spacing = factor;
        self
    }

    pub fn group(mut self, group: u32) -> Self {
        self.group = group;
        self
    }
}

/// A styled piece of a paragraph (e.g. from a `RichText` span).
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    pub text: &'a str,
    pub font: Font,
    pub color: [f32; 4],
    /// Hover group; runs with a non-zero group get their boxes reported.
    pub group: u32,
    /// A key (e.g. from `key_parts`): its box is reported as a keycap.
    pub key: bool,
}

/// Result of laying out a paragraph.
#[derive(Debug, Default)]
pub struct Paragraph {
    pub height: f32,
    /// Per line, the box `[x0, y0, x1, y1]` of each run with a non-zero group,
    /// e.g. for link underlines and hit regions.
    pub boxes: Vec<(u32, [f32; 4])>,
    /// The box of each key run, for its keycap outline.
    pub keys: Vec<[f32; 4]>,
}

/// Room on either side of a key's letter for its keycap outline: a space
/// that doesn't break lines.
const KEY_PAD: char = '\u{a0}';

/// Splits `text` with keys marked like `Press [S] for…` into plain parts
/// and keys (`true`), each key's letters padded for its keycap.
pub fn key_parts(text: &str) -> Vec<(String, bool)> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('[')
        && let Some((key, tail)) = after.split_once(']')
    {
        if !before.is_empty() {
            parts.push((before.to_owned(), false));
        }
        parts.push((format!("{KEY_PAD}{key}{KEY_PAD}"), true));
        rest = tail;
    }
    if !rest.is_empty() {
        parts.push((rest.to_owned(), false));
    }
    parts
}

/// Where a keycap outline goes for a key run's `box` (from
/// `Paragraph::keys`) in text of `size`: around the letter, inside the
/// padding, and about square even for a narrow letter like I.
pub fn keycap([x0, y0, x1, y1]: [f32; 4], size: f32) -> [f32; 4] {
    let half_width = ((x1 - x0) / 2.0 - 0.06 * size).max(0.4 * size);
    let center = (x0 + x1) / 2.0;
    [
        center - half_width,
        y0 + 0.05 * size,
        center + half_width,
        y1 - 0.12 * size,
    ]
}

fn advance(font: Font, ch: char) -> f32 {
    glyph(font, ch).map_or(0.0, |g| g.advance)
}

/// Width of `text` in one font, in the units of `size`.
pub fn width(font: Font, size: f32, text: &str) -> f32 {
    text.chars().map(|ch| advance(font, ch)).sum::<f32>() * size
}

#[derive(Clone, Copy)]
struct StyledChar {
    ch: char,
    font: Font,
    run: usize,
}

fn chars_width(chars: &[StyledChar], size: f32) -> f32 {
    chars.iter().map(|c| advance(c.font, c.ch)).sum::<f32>() * size
}

/// Greedy word wrap; returns line ranges. Lines break at spaces (the space
/// is dropped) or after a slash (kept, so "3D/Node.js/React" can wrap on a
/// narrow screen).
fn wrap(chars: &[StyledChar], size: f32, max_width: Option<f32>) -> Vec<Range<usize>> {
    let max_width = max_width.unwrap_or(f32::INFINITY);
    let mut lines = Vec::new();
    let mut start = 0;
    // Where the line could end, and where the next one would start.
    let mut last_break: Option<(usize, usize)> = None;
    for (i, c) in chars.iter().enumerate() {
        let opportunity = match c.ch {
            ' ' => (i, i + 1),
            '/' => (i + 1, i + 1),
            _ => continue,
        };
        if chars_width(&chars[start..opportunity.0], size) > max_width
            && let Some((end, next)) = last_break
        {
            lines.push(start..end);
            start = next;
        }
        last_break = Some(opportunity);
    }
    if chars_width(&chars[start..], size) > max_width
        && let Some((end, next)) = last_break.filter(|&(end, _)| end > start)
    {
        lines.push(start..end);
        start = next;
    }
    lines.push(start..chars.len());
    lines
}

/// The size (width of the widest line, height) `text` takes up in `style`.
pub fn measure(text: &str, style: TextStyle) -> [f32; 2] {
    let chars: Vec<StyledChar> = text
        .chars()
        .map(|ch| StyledChar {
            ch,
            font: style.font,
            run: 0,
        })
        .collect();
    let lines = wrap(&chars, style.size, style.max_width);
    let width = lines
        .iter()
        .map(|line| chars_width(&chars[line.clone()], style.size))
        .fold(0.0, f32::max);
    [width, block_height(lines.len(), style)]
}

fn block_height(lines: usize, style: TextStyle) -> f32 {
    let m = metrics(style.font);
    let line_advance = m.line_height * style.line_spacing * style.size;
    (lines as f32 - 1.0) * line_advance + (m.ascender - m.descender) * style.size
}

/// Lays out `text` with its first line's top at `top_left` (for centered
/// text, `top_left.x` is the center; for right-aligned text, the right
/// edge). Returns the block height.
pub fn layout(text: &str, style: TextStyle, top_left: Vec3, out: &mut Vec<GlyphInstance>) -> f32 {
    let run = Run {
        text,
        font: style.font,
        color: style.color,
        group: style.group,
        key: false,
    };
    layout_runs(&[run], style, top_left, out).height
}

/// Lays out styled runs as one paragraph. `style` provides size, wrapping,
/// alignment and line spacing (line metrics come from `style.font`).
pub fn layout_runs(
    runs: &[Run],
    style: TextStyle,
    top_left: Vec3,
    out: &mut Vec<GlyphInstance>,
) -> Paragraph {
    let chars: Vec<StyledChar> = runs
        .iter()
        .enumerate()
        .flat_map(|(run, r)| {
            r.text.chars().map(move |ch| StyledChar {
                ch,
                font: r.font,
                run,
            })
        })
        .collect();
    let m = metrics(style.font);
    let size = style.size;
    let line_advance = m.line_height * style.line_spacing * size;
    let mut baseline = top_left.y - m.ascender * size;
    let lines = wrap(&chars, size, style.max_width);
    let mut boxes = Vec::new();
    let mut keys = Vec::new();
    for line in &lines {
        let line_chars = &chars[line.clone()];
        let mut pen = match style.align {
            Align::Left => top_left.x,
            Align::Center => top_left.x - chars_width(line_chars, size) / 2.0,
            Align::Right => top_left.x - chars_width(line_chars, size),
        };
        // Box of the run currently being extended: (run, x0).
        let mut open_box: Option<(usize, f32)> = None;
        let close_box = |run: usize,
                         x0: f32,
                         x1: f32,
                         boxes: &mut Vec<(u32, [f32; 4])>,
                         keys: &mut Vec<[f32; 4]>| {
            let bottom = baseline + m.descender * size;
            let top = baseline + m.ascender * size;
            let rect = [x0, bottom, x1, top];
            if runs[run].key {
                keys.push(rect);
            } else {
                boxes.push((runs[run].group, rect));
            }
        };
        for c in line_chars {
            let run = &runs[c.run];
            if let Some((r, x0)) = open_box
                && r != c.run
            {
                close_box(r, x0, pen, &mut boxes, &mut keys);
                open_box = None;
            }
            if (run.group != 0 || run.key) && open_box.is_none() {
                open_box = Some((c.run, pen));
            }
            let Some(g) = glyph(c.font, c.ch) else {
                continue;
            };
            if g.visible {
                let [l, b, r, t] = g.plane;
                out.push(GlyphInstance {
                    rect: [
                        pen + l * size,
                        baseline + b * size,
                        pen + r * size,
                        baseline + t * size,
                    ],
                    uv: g.uv,
                    color: run.color,
                    z: top_left.z,
                    group: run.group,
                });
            }
            pen += g.advance * size;
        }
        if let Some((r, x0)) = open_box {
            close_box(r, x0, pen, &mut boxes, &mut keys);
        }
        baseline -= line_advance;
    }
    Paragraph {
        height: block_height(lines.len(), style),
        boxes,
        keys,
    }
}

/// sRGB hex color to linear RGBA (the surface view is sRGB).
pub const fn rgb(hex: u32) -> [f32; 4] {
    [
        srgb_to_linear((hex >> 16) as u8),
        srgb_to_linear((hex >> 8) as u8),
        srgb_to_linear(hex as u8),
        1.0,
    ]
}

/// sRGB bytes (e.g. a resume entry's `accent`) to linear RGBA.
pub const fn rgb_bytes([r, g, b]: [u8; 3]) -> [f32; 4] {
    [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), 1.0]
}

const fn srgb_to_linear(c: u8) -> f32 {
    // Cheap gamma 2.2 approximation, good enough for text colors (const fn).
    let c = c as f32 / 255.0;
    c * c * (0.8 + 0.2 * c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_baked_atlas_decodes_to_its_size() {
        let [width, height] = ATLAS_SIZE;
        let rgba = atlas_rgba();
        assert_eq!(rgba.len(), width as usize * height as usize * 4);
        // Glyphs are in there: some texels are inside a letter (above 127).
        assert!(
            rgba.chunks(4)
                .any(|t| t[0] > 200 && t[1] > 200 && t[2] > 200)
        );
    }

    fn styled(text: &str) -> Vec<StyledChar> {
        text.chars()
            .map(|ch| StyledChar {
                ch,
                font: Font::Regular,
                run: 0,
            })
            .collect()
    }

    #[test]
    fn glyph_table_is_sorted_and_complete() {
        assert!(
            baked::GLYPHS
                .windows(2)
                .all(|w| (w[0].font, w[0].ch) < (w[1].font, w[1].ch))
        );
        // Every character the real, baked resume content needs must have a
        // glyph -- derived from the actual content (the same way build.rs
        // derives the atlas) rather than a hand-picked sample, so it can't
        // drift out of sync with content/resume.yaml.
        let resume = crate::content::resume();
        for font in [Font::Regular, Font::Bold] {
            for ch in resume.all_text().chars().filter(|c| !c.is_control()) {
                assert!(glyph(font, ch).is_some(), "{font:?} {ch:?}");
            }
        }
        // Regression check for the non-breaking space in the Dedalus
        // highlight ("DirectX&nbsp;9"): it must be baked even though U+00A0
        // never appears literally in content/resume.yaml's raw bytes.
        assert!(
            glyph(Font::Regular, '\u{a0}').is_some(),
            "non-breaking space (U+00A0) must have a baked glyph"
        );
    }

    #[test]
    fn measures_what_layout_draws() {
        let style = TextStyle::new(Font::Regular, 0.1, [1.0; 4]);
        // One line: the width is the advance width.
        assert_eq!(
            measure("one line", style)[0],
            width(Font::Regular, 0.1, "one line")
        );
        // Wrapped: as wide as the widest line, as tall as the layout.
        let wrapped = style.wrap(0.5);
        let text = "a few words that need wrapping";
        let [widest, height] = measure(text, wrapped);
        assert!(widest <= 0.5 && widest >= width(Font::Regular, 0.1, "wrapping"));
        let mut glyphs = Vec::new();
        assert_eq!(layout(text, wrapped, Vec3::ZERO, &mut glyphs), height);
        // Right-aligned lines end at the layout position (glyph quads reach
        // past the advance by the atlas padding, under 0.1 em).
        let mut glyphs = Vec::new();
        layout("end", style.right_aligned(), Vec3::ZERO, &mut glyphs);
        assert!(glyphs.iter().all(|g| g.rect[2] <= 0.01));
        assert!(glyphs.iter().any(|g| g.rect[2] > -0.01));
    }

    #[test]
    fn wraps_after_slashes_too() {
        let chars = styled("Engineer 3D/Node.js/React/TypeScript");
        let narrow = width(Font::Regular, 1.0, "Engineer 3D/Node.js/");
        let lines: Vec<String> = wrap(&chars, 1.0, Some(narrow))
            .into_iter()
            .map(|range| chars[range].iter().map(|c| c.ch).collect())
            .collect();
        assert_eq!(lines, ["Engineer 3D/Node.js/", "React/TypeScript"]);
    }

    #[test]
    fn wraps_at_spaces() {
        let one_word = width(Font::Regular, 1.0, "word");
        let chars = styled("word word word word");
        let lines: Vec<String> = wrap(&chars, 1.0, Some(one_word * 2.2))
            .into_iter()
            .map(|range| chars[range].iter().map(|c| c.ch).collect())
            .collect();
        assert_eq!(lines, ["word word", "word word"]);
    }

    #[test]
    fn splits_marked_keys_and_reports_their_boxes() {
        let parts = key_parts("Press [S] for the map, [I] too");
        let texts: Vec<(&str, bool)> = parts.iter().map(|(t, k)| (t.as_str(), *k)).collect();
        assert_eq!(
            texts,
            [
                ("Press ", false),
                ("\u{a0}S\u{a0}", true),
                (" for the map, ", false),
                ("\u{a0}I\u{a0}", true),
                (" too", false),
            ]
        );
        assert_eq!(key_parts("no keys"), [("no keys".to_owned(), false)]);
        let white = [1.0; 4];
        let runs: Vec<Run> = parts
            .iter()
            .map(|(text, key)| Run {
                text,
                font: Font::Regular,
                color: white,
                group: 0,
                key: *key,
            })
            .collect();
        let mut out = Vec::new();
        let style = TextStyle::new(Font::Regular, 1.0, white);
        let paragraph = layout_runs(&runs, style, Vec3::ZERO, &mut out);
        assert_eq!(paragraph.keys.len(), 2);
        assert!(paragraph.boxes.is_empty());
        // The keycap sits inside the key's padding, clear of its neighbors.
        let [x0, _, x1, _] = keycap(paragraph.keys[0], 1.0);
        let letter = out
            .iter()
            .find(|g| g.rect[0] > paragraph.keys[0][0] && g.rect[2] < paragraph.keys[0][2])
            .expect("the key's letter");
        assert!(x0 < letter.rect[0] && letter.rect[2] < x1);
    }

    #[test]
    fn reports_boxes_of_grouped_runs_per_line() {
        let white = [1.0; 4];
        let runs = [
            Run {
                text: "see ",
                font: Font::Regular,
                color: white,
                group: 0,
                key: false,
            },
            Run {
                text: "my site",
                font: Font::Bold,
                color: white,
                group: 7,
                key: false,
            },
            Run {
                text: " now",
                font: Font::Regular,
                color: white,
                group: 0,
                key: false,
            },
        ];
        let mut out = Vec::new();
        let paragraph = layout_runs(
            &runs,
            TextStyle::new(Font::Regular, 1.0, white),
            Vec3::ZERO,
            &mut out,
        );
        assert_eq!(paragraph.boxes.len(), 1);
        let (group, [x0, y0, x1, y1]) = paragraph.boxes[0];
        assert_eq!(group, 7);
        assert!((x0 - width(Font::Regular, 1.0, "see ")).abs() < 1e-5);
        assert!((x1 - x0 - width(Font::Bold, 1.0, "my site")).abs() < 1e-5);
        assert!(y1 > y0);
        assert!(out.iter().any(|g| g.group == 7) && out.iter().any(|g| g.group == 0));
    }
}
