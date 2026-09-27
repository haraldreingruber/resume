//! MSDF text: glyph metrics baked by `build.rs`, word-wrapped layout, and
//! the per-glyph GPU instances drawn by `shaders/text.wgsl`.
//!
//! Text lies in a plane facing +z; units are world units.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

// Generated metrics can happen to resemble constants like 1/π.
#[allow(clippy::approx_constant)]
mod baked {
    use super::{FontMetrics, GlyphInfo};
    include!(concat!(env!("OUT_DIR"), "/glyphs.rs"));
}

pub use baked::{ATLAS_SIZE, DISTANCE_RANGE_PX};

pub const ATLAS_PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/atlas.png"));

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
    /// World-space quad: x0, y0 (bottom), x1, y1 (top).
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    /// Linear RGBA.
    pub color: [f32; 4],
    pub z: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
}

#[derive(Debug, Clone, Copy)]
pub struct TextStyle {
    pub font: Font,
    /// Em size in world units.
    pub size: f32,
    pub color: [f32; 4],
    pub max_width: Option<f32>,
    pub align: Align,
    /// Multiple of the font's line height.
    pub line_spacing: f32,
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

    pub fn line_spacing(mut self, factor: f32) -> Self {
        self.line_spacing = factor;
        self
    }
}

fn advance(font: Font, ch: char) -> f32 {
    glyph(font, ch).map_or(0.0, |g| g.advance)
}

fn width(style: &TextStyle, text: &str) -> f32 {
    text.chars().map(|ch| advance(style.font, ch)).sum::<f32>() * style.size
}

/// Greedy word wrap at spaces.
fn lines<'a>(style: &TextStyle, text: &'a str) -> Vec<&'a str> {
    let Some(max_width) = style.max_width else {
        return vec![text];
    };
    let mut lines = Vec::new();
    let mut start = 0;
    let mut last_break = None;
    for (i, ch) in text.char_indices() {
        if ch == ' ' {
            if width(style, &text[start..i]) > max_width
                && let Some(b) = last_break
            {
                lines.push(&text[start..b]);
                start = b + 1;
            }
            last_break = Some(i);
        }
    }
    if width(style, &text[start..]) > max_width
        && let Some(b) = last_break.filter(|&b| b > start)
    {
        lines.push(&text[start..b]);
        start = b + 1;
    }
    lines.push(&text[start..]);
    lines
}

/// Lays out `text` with its first line's top at `top_left` (for centered
/// text, `top_left.x` is the center). Returns the block height.
pub fn layout(text: &str, style: TextStyle, top_left: Vec3, out: &mut Vec<GlyphInstance>) -> f32 {
    let m = metrics(style.font);
    let line_advance = m.line_height * style.line_spacing * style.size;
    let mut baseline = top_left.y - m.ascender * style.size;
    let lines = lines(&style, text);
    for line in &lines {
        let mut pen = match style.align {
            Align::Left => top_left.x,
            Align::Center => top_left.x - width(&style, line) / 2.0,
        };
        for ch in line.chars() {
            let Some(g) = glyph(style.font, ch) else {
                continue;
            };
            if g.visible {
                let [l, b, r, t] = g.plane;
                out.push(GlyphInstance {
                    rect: [
                        pen + l * style.size,
                        baseline + b * style.size,
                        pen + r * style.size,
                        baseline + t * style.size,
                    ],
                    uv: g.uv,
                    color: style.color,
                    z: top_left.z,
                });
            }
            pen += g.advance * style.size;
        }
        baseline -= line_advance;
    }
    let descent = -m.descender * style.size;
    (lines.len() as f32 - 1.0) * line_advance + m.ascender * style.size + descent
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

const fn srgb_to_linear(c: u8) -> f32 {
    // Cheap gamma 2.2 approximation, good enough for text colors (const fn).
    let c = c as f32 / 255.0;
    c * c * (0.8 + 0.2 * c)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn wraps_at_spaces() {
        let style = TextStyle::new(Font::Regular, 1.0, [1.0; 4]);
        let one_word = width(&style, "word");
        let wrapped = lines(&style.wrap(one_word * 2.2), "word word word word");
        assert_eq!(wrapped, ["word word", "word word"]);
    }
}
