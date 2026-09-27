//! Instanced rounded rectangles (signed distance field in `shaders/shapes.wgsl`):
//! skill chips, link underlines, buttons and path dots. Like text, shapes lie
//! in a plane facing +z (or in screen pixels on the screen-space layer).

use bytemuck::{Pod, Zeroable};

/// One shape; matches the vertex layout in `shaders/shapes.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ShapeInstance {
    /// x0, y0 (bottom), x1, y1 (top).
    pub rect: [f32; 4],
    /// Linear RGBA.
    pub color: [f32; 4],
    pub z: f32,
    /// Corner radius (clamped to half the shorter side).
    pub radius: f32,
    /// Outline width; 0 fills the shape.
    pub border: f32,
    /// Hover group (0 = not interactive), see `Renderer::set_highlight`.
    pub group: u32,
}

impl ShapeInstance {
    pub fn filled(rect: [f32; 4], z: f32, radius: f32, color: [f32; 4]) -> Self {
        Self {
            rect,
            color,
            z,
            radius,
            border: 0.0,
            group: 0,
        }
    }

    pub fn outlined(rect: [f32; 4], z: f32, radius: f32, border: f32, color: [f32; 4]) -> Self {
        Self {
            border,
            ..Self::filled(rect, z, radius, color)
        }
    }

    /// A filled circle.
    pub fn dot(center: [f32; 2], z: f32, radius: f32, color: [f32; 4]) -> Self {
        let [x, y] = center;
        Self::filled(
            [x - radius, y - radius, x + radius, y + radius],
            z,
            radius,
            color,
        )
    }

    pub fn group(mut self, group: u32) -> Self {
        self.group = group;
        self
    }
}
