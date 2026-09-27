//! The timeline scene: an intro, then every work, project and education entry
//! with the most recent first, then an outro with languages and contact
//! links. Stations lie along a gently winding path into the screen (-z); the
//! camera flies along a Catmull-Rom spline through them, so scrolling forward
//! travels back in time.

use std::ops::Range;

use glam::{Mat4, Vec2, Vec3};
use resume_model::{DateRange, Education, PartialDate, Project, Resume, RichText, Work};

use crate::shapes::ShapeInstance;
use crate::text::{self, Font, GlyphInstance, Run, TextStyle, rgb, rgb_bytes};

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
/// Height of the path dots below the stations: low enough that the path
/// ahead stays below a station's text as it recedes.
const FLOOR_Y: f32 = -2.4;
/// Top of the year labels, just above the bottom of the screen.
const YEAR_TOP_Y: f32 = -1.62;

/// Content fades in/out relative to the focus distance: stations the camera
/// passes fade out, the next one fades in as the camera approaches. Shared
/// with the shaders (via the renderer) and with picking.
pub const NEAR_FADE: (f32, f32) = (-2.8, -1.2);
pub const FAR_FADE: (f32, f32) = (1.8, 4.5);

/// Hover groups are indices into a 64-entry uniform array; 0 means none.
pub const MAX_GROUPS: usize = 64;
/// The group of the name on the intro: reserved (never a link), so the
/// renderer can fade the crisp title while the particles form it.
pub const TITLE_GROUP: u32 = MAX_GROUPS as u32 - 1;

const NAME: [f32; 4] = rgb(0xF0F4F8);
const BODY: [f32; 4] = rgb(0xC9D4DE);
const MUTED: [f32; 4] = rgb(0x8AA0B4);
const PATH: [f32; 4] = rgb(0x3E5A73);
const CODE: [f32; 4] = rgb(0xE2C08D);
/// Station accents when an entry has no `x-color`, derived from the PDF blues.
const PALETTE: [[f32; 4]; 4] = [rgb(0x6DB3E8), rgb(0x8CC4EF), rgb(0x6FC2C9), rgb(0x9AB6E8)];

/// What clicking a link does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    OpenUrl(String),
    /// The plain HTML version (native: embedded copy; web: the HTML nav).
    TextVersion,
    Pdf,
}

pub struct Scene {
    stations: Vec<Station>,
    /// Station anchors, the control points of the camera path.
    points: Vec<Vec3>,
    /// Glyphs and shapes are ordered far to near (drawn without depth buffer).
    pub glyphs: Vec<GlyphInstance>,
    pub shapes: Vec<ShapeInstance>,
    hits: Vec<Hit>,
    /// `actions[group - 1]` is what clicking hover group `group` does.
    actions: Vec<Action>,
    pub title: Title,
}

/// The name on the intro, which the particles form.
pub struct Title {
    pub glyphs: Vec<GlyphInstance>,
    /// x0, y0, x1, y1 around the glyphs, in the plane z = `z`.
    pub bounds: [f32; 4],
    pub z: f32,
}

