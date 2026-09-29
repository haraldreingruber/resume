//! Layout of the skill map: the skills, grouped by category, and the jobs and
//! projects that used them. A constellation that stays readable: entries sit
//! in a column on the left (in timeline order, labels ending at their dot),
//! skill groups flow into up to three columns on the right, and curves link
//! each entry to its skills (drawn by `scene.rs`). Everything is measured, so
//! no two labels overlap.

use glam::Vec2;

use crate::render::text::{self, Font, TextStyle};

/// Label sizes (world units per em, before `Params::text_scale`).
const ENTRY_SIZE: f32 = 0.062;
const SKILL_SIZE: f32 = 0.056;
const HEADING_SIZE: f32 = 0.058;
/// From a dot to its label.
const LABEL_GAP: f32 = 0.07;
/// Vertical space after a skill row and after a group.
const ROW_GAP: f32 = 0.045;
const GROUP_GAP: f32 = 0.11;
/// Around dot and label, for hit regions and focus rings.
pub const PAD: f32 = 0.025;

/// Layout parameters per screen shape (see `scene::Metrics`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// Multiplier for the label sizes.
    pub text_scale: f32,
    /// Width of the entry column, up to and including its dots.
    pub entry_column: f32,
    /// Space between the entry dots and the first skill column.
    pub edge_span: f32,
    /// At most this many skill columns.
    pub columns: usize,
    /// Rows of entries are at most this far apart.
    pub entry_step: f32,
}

impl Params {
    pub const WIDE: Self = Self {
        text_scale: 1.0,
        entry_column: 1.35,
        edge_span: 0.5,
        columns: 3,
        entry_step: 0.27,
    };
    /// Portrait phones: two narrow skill columns (labels wrap), so the map
    /// stays clear of the floor path; entries spread over the height.
    pub const COMPACT: Self = Self {
        text_scale: 1.12,
        entry_column: 0.8,
        edge_span: 0.22,
        columns: 2,
        entry_step: 0.4,
    };
}

/// A job or project with the skills it used.
pub struct Entry<'a> {
    pub label: &'a str,
    pub station: usize,
    pub skills: &'a [String],
}

/// A skill group: heading and keywords.
pub struct Group<'a> {
    pub name: &'a str,
    pub keywords: &'a [String],
}

pub struct Layout {
    /// Entries first (top to bottom), then skills (group by group), which is
    /// also the reading and Tab order.
    pub nodes: Vec<Node>,
    pub headings: Vec<Label>,
    /// Links as (entry node, skill node) indices into `nodes`.
    pub edges: Vec<(usize, usize)>,
}

pub struct Node {
    pub dot: Vec2,
    pub label: Label,
    /// Around dot and label (hit region, focus ring).
    pub bounds: [f32; 4],
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeKind {
    Entry {
        station: usize,
    },
    /// Index into the groups.
    Skill {
        group: usize,
    },
}

/// Text placed at `at` (its top-left, or top-right for right-aligned text).
pub struct Label {
    pub text: String,
    pub at: Vec2,
    pub style: TextStyle,
    pub bounds: [f32; 4],
}

impl Label {
    fn new(text: &str, at: Vec2, style: TextStyle) -> Self {
        let [width, height] = text::measure(text, style);
        let x0 = match style.align {
            text::Align::Right => at.x - width,
            text::Align::Center => at.x - width / 2.0,
            text::Align::Left => at.x,
        };
        Self {
            text: text.to_owned(),
            at,
            style,
            bounds: [x0, at.y - height, x0 + width, at.y],
        }
    }

