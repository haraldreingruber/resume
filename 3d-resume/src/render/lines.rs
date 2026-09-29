//! Instanced line segments (`shaders/lines.wgsl`): the edges of the skill
//! map. Each segment belongs to two hover groups (its end nodes) and lights
//! up when either is the active node. Like text and shapes, lines lie in a
//! plane facing +z.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// One segment; matches the vertex layout in `shaders/lines.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LineInstance {
    /// xyz: start, w: width.
    pub start: [f32; 4],
    /// xyz: end (w unused).
    pub end: [f32; 4],
    /// Linear RGBA.
    pub color: [f32; 4],
    /// The hover groups of the two nodes the line connects.
    pub groups: [u32; 2],
}

/// A cubic curve from `from` to `to` that leaves and arrives horizontally
/// (like a flow diagram), as `segments` straight pieces.
pub fn curve(
    from: Vec3,
    to: Vec3,
    width: f32,
    color: [f32; 4],
    groups: [u32; 2],
    segments: usize,
) -> impl Iterator<Item = LineInstance> {
    let bend = (to.x - from.x) * 0.5;
    let (c1, c2) = (from + Vec3::X * bend, to - Vec3::X * bend);
    let point = move |t: f32| {
        let u = 1.0 - t;
        from * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + to * (t * t * t)
    };
    (0..segments).map(move |i| {
        let (a, b) = (
            point(i as f32 / segments as f32),
            point((i + 1) as f32 / segments as f32),
        );
        LineInstance {
            start: a.extend(width).to_array(),
            end: b.extend(0.0).to_array(),
            color,
            groups,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_connect_their_end_points() {
        let (from, to) = (Vec3::new(-1.0, 0.5, -3.0), Vec3::new(1.0, -0.5, -3.0));
        let lines: Vec<_> = curve(from, to, 0.01, [1.0; 4], [3, 7], 8).collect();
        assert_eq!(lines.len(), 8);
        assert!(Vec3::from_slice(&lines[0].start[..3]).distance(from) < 1e-6);
        assert!(Vec3::from_slice(&lines[7].end[..3]).distance(to) < 1e-6);
        // Consecutive pieces join up.
        for pair in lines.windows(2) {
            assert_eq!(pair[0].end[..3], pair[1].start[..3]);
        }
    }
}