impl Title {
    fn new(glyphs: Vec<GlyphInstance>) -> Self {
        let bounds = glyphs.iter().fold(
            [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
            |[x0, y0, x1, y1], g| {
                let [gx0, gy0, gx1, gy1] = g.rect;
                [x0.min(gx0), y0.min(gy0), x1.max(gx1), y1.max(gy1)]
            },
        );
        let z = glyphs.first().map_or(0.0, |g| g.z);
        Self { glyphs, bounds, z }
    }

    pub fn center(&self) -> Vec3 {
        let [x0, y0, x1, y1] = self.bounds;
        Vec3::new((x0 + x1) / 2.0, (y0 + y1) / 2.0, self.z)
    }
}

struct Station {
    id: String,
    anchor: Vec3,
    /// Hover groups of the station's links, in reading order.
    links: Range<u32>,
}

impl Station {
    fn new(id: &str, anchor: Vec3) -> Self {
        Self {
            id: id.to_owned(),
            anchor,
            links: 0..0,
        }
    }
}

/// One station's glyphs and shapes, and the hover groups of its links.
struct Block {
    glyphs: Vec<GlyphInstance>,
    shapes: Vec<ShapeInstance>,
    links: Range<u32>,
}

/// A clickable rectangle in a z-plane.
struct Hit {
    rect: [f32; 4],
    z: f32,
    group: u32,
}

/// Camera parameters that only depend on the aspect ratio (cached until resize).
pub struct Lens {
    projection: Mat4,
    distance: f32,
}

pub struct Camera {
    pub eye: Vec3,
    pub view_proj: Mat4,
    /// Distance from the eye to the station in focus.
    pub focus_distance: f32,
}

/// A timeline entry of any kind.
#[derive(Clone, Copy)]
enum Entry<'a> {
    Job(&'a Work),
    Project(&'a Project),
    Education(&'a Education),
}

impl Entry<'_> {
    fn id(&self) -> &str {
        match self {
            Entry::Job(w) => &w.id,
            Entry::Project(p) => &p.id,
            Entry::Education(e) => &e.id,
        }
    }

    fn dates(&self) -> DateRange {
        match self {
            Entry::Job(w) => w.dates,
            Entry::Project(p) => p.dates,
            Entry::Education(e) => e.dates,
        }
    }

    fn accent(&self) -> Option<[u8; 3]> {
        match self {
            Entry::Job(w) => w.accent,
            Entry::Project(p) => p.accent,
            Entry::Education(e) => e.accent,
        }
    }

    /// Floor label: the year the entry ended, or "Now".
    fn year_label(&self) -> String {
        self.dates()
            .end
            .map_or_else(|| "Now".to_owned(), |end| end.year.to_string())
    }
}

/// Most recent first: by end date (ongoing first; a year-only date counts as
/// its latest possible month), then by start date.
fn timeline(resume: &Resume) -> Vec<Entry<'_>> {
    let mut entries: Vec<Entry> = resume
        .work
        .iter()
        .map(Entry::Job)
        .chain(resume.projects.iter().map(Entry::Project))
        .chain(resume.education.iter().map(Entry::Education))
        .collect();
    entries.sort_by_key(|entry| {
        let dates = entry.dates();
        let end = dates.end.map_or((u16::MAX, 12), PartialDate::latest);
        std::cmp::Reverse((end, dates.start.latest()))
    });
    entries
}

impl Scene {
    pub fn new(resume: &Resume) -> Self {
        let entries = timeline(resume);
        let count = entries.len() + 2;
        let anchor = |i: usize| {
            let side = match i {
                0 => 0.0,
                i if i == count - 1 => 0.0,
                i if i % 2 == 1 => SWAY,
                _ => -SWAY,
            };
            Vec3::new(side, 0.6, -(i as f32) * SPACING)
        };
        let mut stations = vec![Station::new("intro", anchor(0))];
        stations.extend(
            entries
                .iter()
                .enumerate()
                .map(|(i, entry)| Station::new(entry.id(), anchor(i + 1))),
        );
        stations.push(Station::new("outro", anchor(count - 1)));

        // Build each station, then emit them far to near so that nearer
        // stations are drawn on top.
        let mut blocks = Vec::with_capacity(count);
        let mut builder = Builder::default();
        blocks.push(builder.take(|b| b.intro(resume, stations[0].anchor)));
        for (i, entry) in entries.iter().enumerate() {
            let station = &stations[i + 1];
            let accent = entry.accent().map_or(PALETTE[i % PALETTE.len()], rgb_bytes);
            blocks.push(builder.take(|b| {
                match entry {
                    Entry::Job(work) => b.job(work, station.anchor, accent),
                    Entry::Project(project) => b.project(project, station.anchor, accent),
                    Entry::Education(education) => b.education(education, station.anchor, accent),
                }
                b.year_label(&entry.year_label(), station.anchor);
            }));
        }
        blocks.push(builder.take(|b| b.outro(resume, stations[count - 1].anchor)));

        let points: Vec<Vec3> = stations.iter().map(|s| s.anchor).collect();
        let title = Title::new(
            blocks[0]
                .glyphs
                .iter()
                .filter(|g| g.group == TITLE_GROUP)
                .copied()
                .collect(),
        );
        let mut glyphs = Vec::new();
        let mut shapes = Vec::new();
        for (i, block) in blocks.into_iter().enumerate().rev() {
            stations[i].links = block.links;
            shapes.extend(path_dots(&points, i));
            shapes.extend(block.shapes);
            glyphs.extend(block.glyphs);
        }
        Self {
            stations,
            points,
            glyphs,
            shapes,
            hits: builder.hits,
            actions: builder.actions,
            title,
        }
    }

