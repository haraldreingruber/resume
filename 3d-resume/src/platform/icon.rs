//! The app icon: an accent-blue dot with a soft halo (the skill map's node
//! style) on a rounded square in the background's night blue. The browser
//! favicon (`web/favicon.svg`) is the same design, and `--icons <dir>` writes
//! the web app's PNG icons from it (`web/icon-*.png`).

/// Designed on a 64-unit grid.
const GRID: f32 = 64.0;
const CORNER: f32 = 14.0;
const HALO: f32 = 18.0;
const DOT: f32 = 9.0;
const BACKGROUND: [f32; 3] = [0x0B as f32, 0x1A as f32, 0x2B as f32];
const ACCENT: [f32; 3] = [0x6D as f32, 0xB3 as f32, 0xE8 as f32];

/// Straight-alpha RGBA pixels of a `size`×`size` icon, top row first.
pub fn pixels(size: u32) -> Vec<u8> {
    pixels_with(size, CORNER)
}

/// The icon filling its whole square (no rounded, transparent corners), for
/// platforms that cut their own shape: installed web apps' "maskable" icons
/// and iOS home screens. The dot stays inside the central safe zone.
pub fn full_bleed_pixels(size: u32) -> Vec<u8> {
    pixels_with(size, 0.0)
}

fn pixels_with(size: u32, corner: f32) -> Vec<u8> {
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
            let square = rounded_square(x, y, GRID / 2.0, corner);
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

/// Writes the web app's icons (see `web/manifest.webmanifest`) into `dir`.
pub fn write_web_icons(dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    type Draw = fn(u32) -> Vec<u8>;
    let icons: [(&str, u32, Draw); 4] = [
        ("icon-192.png", 192, pixels),
        ("icon-512.png", 512, pixels),
        ("icon-maskable-512.png", 512, full_bleed_pixels),
        ("apple-touch-icon.png", 180, full_bleed_pixels),
    ];
    for (name, size, draw) in icons {
        let path = dir.join(name);
        let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size, size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&draw(size)))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        log::info!("wrote {}", path.display());
    }
    Ok(())
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

    #[test]
    fn the_full_bleed_icon_has_no_transparent_pixel() {
        let pixels = full_bleed_pixels(48);
        assert!(pixels.chunks(4).all(|p| p[3] == 255));
        // The same dot in the middle.
        let middle = ((24 * 48 + 24) * 4) as usize;
        assert_eq!(&pixels[middle..middle + 3], &[0x6D, 0xB3, 0xE8]);
    }
}
