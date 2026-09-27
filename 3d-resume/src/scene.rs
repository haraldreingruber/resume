//! The timeline scene: an intro station followed by one station per job,
//! laid out along a gently winding path into the screen (-z). The camera
//! flies along a Catmull-Rom spline through the stations.

use glam::{Mat4, Vec3};
use resume_model::{Resume, Work};

use crate::text::{self, Font, GlyphInstance, TextStyle, rgb};

/// Distance between stations along -z.
const SPACING: f32 = 6.0;
/// Sideways offset of alternating stations.
const SWAY: f32 = 0.9;
/// Width of wrapped text blocks.
const BLOCK_WIDTH: f32 = 3.2;
/// Half-width the camera keeps in view, so narrow screens back off.
const VIEW_HALF_WIDTH: f32 = 2.3;
/// Width of the centered intro tagline.
const TAGLINE_WIDTH: f32 = 4.4;
const FOV_Y: f32 = 50.0_f32.to_radians();

const NAME: [f32; 4] = rgb(0xF0F4F8);
const BODY: [f32; 4] = rgb(0xC9D4DE);
const ACCENT: [f32; 4] = rgb(0x6DB3E8);
const MUTED: [f32; 4] = rgb(0x8AA0B4);
const PATH: [f32; 4] = rgb(0x3E5A73);

pub struct Scene {
    /// Station anchors: horizontal center of each text block, at its top.
    stations: Vec<Vec3>,
    /// Glyphs ordered far to near (drawn without depth buffer).
    pub glyphs: Vec<GlyphInstance>,
}

impl Scene {
    pub fn new(resume: &Resume) -> Self {
        let count = 1 + resume.work.len();
        let stations: Vec<Vec3> = (0..count)
            .map(|i| {
                let side = if i == 0 {
                    0.0
                } else if i % 2 == 1 {
                    SWAY
                } else {
                    -SWAY
                };
                Vec3::new(side, 0.6, -(i as f32) * SPACING)
            })
            .collect();

        // Build near to far, then reverse so far glyphs are drawn first.
        let mut blocks: Vec<Vec<GlyphInstance>> = Vec::new();
        blocks.push(intro(resume, stations[0]));
        for (work, &anchor) in resume.work.iter().zip(&stations[1..]) {
            blocks.push(job(work, anchor));
        }
        let mut glyphs = Vec::new();
        for (i, block) in blocks.into_iter().enumerate().rev() {
            glyphs.extend(path_dots(&stations, i));
            glyphs.extend(block);
        }
        Self { stations, glyphs }
    }

    pub fn station_count(&self) -> usize {
        self.stations.len()
    }

    /// Camera for timeline position `t`.
    pub fn camera(&self, t: f32, aspect: f32) -> Camera {
        // Back off on narrow screens so text blocks fit horizontally.
        let half_fov_x = ((FOV_Y / 2.0).tan() * aspect).atan();
        let distance = (VIEW_HALF_WIDTH / half_fov_x.tan()).max(3.6);

        let target = catmull_rom(&self.stations, t) + Vec3::new(0.0, -0.9, 0.0);
        // The camera sways less than the stations for a calmer ride.
        let eye = Vec3::new(target.x * 0.6, target.y + 0.35, target.z + distance);
        let view = glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Y);
        // wgpu uses DirectX-style clip space (depth 0..1).
        let projection = glam::camera::rh::proj::directx::perspective(FOV_Y, aspect, 0.1, 100.0);
        Camera {
            eye,
            view_proj: projection * view,
            focus_distance: distance,
        }
    }
}

pub struct Camera {
    pub eye: Vec3,
    pub view_proj: Mat4,
    /// Distance from the eye to the station in focus.
    pub focus_distance: f32,
}