    pub fn station_count(&self) -> usize {
        self.stations.len()
    }

    /// The deep-link id of a station (`intro`, `dedalus`, …, `outro`).
    pub fn station_id(&self, station: usize) -> Option<&str> {
        self.stations.get(station).map(|s| s.id.as_str())
    }

    /// Hover groups of a station's links, in reading order (keyboard focus
    /// moves through them).
    pub fn links(&self, station: usize) -> Range<u32> {
        self.stations.get(station).map_or(0..0, |s| s.links.clone())
    }

    /// A deep-link target: a station id (`dedalus`, `intro`, `outro`) or index.
    pub fn station_index(&self, key: &str) -> Option<usize> {
        self.stations
            .iter()
            .position(|s| s.id == key)
            .or_else(|| key.parse().ok().filter(|&i| i < self.stations.len()))
    }

    pub fn action(&self, group: u32) -> Option<&Action> {
        self.actions.get((group as usize).checked_sub(1)?)
    }

    /// Registers an action outside the scene (e.g. screen-space buttons) and
    /// returns its hover group, or 0 if all groups are taken.
    pub fn add_action(&mut self, action: Action) -> u32 {
        add_action(&mut self.actions, action)
    }

    /// Camera parameters for an aspect ratio; recompute only on resize.
    pub fn lens(aspect: f32) -> Lens {
        // Back off on narrow screens so text blocks fit horizontally.
        let half_fov_x = ((FOV_Y / 2.0).tan() * aspect).atan();
        Lens {
            // wgpu uses DirectX-style clip space (depth 0..1).
            projection: glam::camera::rh::proj::directx::perspective(FOV_Y, aspect, 0.1, 100.0),
            distance: (VIEW_HALF_WIDTH / half_fov_x.tan()).max(3.6),
        }
    }

    /// Camera for timeline position `t`.
    pub fn camera(&self, t: f32, lens: &Lens) -> Camera {
        let target = catmull_rom(&self.points, t) + Vec3::new(0.0, -0.9, 0.0);
        // The camera sways less than the stations for a calmer ride.
        let eye = Vec3::new(target.x * 0.6, target.y + 0.35, target.z + lens.distance);
        let view = glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Y);
        Camera {
            eye,
            view_proj: lens.projection * view,
            focus_distance: lens.distance,
        }
    }

    /// The hover group of the nearest link under `ndc` (-1..1, y up), among
    /// stations that are in focus (not faded out).
    pub fn pick(&self, camera: &Camera, ndc: Vec2) -> Option<u32> {
        let visible = (camera.focus_distance + NEAR_FADE.1)..(camera.focus_distance + FAR_FADE.0);
        self.hits
            .iter()
            .filter_map(|hit| {
                let (t, p) = camera.ray_to_plane(ndc, hit.z)?;
                let [x0, y0, x1, y1] = hit.rect;
                let inside = (x0..=x1).contains(&p.x) && (y0..=y1).contains(&p.y);
                (inside && visible.contains(&p.distance(camera.eye))).then_some((t, hit.group))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, group)| group)
    }
}

impl Camera {
    /// Where the ray through `ndc` (-1..1, y up) meets the plane at `z`, and
    /// the ray parameter there; `None` if the plane is behind the camera.
    pub fn ray_to_plane(&self, ndc: Vec2, z: f32) -> Option<(f32, Vec3)> {
        let inverse = self.view_proj.inverse();
        let origin = inverse.project_point3(ndc.extend(0.0));
        let direction = inverse.project_point3(ndc.extend(1.0)) - origin;
        if direction.z.abs() < 1e-6 {
            return None;
        }
        let t = (z - origin.z) / direction.z;
        (t > 0.0).then(|| (t, origin + direction * t))
    }
}

fn add_action(actions: &mut Vec<Action>, action: Action) -> u32 {
    // Groups are 1-based; the last one is reserved for the title.
    if actions.len() + 1 >= TITLE_GROUP as usize {
        log::warn!("out of hover groups; {action:?} won't be clickable");
        return 0;
    }
    actions.push(action);
    actions.len() as u32
}

/// Accumulates one station's glyphs and shapes; hits and actions (hover
/// groups) accumulate over the whole scene.
#[derive(Default)]
struct Builder {
    glyphs: Vec<GlyphInstance>,
    shapes: Vec<ShapeInstance>,
    hits: Vec<Hit>,
    actions: Vec<Action>,
}