    fn height(&self) -> f32 {
        self.bounds[3] - self.bounds[1]
    }
}

/// Lays out the map in `area` (x0, y0 bottom, x1, y1 top). Text styles carry
/// neutral colors; the scene colors them.
pub fn layout(entries: &[Entry], groups: &[Group], area: [f32; 4], params: &Params) -> Layout {
    let [x0, y0, x1, y1] = area;
    let mut nodes = Vec::new();
    let (entry_size, skill_size, heading_size) = (
        ENTRY_SIZE * params.text_scale,
        SKILL_SIZE * params.text_scale,
        HEADING_SIZE * params.text_scale,
    );

    // Entries: right-aligned labels ending just left of their dots.
    let entry_dot_x = x0 + params.entry_column;
    let step = if entries.len() > 1 {
        ((y1 - y0 - entry_size * 1.3) / (entries.len() - 1) as f32).min(params.entry_step)
    } else {
        0.0
    };
    let entry_style = TextStyle::new(Font::Regular, entry_size, [1.0; 4])
        .wrap(params.entry_column - LABEL_GAP)
        .right_aligned();
    for (i, entry) in entries.iter().enumerate() {
        let top = y1 - i as f32 * step;
        let label = Label::new(
            entry.label,
            Vec2::new(entry_dot_x - LABEL_GAP, top),
            entry_style,
        );
        let dot = Vec2::new(entry_dot_x, top - entry_size * 0.62);
        let [lx0, ly0, _, ly1] = label.bounds;
        nodes.push(Node {
            dot,
            bounds: [lx0 - PAD, ly0 - PAD, dot.x + PAD * 1.5, ly1 + PAD],
            label,
            kind: NodeKind::Entry {
                station: entry.station,
            },
        });
    }

    // Skill groups: each a heading and rows, flowing into columns.
    let columns_x0 = entry_dot_x + params.edge_span;
    let columns = groups.len().clamp(1, params.columns);
    let column_width = (x1 - columns_x0) / columns as f32;
    let skill_style =
        TextStyle::new(Font::Regular, skill_size, [1.0; 4]).wrap(column_width - LABEL_GAP - 0.06);
    let heading_style = TextStyle::new(Font::Bold, heading_size, [1.0; 4]);
    let heights: Vec<f32> = groups
        .iter()
        .map(|group| {
            let rows: f32 = group
                .keywords
                .iter()
                .map(|k| text::measure(k, skill_style)[1] + ROW_GAP)
                .sum();
            text::measure(group.name, heading_style)[1] + ROW_GAP + rows + GROUP_GAP
        })
        .collect();
    let mut headings = Vec::new();
    for (column, range) in split_columns(&heights, columns).into_iter().enumerate() {
        let dot_x = columns_x0 + column as f32 * column_width;
        let mut top = y1;
        for g in range {
            let group = &groups[g];
            let heading = Label::new(group.name, Vec2::new(dot_x - 0.02, top), heading_style);
            top -= heading.height() + ROW_GAP;
            headings.push(heading);
            for keyword in group.keywords {
                let label = Label::new(keyword, Vec2::new(dot_x + LABEL_GAP, top), skill_style);
                let dot = Vec2::new(dot_x, top - skill_size * 0.62);
                let [_, ly0, lx1, ly1] = label.bounds;
                top -= label.height() + ROW_GAP;
                nodes.push(Node {
                    dot,
                    bounds: [
                        dot.x - PAD * 1.5,
                        ly0 - PAD * 0.5,
                        lx1 + PAD,
                        ly1 + PAD * 0.5,
                    ],
                    label,
                    kind: NodeKind::Skill { group: g },
                });
            }
            top -= GROUP_GAP;
        }
    }

    // Edges, by keyword.
    let skill_node = |keyword: &str| {
        nodes
            .iter()
            .position(|n| matches!(n.kind, NodeKind::Skill { .. }) && n.label.text == keyword)
    };
    let edges = entries
        .iter()
        .enumerate()
        .flat_map(|(e, entry)| {
            entry
                .skills
                .iter()
                .filter_map(move |k| skill_node(k).map(|s| (e, s)))
                .collect::<Vec<_>>()
        })
        .collect();
    Layout {
        nodes,
        headings,
        edges,
    }
}

/// Splits consecutive groups into at most `columns` columns so that the
/// tallest column is as short as possible (few groups: brute force).
// Lists of column ranges, not ranges of numbers.
#[allow(clippy::single_range_in_vec_init)]
fn split_columns(heights: &[f32], columns: usize) -> Vec<std::ops::Range<usize>> {
    let n = heights.len();
    let height = |range: std::ops::Range<usize>| heights[range].iter().sum::<f32>();
    let mut best: Option<(f32, Vec<std::ops::Range<usize>>)> = None;
    // Every way to cut 0..n into `columns` non-empty consecutive ranges
    // (fewer columns when there are fewer groups).
    let cuts = columns.min(n).saturating_sub(1);
    let mut candidate = |ranges: Vec<std::ops::Range<usize>>| {
        let tallest = ranges.iter().map(|r| height(r.clone())).fold(0.0, f32::max);
        if best.as_ref().is_none_or(|(b, _)| tallest < *b) {
            best = Some((tallest, ranges));
        }
    };
    match cuts {
        0 => candidate(vec![0..n]),
        1 => (1..n).for_each(|a| candidate(vec![0..a, a..n])),
        _ => {
            for a in 1..n {
                for b in a + 1..n {
                    candidate(vec![0..a, a..b, b..n]);
                }
            }
        }
    }
    best.map_or_else(Vec::new, |(_, ranges)| ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps([a0, b0, a1, b1]: [f32; 4], [c0, d0, c1, d1]: [f32; 4]) -> bool {
        a0 < c1 && c0 < a1 && b0 < d1 && d0 < b1
    }

    /// The real content, as the scene builds it.
    fn real_layout(area: [f32; 4], params: &Params) -> (Layout, usize, usize) {
        let resume = crate::content::resume();
        let entries: Vec<Entry> = resume
            .work
            .iter()
            .map(|w| Entry {
                label: w.short_label(),
                station: 0,
                skills: &w.skills,
            })
            .chain(resume.projects.iter().map(|p| Entry {
                label: p.short_label(),
                station: 0,
                skills: &p.skills,
            }))
            .filter(|e| !e.skills.is_empty())
            .collect();
        let groups: Vec<Group> = resume
            .skills
            .iter()
            .map(|g| Group {
                name: &g.name,
                keywords: &g.keywords,
            })
            .collect();
        let references = entries.iter().map(|e| e.skills.len()).sum();
        let keywords = groups.iter().map(|g| g.keywords.len()).sum();
        (
            layout(&entries, &groups, area, params),
            references,
            keywords,
        )
    }

    /// Roughly the areas the scene gives the map (wide, and compact on a
    /// portrait phone).
    const WIDE_AREA: [f32; 4] = [-2.2, -2.1, 2.2, 0.1];
    const COMPACT_AREA: [f32; 4] = [-1.2, -2.4, 1.2, 0.6];

    #[test]
    fn nothing_overlaps_and_everything_fits() {
        for (area, params) in [(WIDE_AREA, Params::WIDE), (COMPACT_AREA, Params::COMPACT)] {
            let (map, _, _) = real_layout(area, &params);
            let boxes: Vec<[f32; 4]> = map
                .nodes
                .iter()
                .map(|n| n.bounds)
                .chain(map.headings.iter().map(|h| h.bounds))
                .collect();
            for (i, a) in boxes.iter().enumerate() {
                for b in &boxes[i + 1..] {
                    assert!(!overlaps(*a, *b), "{a:?} overlaps {b:?}");
                }
                let [x0, y0, x1, y1] = *a;
                let wide = x0 >= area[0] - PAD * 2.0 && x1 <= area[2] + PAD * 2.0;
                let tall = y0 >= area[1] - PAD * 2.0 && y1 <= area[3] + PAD * 2.0;
                assert!(wide && tall, "{a:?} outside {area:?}");
            }
        }
    }

    #[test]
    fn links_every_skill_reference_once() {
        let (map, references, keywords) = real_layout(WIDE_AREA, &Params::WIDE);
        let skills = map
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Skill { .. }))
            .count();
        assert_eq!(skills, keywords);
        assert_eq!(map.edges.len(), references);
        // Entries come first, then skills.
        let first_skill = map.nodes.len() - skills;
        for &(e, s) in &map.edges {
            assert!(e < first_skill && s >= first_skill);
        }
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn balances_groups_over_columns() {
        let columns = split_columns(&[5.0, 3.0, 3.0, 5.0, 5.0], 3);
        assert_eq!(columns, [0..2, 2..4, 4..5]);
        assert_eq!(split_columns(&[1.0], 3), [0..1]);
        assert_eq!(split_columns(&[1.0, 1.0], 3), [0..1, 1..2]);
    }
}