fn intro(resume: &Resume, anchor: Vec3) -> Vec<GlyphInstance> {
    let basics = &resume.basics;
    let mut out = Vec::new();
    let mut cursor = anchor + Vec3::new(0.0, 0.5, 0.0);
    stack(
        &basics.name,
        TextStyle::new(Font::Bold, 0.42, NAME).centered(),
        0.12,
        &mut cursor,
        &mut out,
    );
    let label = TextStyle::new(Font::Bold, 0.14, ACCENT)
        .centered()
        .wrap(TAGLINE_WIDTH);
    stack(&basics.label, label, 0.25, &mut cursor, &mut out);
    let summary = TextStyle::new(Font::Regular, 0.085, BODY)
        .centered()
        .wrap(BLOCK_WIDTH)
        .line_spacing(1.15);
    stack(
        &basics.summary.plain(),
        summary,
        0.35,
        &mut cursor,
        &mut out,
    );
    let hint = TextStyle::new(Font::Regular, 0.07, MUTED).centered();
    stack(
        "Scroll, swipe or use the arrow keys to travel through time",
        hint,
        0.0,
        &mut cursor,
        &mut out,
    );
    out
}

fn job(work: &Work, anchor: Vec3) -> Vec<GlyphInstance> {
    let mut out = Vec::new();
    let mut cursor = anchor - Vec3::new(BLOCK_WIDTH / 2.0, 0.0, 0.0);
    let mut meta = work.dates.to_string();
    if let Some(location) = &work.location {
        meta = format!("{meta}  ·  {location}");
    }
    stack(
        &meta,
        TextStyle::new(Font::Regular, 0.075, MUTED),
        0.08,
        &mut cursor,
        &mut out,
    );
    let position = TextStyle::new(Font::Bold, 0.16, NAME).wrap(BLOCK_WIDTH);
    stack(&work.position, position, 0.06, &mut cursor, &mut out);
    stack(
        &work.organization,
        TextStyle::new(Font::Bold, 0.11, ACCENT),
        0.18,
        &mut cursor,
        &mut out,
    );
    if let Some(summary) = &work.summary {
        let style = TextStyle::new(Font::Regular, 0.08, BODY)
            .wrap(BLOCK_WIDTH)
            .line_spacing(1.15);
        stack(&summary.plain(), style, 0.0, &mut cursor, &mut out);
    }
    out
}

/// Lays out a text block at `cursor` and moves the cursor below it.
fn stack(text: &str, style: TextStyle, gap: f32, cursor: &mut Vec3, out: &mut Vec<GlyphInstance>) {
    cursor.y -= text::layout(text, style, *cursor, out) + gap;
}

/// Dots along the path from station `i` towards the previous one, on the floor.
fn path_dots(stations: &[Vec3], i: usize) -> Vec<GlyphInstance> {
    let mut out = Vec::new();
    if i == 0 {
        return out;
    }
    const DOTS: usize = 14;
    // Skip the dots right at the stations, where the text is.
    for d in 2..DOTS - 1 {
        let t = i as f32 - d as f32 / DOTS as f32;
        let p = catmull_rom(stations, t);
        let center = Vec3::new(p.x, -1.9, p.z);
        text::layout(
            "•",
            TextStyle::new(Font::Regular, 0.16, PATH).centered(),
            center,
            &mut out,
        );
    }
    out
}

/// Catmull-Rom spline through `points` at parameter `t` (0..len-1).
fn catmull_rom(points: &[Vec3], t: f32) -> Vec3 {
    let last = points.len() - 1;
    let t = t.clamp(0.0, last as f32);
    let i = (t.floor() as usize).min(last.saturating_sub(1));
    let s = t - i as f32;
    let p = |k: isize| points[(i as isize + k).clamp(0, last as isize) as usize];
    let (p0, p1, p2, p3) = (p(-1), p(0), p(1), p(2));
    0.5 * ((2.0 * p1)
        + (p2 - p0) * s
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * s * s
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * s * s * s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_passes_through_points() {
        let points = [
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, -6.0),
            Vec3::new(-1.0, 0.0, -12.0),
        ];
        for (i, &p) in points.iter().enumerate() {
            assert!(catmull_rom(&points, i as f32).distance(p) < 1e-5);
        }
    }

    #[test]
    fn builds_a_station_per_job() {
        let resume = crate::content::resume();
        let scene = Scene::new(&resume);
        assert_eq!(scene.station_count(), 1 + resume.work.len());
        assert!(!scene.glyphs.is_empty());
    }
}