impl Builder {
    /// Runs `build` and returns the glyphs, shapes and links it added.
    fn take(&mut self, build: impl FnOnce(&mut Self)) -> Block {
        let first = self.actions.len() as u32 + 1;
        build(self);
        Block {
            glyphs: std::mem::take(&mut self.glyphs),
            shapes: std::mem::take(&mut self.shapes),
            links: first..self.actions.len() as u32 + 1,
        }
    }

    /// Lays out a text block at `cursor` and moves the cursor below it.
    fn text(&mut self, text: &str, style: TextStyle, gap: f32, cursor: &mut Vec3) {
        cursor.y -= text::layout(text, style, *cursor, &mut self.glyphs) + gap;
    }

    /// Rich text: bold spans use the bold font; italic, code and links are
    /// set apart by color. Links are underlined and clickable.
    fn rich(
        &mut self,
        text: &RichText,
        style: TextStyle,
        accent: [f32; 4],
        gap: f32,
        cursor: &mut Vec3,
    ) {
        let runs: Vec<Run> = text
            .spans()
            .iter()
            .map(|span| {
                let group = span.link.as_ref().map_or(0, |url| {
                    add_action(&mut self.actions, Action::OpenUrl(url.clone()))
                });
                let color = if group != 0 || span.style.italic {
                    accent
                } else if span.style.code {
                    CODE
                } else {
                    style.color
                };
                Run {
                    text: &span.text,
                    font: if span.style.bold {
                        Font::Bold
                    } else {
                        style.font
                    },
                    color,
                    group,
                }
            })
            .collect();
        let paragraph = text::layout_runs(&runs, style, *cursor, &mut self.glyphs);
        for (group, rect) in paragraph.boxes {
            self.link_decoration(rect, cursor.z, group, accent, style.size);
        }
        cursor.y -= paragraph.height + gap;
    }

    /// A single-line clickable link (e.g. a contact detail).
    fn link(&mut self, label: &str, action: Action, style: TextStyle, gap: f32, cursor: &mut Vec3) {
        let group = add_action(&mut self.actions, action);
        let run = Run {
            text: label,
            font: style.font,
            color: style.color,
            group,
        };
        let paragraph = text::layout_runs(&[run], style, *cursor, &mut self.glyphs);
        for (group, rect) in paragraph.boxes {
            self.link_decoration(rect, cursor.z, group, style.color, style.size);
        }
        cursor.y -= paragraph.height + gap;
    }

    /// Underline, a slightly enlarged hit region and a focus ring for a
    /// link's text box.
    fn link_decoration(
        &mut self,
        [x0, y0, x1, y1]: [f32; 4],
        z: f32,
        group: u32,
        color: [f32; 4],
        size: f32,
    ) {
        let underline_y = y0 + (y1 - y0) * 0.17;
        let thickness = size * 0.06;
        self.shapes.push(
            ShapeInstance::filled(
                [x0, underline_y - thickness, x1, underline_y],
                z,
                0.0,
                color,
            )
            .group(group),
        );
        let pad = size * 0.2;
        let rect = [x0 - pad, y0 - pad, x1 + pad, y1 + pad];
        let (radius, border) = (size * 0.3, size * 0.06);
        self.shapes.push(ShapeInstance::focus_ring(
            rect, z, radius, border, color, group,
        ));
        self.hits.push(Hit { rect, z, group });
    }

    /// Outlined tags, wrapped into rows of at most `max_width` starting at
    /// the cursor (left-aligned) or centered on it.
    fn chips(
        &mut self,
        labels: &[String],
        size: f32,
        border: [f32; 4],
        max_width: f32,
        centered: bool,
        cursor: &mut Vec3,
    ) {
        if labels.is_empty() {
            return;
        }
        let (pad_x, pad_y, gap) = (size * 0.6, size * 0.35, size * 0.45);
        let height = size * 1.35 + 2.0 * pad_y;
        let widths: Vec<f32> = labels
            .iter()
            .map(|label| text::width(Font::Regular, size, label) + 2.0 * pad_x)
            .collect();
        // Split into rows first so centered rows can be positioned.
        let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
        let mut row_width = 0.0;
        for (i, &w) in widths.iter().enumerate() {
            let row = rows.last_mut().expect("at least one row");
            if !row.is_empty() && row_width + gap + w > max_width {
                rows.push(vec![i]);
                row_width = w;
            } else {
                row_width += if row.is_empty() { w } else { gap + w };
                row.push(i);
            }
        }
        let mut top = cursor.y;
        for row in rows {
            let total: f32 =
                row.iter().map(|&i| widths[i]).sum::<f32>() + gap * (row.len() - 1) as f32;
            let mut x = if centered {
                cursor.x - total / 2.0
            } else {
                cursor.x
            };
            for i in row {
                let rect = [x, top - height, x + widths[i], top];
                self.shapes.push(ShapeInstance::outlined(
                    rect,
                    cursor.z,
                    height / 2.0,
                    size * 0.08,
                    border,
                ));
                let style = TextStyle::new(Font::Regular, size, BODY);
                text::layout(
                    &labels[i],
                    style,
                    Vec3::new(x + pad_x, top - pad_y * 0.9, cursor.z),
                    &mut self.glyphs,
                );
                x += widths[i] + gap;
            }
            top -= height + gap;
        }
        cursor.y = top + gap;
    }

