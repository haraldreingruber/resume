//! Bakes content and fonts at build time, so the app ships no YAML, Markdown
//! or font parser and no font files:
//! - `resume.postcard`: the normalized resume from `content/resume.yaml`
//! - `atlas.png` + `glyphs.rs`: a multi-channel signed distance field (MSDF)
//!   atlas holding exactly the glyphs the content can display, plus metrics.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

use fdsm::bezier::scanline::FillRule;
use fdsm::generate::generate_msdf;
use fdsm::render::correct_sign_msdf;
use fdsm::shape::Shape;
use fdsm::transform::Transform;
use image::{RgbImage, RgbaImage};
use nalgebra::{Affine2, Similarity2, Vector2};
use resume_model::Resume;
use ttf_parser::Face;

/// Font files, indexed by `text::Font` in the app.
const FONTS: [&str; 2] = [
    "assets/fonts/Lato-Regular.ttf",
    "assets/fonts/Lato-Bold.ttf",
];
/// Atlas pixels per em.
const EM_PX: f64 = 40.0;
/// Width of the distance field around each glyph edge, in atlas pixels.
const RANGE_PX: f64 = 4.0;
const ATLAS_WIDTH: u32 = 1024;
const PADDING: u32 = 1;
/// Characters that normalization or the app may add to the YAML's own.
const EXTRA_CHARS: &str = "–—·“”‘’…•";

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let content = manifest.join("../content/resume.yaml");
    println!("cargo:rerun-if-changed={}", content.display());
    for font in FONTS {
        println!("cargo:rerun-if-changed={font}");
    }

    let resume = resume_model::load_file(&content).unwrap_or_else(|error| panic!("{error}"));
    let bytes = postcard::to_allocvec(&resume).expect("serialize resume");
    fs::write(out.join("resume.postcard"), bytes).unwrap();

    let chars = charset(&resume);
    bake_atlas(&manifest, &out, &chars);
}

/// Every character the normalized resume content can display, plus printable
/// ASCII and typographic characters the app hardcodes for UI (not sourced
/// from the resume, e.g. the path's "•" in scene.rs).
///
/// Uses `Resume::all_text()` (every field, exhaustively) rather than the raw
/// YAML, so characters produced only by Markdown/entity normalization --
/// e.g. `&nbsp;` becoming an actual U+00A0 -- still get a glyph baked.
fn charset(resume: &Resume) -> BTreeSet<char> {
    resume
        .all_text()
        .chars()
        .chain(' '..='~')
        .chain(EXTRA_CHARS.chars())
        .filter(|c| !c.is_control())
        .collect()
}

struct Glyph {
    font: usize,
    ch: char,
    /// Horizontal advance in em.
    advance: f64,
    /// Quad bounds relative to the pen position, in em: left, bottom, right, top.
    plane: [f64; 4],
    /// `None` for glyphs without outline (e.g. space).
    image: Option<RgbImage>,
    /// Atlas position of the image (top-left), assigned when packing.
    atlas_pos: (u32, u32),
}

struct Metrics {
    ascender: f64,
    descender: f64,
    line_height: f64,
}

