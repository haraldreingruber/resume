//! The app icon: an accent-blue dot with a soft halo (the skill map's node
//! style) on a rounded square in the background's night blue. The browser
//! favicon (`web/favicon.svg`) is the same design.

/// Designed on a 64-unit grid.
const GRID: f32 = 64.0;
const CORNER: f32 = 14.0;
const HALO: f32 = 18.0;
const DOT: f32 = 9.0;
const BACKGROUND: [f32; 3] = [0x0B as f32, 0x1A as f32, 0x2B as f32];
const ACCENT: [f32; 3] = [0x6D as f32, 0xB3 as f32, 0xE8 as f32];

/// Straight-alpha RGBA pixels of a `size`×`size` icon, top row first.
pub fn pixels(size: u32) -> Vec<u8> {
    let scale = size as f32 / GRID;
    // One pixel in grid units, for anti-aliasing.
    let pixel = 1.0 / scale;
    let coverage = |distance: f32| (0.5 - distance / pixel).clamp(0.0, 1.0);
    (0..size * size)
        .flat_map(|i| {
            // Grid coordinates of the pixel center, relative to the middle.
            let x = ((i % size) as f32 + 0.5) / scale - GRID / 2.0;
            let y = ((i / size) as f32 + 0.5) / scale - GRID / 2.0;
            let r = x.hypot(y);
            let square = rounded_square(x, y, GRID / 2.0, CORNER);
            // Layers, back to front: (color, opacity), composited "over".
            let layers = [
                (BACKGROUND, coverage(square)),
                (ACCENT, 0.3 * coverage(r - HALO)),
                (ACCENT, coverage(r - DOT)),
            ];
            let (mut color, mut alpha) = ([0.0f32; 3], 0.0f32);
            for (layer, a) in layers {
                for (c, l) in color.iter_mut().zip(layer) {
                    *c = l * a + *c * (1.0 - a);
                }
                alpha = a + alpha * (1.0 - a);
            }
            let straight = color.map(|c| if alpha > 0.0 { c / alpha } else { 0.0 });
            [
                straight[0].round() as u8,
                straight[1].round() as u8,
                straight[2].round() as u8,
                (alpha * 255.0).round() as u8,
            ]
        })
        .collect()
}

/// Signed distance to a square of half-size `half` with rounded corners.
fn rounded_square(x: f32, y: f32, half: f32, radius: f32) -> f32 {
    let (qx, qy) = (x.abs() - half + radius, y.abs() - half + radius);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

/// The window icon (title bar, taskbar).
pub fn window_icon() -> Option<winit::window::Icon> {
    const SIZE: u32 = 64;
    winit::window::Icon::from_rgba(pixels(SIZE), SIZE, SIZE).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_a_dot_on_a_rounded_square() {
        let size = 64;
        let pixels = pixels(size);
        let at = |x: u32, y: u32| {
            let i = ((y * size + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };
        assert_eq!(at(32, 32), [0x6D, 0xB3, 0xE8, 255]);
        assert_eq!(at(32, 2), [0x0B, 0x1A, 0x2B, 255]);
        // The rounded corner is transparent.
        assert_eq!(at(0, 0)[3], 0);
    }
}