    fn meta_line(dates: &DateRange, location: Option<&str>) -> String {
        match location {
            Some(location) => format!("{dates}  ·  {location}"),
            None => dates.to_string(),
        }
    }

    fn intro(&mut self, resume: &Resume, anchor: Vec3) {
        let basics = &resume.basics;
        let mut cursor = anchor + Vec3::new(0.0, 0.5, 0.0);
        let name = TextStyle::new(Font::Bold, 0.42, NAME)
            .centered()
            .group(TITLE_GROUP);
        self.text(&basics.name, name, 0.12, &mut cursor);
        let label = TextStyle::new(Font::Bold, 0.14, PALETTE[0])
            .centered()
            .wrap(TAGLINE_WIDTH);
        self.text(&basics.label, label, 0.25, &mut cursor);
        let summary = TextStyle::new(Font::Regular, 0.085, BODY)
            .centered()
            .wrap(BLOCK_WIDTH)
            .line_spacing(1.15);
        self.rich(&basics.summary, summary, PALETTE[0], 0.35, &mut cursor);
        let hint = TextStyle::new(Font::Regular, 0.07, MUTED).centered();
        self.text(
            "Scroll, swipe or use the arrow keys to travel back in time",
            hint,
            0.05,
            &mut cursor,
        );
        self.text(
            "Double-click or press F for fullscreen",
            hint,
            0.0,
            &mut cursor,
        );
    }

    fn job(&mut self, work: &Work, anchor: Vec3, accent: [f32; 4]) {
        let mut cursor = anchor - Vec3::new(BLOCK_WIDTH / 2.0, 0.0, 0.0);
        let meta = Self::meta_line(&work.dates, work.location.as_deref());
        self.text(
            &meta,
            TextStyle::new(Font::Regular, 0.075, MUTED),
            0.08,
            &mut cursor,
        );
        let position = TextStyle::new(Font::Bold, 0.16, NAME).wrap(BLOCK_WIDTH);
        self.text(&work.position, position, 0.06, &mut cursor);
        let organization = TextStyle::new(Font::Bold, 0.11, accent);
        self.text(&work.organization, organization, 0.18, &mut cursor);
        if let Some(summary) = &work.summary {
            let style = TextStyle::new(Font::Regular, 0.08, BODY)
                .wrap(BLOCK_WIDTH)
                .line_spacing(1.15);
            self.rich(summary, style, accent, 0.16, &mut cursor);
        }
        self.chips(&work.skills, 0.06, accent, BLOCK_WIDTH, false, &mut cursor);
    }

    fn project(&mut self, project: &Project, anchor: Vec3, accent: [f32; 4]) {
        let mut cursor = anchor - Vec3::new(BLOCK_WIDTH / 2.0, 0.0, 0.0);
        let meta = Self::meta_line(&project.dates, project.location.as_deref());
        self.text(
            &meta,
            TextStyle::new(Font::Regular, 0.075, MUTED),
            0.08,
            &mut cursor,
        );
        let heading = TextStyle::new(Font::Bold, 0.11, accent).wrap(BLOCK_WIDTH);
        self.text(&project.heading(), heading, 0.08, &mut cursor);
        let title = TextStyle::new(Font::Bold, 0.14, NAME).wrap(BLOCK_WIDTH);
        self.text(&format!("“{}”", project.title), title, 0.16, &mut cursor);
        if let Some(description) = &project.description {
            let style = TextStyle::new(Font::Regular, 0.08, BODY)
                .wrap(BLOCK_WIDTH)
                .line_spacing(1.15);
            self.rich(description, style, accent, 0.16, &mut cursor);
        }
        self.chips(
            &project.skills,
            0.06,
            accent,
            BLOCK_WIDTH,
            false,
            &mut cursor,
        );
    }