fn bake_atlas(manifest: &Path, out: &Path, chars: &BTreeSet<char>) {
    let mut glyphs = Vec::new();
    let mut metrics = Vec::new();
    for (font, path) in FONTS.iter().enumerate() {
        let data = fs::read(manifest.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        let face = Face::parse(&data, 0).unwrap_or_else(|e| panic!("{path}: {e}"));
        let upem = f64::from(face.units_per_em());
        metrics.push(Metrics {
            ascender: f64::from(face.ascender()) / upem,
            descender: f64::from(face.descender()) / upem,
            line_height: f64::from(face.ascender() - face.descender() + face.line_gap()) / upem,
        });
        for &ch in chars {
            if let Some(glyph) = bake_glyph(&face, font, ch) {
                glyphs.push(glyph);
            }
        }
    }

    let height = pack(&mut glyphs);
    let mut atlas = RgbaImage::new(ATLAS_WIDTH, height);
    for glyph in &glyphs {
        if let Some(image) = &glyph.image {
            for (x, y, pixel) in image.enumerate_pixels() {
                let [r, g, b] = pixel.0;
                atlas.put_pixel(
                    glyph.atlas_pos.0 + x,
                    glyph.atlas_pos.1 + y,
                    image::Rgba([r, g, b, 255]),
                );
            }
        }
    }
    atlas.save(out.join("atlas.png")).expect("write atlas");
    fs::write(
        out.join("glyphs.rs"),
        glyph_table(&glyphs, &metrics, height),
    )
    .unwrap();
}

fn bake_glyph(face: &Face, font: usize, ch: char) -> Option<Glyph> {
    let id = face.glyph_index(ch)?;
    let upem = f64::from(face.units_per_em());
    let advance = f64::from(face.glyph_hor_advance(id).unwrap_or(0)) / upem;
    let outline = face
        .glyph_bounding_box(id)
        .zip(fdsm_ttf_parser::load_shape_from_face(face, id));
    let Some((bbox, mut shape)) = outline else {
        // No outline (e.g. space): advance only.
        return Some(Glyph {
            font,
            ch,
            advance,
            plane: [0.0; 4],
            image: None,
            atlas_pos: (0, 0),
        });
    };

    // Font units per atlas pixel.
    let shrinkage = upem / EM_PX;
    let transformation = nalgebra::convert::<_, Affine2<f64>>(Similarity2::new(
        Vector2::new(
            RANGE_PX - f64::from(bbox.x_min) / shrinkage,
            RANGE_PX - f64::from(bbox.y_min) / shrinkage,
        ),
        0.0,
        1.0 / shrinkage,
    ));
    let width = ((f64::from(bbox.x_max) - f64::from(bbox.x_min)) / shrinkage + 2.0 * RANGE_PX)
        .ceil() as u32;
    let height = ((f64::from(bbox.y_max) - f64::from(bbox.y_min)) / shrinkage + 2.0 * RANGE_PX)
        .ceil() as u32;

    shape.transform(&transformation);
    let prepared = Shape::edge_coloring_simple(shape, 0.03, 69441337420).prepare();
    let mut msdf = RgbImage::new(width, height);
    generate_msdf(&prepared, RANGE_PX, &mut msdf);
    correct_sign_msdf(&mut msdf, &prepared, FillRule::Nonzero);
    // Font y points up, image rows go down.
    image::imageops::flip_vertical_in_place(&mut msdf);

    let left = (f64::from(bbox.x_min) - RANGE_PX * shrinkage) / upem;
    let bottom = (f64::from(bbox.y_min) - RANGE_PX * shrinkage) / upem;
    let plane = [
        left,
        bottom,
        left + f64::from(width) * shrinkage / upem,
        bottom + f64::from(height) * shrinkage / upem,
    ];
    Some(Glyph {
        font,
        ch,
        advance,
        plane,
        image: Some(msdf),
        atlas_pos: (0, 0),
    })
}

/// Shelf packing, tallest first. Returns the atlas height.
fn pack(glyphs: &mut [Glyph]) -> u32 {
    let mut order: Vec<usize> = (0..glyphs.len())
        .filter(|&i| glyphs[i].image.is_some())
        .collect();
    order.sort_by_key(|&i| std::cmp::Reverse(glyphs[i].image.as_ref().unwrap().height()));
    let (mut x, mut y, mut shelf) = (0, 0, 0);
    for i in order {
        let image = glyphs[i].image.as_ref().unwrap();
        let (w, h) = (image.width() + PADDING, image.height() + PADDING);
        if x + w > ATLAS_WIDTH {
            x = 0;
            y += shelf;
            shelf = 0;
        }
        glyphs[i].atlas_pos = (x, y);
        x += w;
        shelf = shelf.max(h);
    }
    (y + shelf).next_multiple_of(4)
}

/// Rust source with the glyph metrics, sorted by (font, char) for lookup.
fn glyph_table(glyphs: &[Glyph], metrics: &[Metrics], height: u32) -> String {
    let mut sorted: Vec<&Glyph> = glyphs.iter().collect();
    sorted.sort_by_key(|g| (g.font, g.ch));
    let (w, h) = (f64::from(ATLAS_WIDTH), f64::from(height));

    let mut out = String::from("// Generated by build.rs.\n");
    let _ = writeln!(
        out,
        "pub const ATLAS_SIZE: [u32; 2] = [{ATLAS_WIDTH}, {height}];"
    );
    let _ = writeln!(
        out,
        "pub const DISTANCE_RANGE_PX: f32 = {:?};",
        RANGE_PX as f32
    );
    out.push_str("pub const FONT_METRICS: &[FontMetrics] = &[\n");
    for m in metrics {
        let _ = writeln!(
            out,
            "    FontMetrics {{ ascender: {:?}, descender: {:?}, line_height: {:?} }},",
            m.ascender as f32, m.descender as f32, m.line_height as f32
        );
    }
    out.push_str("];\npub const GLYPHS: &[GlyphInfo] = &[\n");
    for g in sorted {
        let uv = match &g.image {
            Some(image) => {
                let (x, y) = g.atlas_pos;
                [
                    f64::from(x) / w,
                    f64::from(y) / h,
                    f64::from(x + image.width()) / w,
                    f64::from(y + image.height()) / h,
                ]
            }
            None => [0.0; 4],
        };
        let f = |v: [f64; 4]| v.map(|x| format!("{:?}", x as f32)).join(", ");
        let _ = writeln!(
            out,
            "    GlyphInfo {{ font: {}, ch: {:?}, advance: {:?}, plane: [{}], uv: [{}], visible: {} }},",
            g.font,
            g.ch,
            g.advance as f32,
            f(g.plane),
            f(uv),
            g.image.is_some()
        );
    }
    out.push_str("];\n");
    out
}
