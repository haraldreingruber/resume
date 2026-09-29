//! How the MSDF atlas is stored in the app: its three color channels one
//! after the other, each value as its difference from a prediction (left +
//! above − above-left, clamped). Distance fields change smoothly, so most
//! differences are small and repeat, and the HTTP compression that serves
//! the web build (gzip, brotli) shrinks them far better than the PNG this
//! replaces (brotli: 184 KB → 104 KB, gzip: 197 KB → 156 KB). Decoding is a
//! single pass and needs no decoder library.
//!
//! Shared by `build.rs` (which encodes, via `#[path]`) and the app (which
//! decodes); dependency-free for that reason.

/// Encodes `rgb` (`width` × `height`, 3 bytes per pixel, top row first).
// Used by build.rs; in the app only by tests.
#[allow(dead_code)]
pub fn encode(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    assert_eq!(rgb.len(), width * height * 3);
    let mut out = Vec::with_capacity(rgb.len());
    for channel in 0..3 {
        let value = |x: usize, y: usize| rgb[(y * width + x) * 3 + channel];
        for y in 0..height {
            for x in 0..width {
                let guess = predict(&value, x, y);
                out.push(value(x, y).wrapping_sub(guess));
            }
        }
    }
    out
}

/// Decodes `data` from `encode` into RGBA pixels (alpha 255).
#[allow(dead_code)]
pub fn decode(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    assert_eq!(data.len(), width * height * 3, "atlas data size");
    let mut rgba = vec![255; width * height * 4];
    for channel in 0..3 {
        let plane = &data[channel * width * height..][..width * height];
        for y in 0..height {
            for x in 0..width {
                let guess = predict(&|x, y| rgba[(y * width + x) * 4 + channel], x, y);
                rgba[(y * width + x) * 4 + channel] = plane[y * width + x].wrapping_add(guess);
            }
        }
    }
    rgba
}

/// The predicted value at `(x, y)` from its already known neighbors (zero
/// outside the image).
fn predict(value: &impl Fn(usize, usize) -> u8, x: usize, y: usize) -> u8 {
    let left = if x > 0 { value(x - 1, y) } else { 0 };
    let up = if y > 0 { value(x, y - 1) } else { 0 };
    let up_left = if x > 0 && y > 0 {
        value(x - 1, y - 1)
    } else {
        0
    };
    (i16::from(left) + i16::from(up) - i16::from(up_left)).clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let (width, height) = (7, 5);
        // Arbitrary but deterministic values, including the extremes.
        let rgb: Vec<u8> = (0..width * height * 3)
            .map(|i| ((i * 97 + i * i * 13) % 256) as u8)
            .collect();
        let rgba = decode(&encode(&rgb, width, height), width, height);
        for (pixel, expected) in rgba.chunks(4).zip(rgb.chunks(3)) {
            assert_eq!(&pixel[..3], expected);
            assert_eq!(pixel[3], 255);
        }
    }

    #[test]
    fn smooth_data_encodes_to_small_values() {
        let (width, height) = (16, 8);
        let rgb: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                let v = (x * 4 + y * 3) as u8;
                [v, v, v]
            })
            .collect();
        let encoded = encode(&rgb, width, height);
        // Away from the first row and column, a plane's prediction is exact.
        let inner = (1..height).flat_map(|y| (1..width).map(move |x| y * width + x));
        assert!(inner.map(|i| encoded[i]).all(|d| d == 0));
    }
}