    fn education(&mut self, education: &Education, anchor: Vec3, accent: [f32; 4]) {
        let mut cursor = anchor - Vec3::new(BLOCK_WIDTH / 2.0, 0.0, 0.0);
        let meta = Self::meta_line(&education.dates, education.location.as_deref());
        self.text(
            &meta,
            TextStyle::new(Font::Regular, 0.075, MUTED),
            0.08,
            &mut cursor,
        );
        let title = TextStyle::new(Font::Bold, 0.16, NAME).wrap(BLOCK_WIDTH);
        self.text(&education.title(), title, 0.06, &mut cursor);
        let institution = TextStyle::new(Font::Bold, 0.11, accent).wrap(BLOCK_WIDTH);
        self.text(&education.institution, institution, 0.18, &mut cursor);
        if let Some(courses) = education.courses_sentence() {
            let style = TextStyle::new(Font::Regular, 0.08, BODY)
                .wrap(BLOCK_WIDTH)
                .line_spacing(1.15);
            self.text(&courses, style, 0.0, &mut cursor);
        }
    }

    fn outro(&mut self, resume: &Resume, anchor: Vec3) {
        let accent = PALETTE[0];
        let mut cursor = anchor + Vec3::new(0.0, 0.3, 0.0);
        let heading = |text| (text, TextStyle::new(Font::Bold, 0.16, NAME).centered());
        let (text, style) = heading("Languages");
        self.text(text, style, 0.14, &mut cursor);
        let languages: Vec<String> = resume.languages.iter().map(ToString::to_string).collect();
        self.chips(&languages, 0.075, accent, BLOCK_WIDTH, true, &mut cursor);
        cursor.y -= 0.3;
        let (text, style) = heading("Get in touch");
        self.text(text, style, 0.16, &mut cursor);

        let link_style = TextStyle::new(Font::Regular, 0.09, accent).centered();
        let basics = &resume.basics;
        let email = Action::OpenUrl(format!("mailto:{}", basics.email));
        self.link(&basics.email, email, link_style, 0.1, &mut cursor);
        for (label, profile) in basics.labeled_profiles() {
            self.link(
                &label,
                Action::OpenUrl(profile.url.clone()),
                link_style,
                0.1,
                &mut cursor,
            );
        }
        cursor.y -= 0.25;
        let hint = TextStyle::new(Font::Regular, 0.07, MUTED).centered();
        self.text("Press Home to return to the start", hint, 0.0, &mut cursor);
    }

    /// The entry's year on the floor below the station.
    fn year_label(&mut self, label: &str, anchor: Vec3) {
        let style = TextStyle::new(Font::Bold, 0.14, PATH).centered();
        let top = Vec3::new(anchor.x, YEAR_TOP_Y, anchor.z);
        text::layout(label, style, top, &mut self.glyphs);
    }
}

