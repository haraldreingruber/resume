//! Screen-space layer in physical pixels (origin bottom-left, y up), drawn on
//! top of the scene with an orthographic projection and no fading. The native
//! app uses it for the "Text version · PDF" buttons that the web page provides
//! as HTML.

use glam::{Mat4, Vec3};

use crate::shapes::ShapeInstance;
use crate::text::{self, Font, GlyphInstance, TextStyle, rgb};

const BUTTON_TEXT: [f32; 4] = rgb(0xC9D4DE);
const BUTTON_BORDER: [f32; 4] = rgb(0x3E5A73);
const BUTTON_FILL: [f32; 4] = [0.0005, 0.002, 0.004, 0.6];

#[derive(Default)]
pub struct UiLayer {
    pub glyphs: Vec<GlyphInstance>,
    pub shapes: Vec<ShapeInstance>,
    hits: Vec<([f32; 4], u32)>,
}

impl UiLayer {
    /// Pill buttons in the bottom-right corner, right to left in the given
    /// order, sized in logical pixels times `scale`.
    pub fn buttons(width: f32, scale: f32, buttons: &[(&str, u32)]) -> Self {
        let mut layer = Self::default();
        let (size, pad_x, pad_y, gap, margin) = (
            14.0 * scale,
            14.0 * scale,
            9.0 * scale,
            8.0 * scale,
            16.0 * scale,
        );
        let height = size * 1.2 + 2.0 * pad_y;
        let mut right = width - margin;
        for &(label, group) in buttons.iter().rev() {
            let style = TextStyle::new(Font::Bold, size, BUTTON_TEXT).group(group);
            let w = text::width(Font::Bold, size, label) + 2.0 * pad_x;
            let rect = [right - w, margin, right, margin + height];
            let radius = height / 2.0;
            layer
                .shapes
                .push(ShapeInstance::filled(rect, 0.0, radius, BUTTON_FILL).group(group));
            layer.shapes.push(
                ShapeInstance::outlined(rect, 0.0, radius, scale, BUTTON_BORDER).group(group),
            );
            let top = Vec3::new(right - w + pad_x, margin + height - pad_y * 0.8, 0.0);
            text::layout(label, style, top, &mut layer.glyphs);
            layer.hits.push((rect, group));
            right -= w + gap;
        }
        layer
    }

    /// The hover group of the button at `(x, y)` (physical pixels, y up).
    pub fn pick(&self, x: f32, y: f32) -> Option<u32> {
        self.hits
            .iter()
            .find(|([x0, y0, x1, y1], _)| (*x0..=*x1).contains(&x) && (*y0..=*y1).contains(&y))
            .map(|&(_, group)| group)
    }

    /// Maps physical pixels (origin bottom-left) to clip space.
    pub fn projection(width: f32, height: f32) -> Mat4 {
        glam::camera::rh::proj::directx::orthographic(0.0, width, 0.0, height, -1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_buttons_in_the_bottom_right_corner() {
        let layer = UiLayer::buttons(800.0, 1.0, &[("Text version", 5), ("PDF", 6)]);
        assert_eq!(layer.hits.len(), 2);
        // "PDF" is rightmost.
        let ([x0, y0, x1, y1], group) = layer.hits[0];
        assert_eq!(group, 6);
        assert!(x1 <= 800.0 && x0 < x1 && y0 < y1);
        assert_eq!(layer.pick((x0 + x1) / 2.0, (y0 + y1) / 2.0), Some(6));
        assert_eq!(layer.pick(10.0, 590.0), None);
    }
}