/// Dots along the path from station `i` towards the previous one, on the floor.
fn path_dots(stations: &[Vec3], i: usize) -> Vec<ShapeInstance> {
    const DOTS: usize = 14;
    if i == 0 {
        return Vec::new();
    }
    // Skip the dots right at the stations, where the year labels are.
    (2..DOTS - 1)
        .map(|d| {
            let p = catmull_rom(stations, i as f32 - d as f32 / DOTS as f32);
            ShapeInstance::dot([p.x, FLOOR_Y], p.z, 0.035, PATH)
        })
        .collect()
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
    use crate::shapes::FOCUS_RING;

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
    fn orders_entries_most_recent_first() {
        let resume = crate::content::resume();
        let scene = Scene::new(&resume);
        let ids: Vec<&str> = scene.stations.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "intro",
                "dedalus",
                "mob-tour",
                "three10",
                "viewar",
                "agfa-volume-rendering",
                "agfa-3d-visualization",
                "msc-tu-wien",
                "masters-thesis",
                "exchange-uab",
                "final-year-project",
                "bsc-hagenberg",
                "outro",
            ]
        );
        assert!(!scene.glyphs.is_empty() && !scene.shapes.is_empty());
    }

    #[test]
    fn resolves_deep_links_by_id_or_index() {
        let scene = Scene::new(&crate::content::resume());
        assert_eq!(scene.station_index("intro"), Some(0));
        assert_eq!(scene.station_index("viewar"), Some(4));
        assert_eq!(scene.station_index("3"), Some(3));
        assert_eq!(scene.station_index("99"), None);
        assert_eq!(scene.station_index("unknown"), None);
        // The address bar shows ids; they lead back to the same station.
        for i in 0..scene.station_count() {
            let id = scene.station_id(i).unwrap();
            assert_eq!(scene.station_index(id), Some(i));
        }
        assert_eq!(scene.station_id(99), None);
    }

    #[test]
    fn stations_own_their_links_in_order() {
        let scene = Scene::new(&crate::content::resume());
        let outro = scene.station_count() - 1;
        // The outro's links are its contact details, in reading order.
        let targets: Vec<&Action> = scene
            .links(outro)
            .filter_map(|group| scene.action(group))
            .collect();
        assert_eq!(
            targets.first(),
            Some(&&Action::OpenUrl(
                "mailto:harald.reingruber@gmail.com".into()
            ))
        );
        assert_eq!(targets.len(), 4);
        // Every clickable group belongs to exactly one station.
        let owned: usize = (0..scene.station_count())
            .map(|i| scene.links(i).len())
            .sum();
        assert_eq!(owned, scene.actions.len());
    }

    #[test]
    fn the_title_has_a_reserved_group_without_an_action() {
        let resume = crate::content::resume();
        let mut scene = Scene::new(&resume);
        let title = &scene.title;
        let letters = resume.basics.name.chars().filter(|c| !c.is_whitespace());
        assert_eq!(title.glyphs.len(), letters.count());
        assert!(title.bounds[0] < title.bounds[2] && title.bounds[1] < title.bounds[3]);
        assert_eq!(scene.action(TITLE_GROUP), None);
        // Adding actions never hands out the title's group.
        while let group @ 1.. = scene.add_action(Action::Pdf) {
            assert!(group < TITLE_GROUP);
        }
    }

    #[test]
    fn every_link_region_has_a_focus_ring() {
        let scene = Scene::new(&crate::content::resume());
        for hit in &scene.hits {
            assert!(
                scene
                    .shapes
                    .iter()
                    .any(|s| s.group == hit.group | FOCUS_RING && s.rect == hit.rect),
                "group {}",
                hit.group
            );
        }
    }

    #[test]
    fn outro_links_are_clickable_actions() {
        let resume = crate::content::resume();
        let scene = Scene::new(&resume);
        let urls: Vec<&str> = scene
            .actions
            .iter()
            .filter_map(|a| match a {
                Action::OpenUrl(url) => Some(url.as_str()),
                _ => None,
            })
            .collect();
        assert!(urls.contains(&"mailto:harald.reingruber@gmail.com"));
        assert!(urls.contains(&"https://github.com/haraldreingruber"));
        assert!(urls.contains(&"https://www.linkedin.com/in/haraldreingruber"));
        // Every action has at least one hit region (a wrapped link has one per line).
        for group in 1..=scene.actions.len() as u32 {
            assert!(
                scene.hits.iter().any(|hit| hit.group == group),
                "group {group}"
            );
        }
    }

    #[test]
    fn picks_the_link_under_the_cursor_only_when_in_focus() {
        let resume = crate::content::resume();
        let scene = Scene::new(&resume);
        let lens = Scene::lens(16.0 / 9.0);
        let outro = scene.station_count() - 1;
        let camera = scene.camera(outro as f32, &lens);
        // Project the center of the first hit region to NDC and pick there.
        let hit = &scene.hits[0];
        let center = Vec3::new(
            (hit.rect[0] + hit.rect[2]) / 2.0,
            (hit.rect[1] + hit.rect[3]) / 2.0,
            hit.z,
        );
        let ndc = camera.view_proj.project_point3(center).truncate();
        assert_eq!(scene.pick(&camera, ndc), Some(hit.group));
        // Away from any link: nothing.
        assert_eq!(scene.pick(&camera, Vec2::new(-0.99, 0.99)), None);
        // From the intro, the outro is faded out and not clickable.
        let far_away = scene.camera(0.0, &lens);
        let ndc = far_away.view_proj.project_point3(center).truncate();
        assert_eq!(scene.pick(&far_away, ndc), None);
    }
}
