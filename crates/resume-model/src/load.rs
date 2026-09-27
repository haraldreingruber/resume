//! Parse `resume.yaml`, validate it and normalize it into the domain model.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::domain::*;
use crate::source;

/// ISO 3166-1 alpha-2 codes accepted in `countryCode` fields.
const COUNTRIES: &[(&str, &str)] = &[
    ("AT", "Austria"),
    ("BE", "Belgium"),
    ("CA", "Canada"),
    ("CH", "Switzerland"),
    ("DE", "Germany"),
    ("ES", "Spain"),
    ("FR", "France"),
    ("GB", "United Kingdom"),
    ("IT", "Italy"),
    ("NL", "Netherlands"),
    ("RO", "Romania"),
    ("US", "United States"),
];

/// Characters that can't appear in contact fields: LaTeX passes those through
/// verbatim (AltaCV detokenizes them), so they would break the PDF.
const CONTACT_FORBIDDEN: &[char] = &['\\', '{', '}', '%', '#', '^'];

#[derive(Debug)]
pub enum LoadError {
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    /// YAML syntax error or a field with the wrong shape; includes line/column.
    Parse(serde_saphyr::Error),
    /// Semantic validation errors, each prefixed with the field path.
    Invalid(Vec<String>),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io { path, error } => write!(f, "cannot read {}: {error}", path.display()),
            LoadError::Parse(error) => write!(f, "invalid resume YAML: {error}"),
            LoadError::Invalid(errors) => {
                writeln!(f, "invalid resume content:")?;
                for error in errors {
                    writeln!(f, "  - {error}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for LoadError {}

pub fn load_file(path: impl AsRef<Path>) -> Result<Resume, LoadError> {
    let path = path.as_ref();
    let yaml = std::fs::read_to_string(path).map_err(|error| LoadError::Io {
        path: path.to_owned(),
        error,
    })?;
    load_str(&yaml)
}

pub fn load_str(yaml: &str) -> Result<Resume, LoadError> {
    let source: source::Source = serde_saphyr::from_str(yaml).map_err(LoadError::Parse)?;
    let mut normalizer = Normalizer::default();
    let resume = normalizer.resume(source);
    if normalizer.errors.is_empty() {
        Ok(resume)
    } else {
        Err(LoadError::Invalid(normalizer.errors))
    }
}

#[derive(Default)]
struct Normalizer {
    errors: Vec<String>,
    ids: HashSet<String>,
    /// `x-places` id -> display string ("City, Country").
    places: BTreeMap<String, String>,
    /// Every skill keyword, for validating `x-skills` references.
    keywords: BTreeSet<String>,
}

impl Normalizer {
    fn error(&mut self, path: &str, message: impl fmt::Display) {
        self.errors.push(format!("{path}: {message}"));
    }

    fn resume(&mut self, source: source::Source) -> Resume {
        // Places and skills first: entries refer to them.
        for (id, place) in source.places {
            let display = self.place(&id, place);
            self.places.insert(id, display);
        }
        let skills = self.list(source.skills, "skills", Self::skill);
        self.keywords = skills
            .iter()
            .flat_map(|group| group.keywords.iter().cloned())
            .collect();
        Resume {
            basics: self.basics(source.basics),
            work: self.list(source.work, "work", Self::work),
            projects: self.list(source.projects, "projects", Self::project),
            skills,
            education: self.list(source.education, "education", Self::education),
            languages: self.list(source.languages, "languages", |n, path, language| {
                Language {
                    language: n.text(&format!("{path}.language"), language.language),
                    fluency: language
                        .fluency
                        .map(|fluency| n.text(&format!("{path}.fluency"), fluency)),
                }
            }),
        }
    }

    /// Normalizes each element of a list, passing its field path (`work[2]`).
    fn list<S, T>(
        &mut self,
        items: Vec<S>,
        name: &str,
        mut normalize: impl FnMut(&mut Self, &str, S) -> T,
    ) -> Vec<T> {
        items
            .into_iter()
            .enumerate()
            .map(|(i, item)| normalize(self, &format!("{name}[{i}]"), item))
            .collect()
    }

    fn basics(&mut self, basics: source::Basics) -> Basics {
        let country = basics
            .location
            .country_code
            .and_then(|code| self.country("basics.location.countryCode", &code));
        Basics {
            name: self.text("basics.name", basics.name),
            label: self.text("basics.label", basics.label),
            email: self.contact("basics.email", basics.email),
            location: Location {
                city: self.contact("basics.location.city", basics.location.city),
                region: basics
                    .location
                    .region
                    .map(|region| self.contact("basics.location.region", region)),
                country,
            },
            profiles: self.list(basics.profiles, "basics.profiles", |n, path, profile| {
                Profile {
                    network: n.text(&format!("{path}.network"), profile.network),
                    username: n.contact(&format!("{path}.username"), profile.username),
                    url: n.contact(&format!("{path}.url"), profile.url),
                }
            }),
            summary: self.rich("basics.summary", &basics.summary),
        }
    }

    fn work(&mut self, path: &str, work: source::Work) -> Work {
        Work {
            id: self.id(path, work.id),
            position: self.text(&format!("{path}.position"), work.position),
            organization: self.text(&format!("{path}.name"), work.name),
            location: self.whereabouts(path, work.place, work.remote),
            dates: self.date_range(path, &work.start_date, work.end_date.as_deref()),
            summary: work
                .summary
                .map(|s| self.rich(&format!("{path}.summary"), &s)),
            highlights: work
                .highlights
                .iter()
                .enumerate()
                .map(|(i, h)| self.rich(&format!("{path}.highlights[{i}]"), h))
                .collect(),
            skills: self.skill_refs(path, work.skills),
            accent: self.accent(path, work.color),
        }
    }

    fn project(&mut self, path: &str, project: source::Project) -> Project {
        Project {
            id: self.id(path, project.id),
            kind: project.kind.map(|k| self.text(&format!("{path}.type"), k)),
            entity: project
                .entity
                .map(|e| self.text(&format!("{path}.entity"), e)),
            title: self.text(&format!("{path}.name"), project.name),
            description: project
                .description
                .map(|d| self.rich(&format!("{path}.description"), &d)),
            location: self.whereabouts(path, project.place, project.remote),
            dates: self.date_range(path, &project.start_date, project.end_date.as_deref()),
            skills: self.skill_refs(path, project.skills),
            accent: self.accent(path, project.color),
        }
    }

    fn skill(&mut self, path: &str, skill: source::Skill) -> SkillGroup {
        SkillGroup {
            id: self.id(path, skill.id),
            name: self.text(&format!("{path}.name"), skill.name),
            keywords: skill
                .keywords
                .into_iter()
                .enumerate()
                .map(|(i, k)| self.text(&format!("{path}.keywords[{i}]"), k))
                .collect(),
        }
    }

    fn education(&mut self, path: &str, education: source::Education) -> Education {
        Education {
            id: self.id(path, education.id),
            study_type: education
                .study_type
                .map(|s| self.text(&format!("{path}.studyType"), s)),
            area: education
                .area
                .map(|a| self.text(&format!("{path}.area"), a)),
            institution: self.text(&format!("{path}.institution"), education.institution),
            location: self.whereabouts(path, education.place, education.remote),
            dates: self.date_range(path, &education.start_date, education.end_date.as_deref()),
            courses: education
                .courses
                .into_iter()
                .enumerate()
                .map(|(i, c)| self.text(&format!("{path}.courses[{i}]"), c))
                .collect(),
            accent: self.accent(path, education.color),
        }
    }

    /// An `x-places` entry, resolved to its display string ("City, Country").
    fn place(&mut self, id: &str, place: source::Place) -> String {
        let path = format!("x-places.{id}");
        self.kebab_case(&path, id);
        let city = self.text(&format!("{path}.city"), place.city);
        let country = self.country(&format!("{path}.countryCode"), &place.country_code);
        join_present([Some(city.as_str()), country.as_deref()], ", ").unwrap_or_default()
    }

    /// Country name for an ISO code from `COUNTRIES`.
    fn country(&mut self, path: &str, code: &str) -> Option<String> {
        let name = COUNTRIES
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, name)| name.to_string());
        if name.is_none() {
            self.error(
                path,
                format!("unknown country code `{code}` (add it to COUNTRIES in resume-model)"),
            );
        }
        name
    }

    /// Resolves `x-place` / `x-remote` into the displayed location, e.g.
    /// "Vienna, Austria", "Scharnstein, Austria (Remote)" or "Remote (Worldwide)".
    fn whereabouts(
        &mut self,
        path: &str,
        place: Option<String>,
        remote: Option<source::Remote>,
    ) -> Option<String> {
        let place = place.and_then(|id| {
            let resolved = self.places.get(&id).cloned();
            if resolved.is_none() {
                let known: Vec<&str> = self.places.keys().map(String::as_str).collect();
                let message = format!(
                    "unknown place `{id}` (defined in x-places: {})",
                    known.join(", ")
                );
                self.error(&format!("{path}.x-place"), message);
            }
            resolved
        });
        let scope = match remote {
            None | Some(source::Remote::Flag(false)) => return place,
            Some(source::Remote::Flag(true)) => None,
            Some(source::Remote::Scope(scope)) => {
                Some(self.text(&format!("{path}.x-remote"), scope))
            }
        };
        Some(match (place, scope) {
            (Some(place), None) => format!("{place} (Remote)"),
            (Some(place), Some(scope)) => format!("{place} (Remote, {scope})"),
            (None, None) => "Remote".to_owned(),
            (None, Some(scope)) => format!("Remote ({scope})"),
        })
    }

    /// `x-skills`: exact keywords from `skills[].keywords`, each listed once.
    fn skill_refs(&mut self, path: &str, skills: Vec<String>) -> Vec<String> {
        let mut seen = HashSet::new();
        skills
            .into_iter()
            .enumerate()
            .map(|(i, skill)| {
                let path = format!("{path}.x-skills[{i}]");
                let skill = self.text(&path, skill);
                if !self.keywords.contains(&skill) {
                    let known: Vec<&str> = self.keywords.iter().map(String::as_str).collect();
                    let message = format!(
                        "`{skill}` is not a keyword in skills[] (known: {})",
                        known.join(", ")
                    );
                    self.error(&path, message);
                } else if !seen.insert(skill.clone()) {
                    self.error(&path, format!("`{skill}` is listed twice"));
                }
                skill
            })
            .collect()
    }

    /// `x-color`: `#RRGGBB`.
    fn accent(&mut self, path: &str, color: Option<String>) -> Option<[u8; 3]> {
        let color = color?;
        let rgb = color
            .strip_prefix('#')
            .filter(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(|hex| [0, 2, 4].map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0)));
        if rgb.is_none() {
            self.error(
                &format!("{path}.x-color"),
                format!("`{color}` is not a color like #005B96"),
            );
        }
        rgb
    }

    /// Stable ids: kebab-case, unique across the whole resume.
    fn id(&mut self, path: &str, id: String) -> String {
        let path = format!("{path}.x-id");
        if self.kebab_case(&path, &id) && !self.ids.insert(id.clone()) {
            self.error(&path, format!("duplicate id `{id}`"));
        }
        id
    }

    fn kebab_case(&mut self, path: &str, id: &str) -> bool {
        let valid = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid {
            self.error(path, format!("`{id}` must be kebab-case ([a-z0-9-]+)"));
        }
        valid
    }

    /// Plain (non-Markdown) text: trimmed, `--`/`---` turned into dashes.
    fn text(&mut self, path: &str, text: String) -> String {
        let text = smart_dashes(text.trim());
        if text.is_empty() {
            self.error(path, "must not be empty");
        }
        text
    }

    /// Contact details are passed verbatim to LaTeX, so no dash conversion and
    /// no LaTeX special characters.
    fn contact(&mut self, path: &str, text: String) -> String {
        let text = text.trim().to_owned();
        if let Some(c) = text.chars().find(|c| CONTACT_FORBIDDEN.contains(c)) {
            self.error(
                path,
                format!("character `{c}` is not allowed in contact details"),
            );
        }
        if text.is_empty() {
            self.error(path, "must not be empty");
        }
        text
    }

    /// Inline Markdown (single paragraph) with smart punctuation.
    fn rich(&mut self, path: &str, markdown: &str) -> RichText {
        let mut spans: Vec<Span> = Vec::new();
        let mut style = Style::default();
        let mut link: Option<String> = None;
        let mut paragraphs = 0;

        // Merges with the previous span when the formatting is the same.
        fn push(spans: &mut Vec<Span>, text: &str, style: Style, link: &Option<String>) {
            match spans.last_mut() {
                Some(last) if last.style == style && last.link == *link => last.text.push_str(text),
                _ => spans.push(Span {
                    text: text.to_owned(),
                    style,
                    link: link.clone(),
                }),
            }
        }

        for event in Parser::new_ext(markdown, Options::ENABLE_SMART_PUNCTUATION) {
            match event {
                Event::Start(Tag::Paragraph) => {
                    paragraphs += 1;
                    if paragraphs == 2 {
                        self.error(path, "only a single paragraph is allowed");
                    }
                }
                Event::Start(Tag::Emphasis) => style.italic = true,
                Event::End(TagEnd::Emphasis) => style.italic = false,
                Event::Start(Tag::Strong) => style.bold = true,
                Event::End(TagEnd::Strong) => style.bold = false,
                Event::Start(Tag::Link { dest_url, .. }) => link = Some(dest_url.to_string()),
                Event::End(TagEnd::Link) => link = None,
                Event::Text(text) => push(&mut spans, &text, style, &link),
                Event::Code(code) => push(&mut spans, &code, Style { code: true, ..style }, &link),
                Event::SoftBreak | Event::HardBreak => push(&mut spans, " ", style, &link),
                Event::End(_) => {}
                Event::Start(tag) => self.error(
                    path,
                    format!("only inline Markdown is allowed, found {tag:?} (e.g. a leading `-` or `1.` starts a list)"),
                ),
                other => self.error(path, format!("unsupported Markdown: {other:?}")),
            }
        }
        if spans.is_empty() {
            self.error(path, "must not be empty");
        }
        RichText(spans)
    }

    fn date_range(&mut self, path: &str, start: &str, end: Option<&str>) -> DateRange {
        let start = self.date(&format!("{path}.startDate"), start);
        let end = end.map(|end| self.date(&format!("{path}.endDate"), end));
        // Only flag ranges that are wrong under every possible reading of a
        // missing month: e.g. startDate 2020-06 / endDate 2020 ("ended
        // sometime in 2020") is fine, since end's latest possible point
        // (2020-12) isn't before start's earliest (2020-06). A plain `<`
        // would wrongly reject that, since `None < Some(_)` always.
        if let Some(end) = end
            && end.latest() < start.earliest()
        {
            self.error(path, format!("endDate {end} is before startDate {start}"));
        }
        DateRange { start, end }
    }

    fn date(&mut self, path: &str, text: &str) -> PartialDate {
        match parse_date(text) {
            Some(date) => date,
            None => {
                self.error(
                    path,
                    format!("`{text}` is not a date like 2020, 2020-11 or 2020-11-03"),
                );
                PartialDate {
                    year: 0,
                    month: None,
                }
            }
        }
    }
}

/// The dash part of pulldown-cmark's smart punctuation, for plain fields.
/// Plain fields (titles, names, keywords) are deliberately not Markdown-parsed:
/// text like `C#`, `C++` or `SIMD (SSE/AVX/Neon)` must stay literal.
fn smart_dashes(text: &str) -> String {
    text.replace("---", "—").replace("--", "–")
}

/// `YYYY`, `YYYY-MM` or `YYYY-MM-DD` (the day is validated, then dropped).
fn parse_date(text: &str) -> Option<PartialDate> {
    fn number(digits: &str, len: usize) -> Option<u16> {
        (digits.len() == len && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| digits.parse().ok())
            .flatten()
    }
    let parts: Vec<&str> = text.split('-').collect();
    if parts.len() > 3 {
        return None;
    }
    let year = number(parts[0], 4)?;
    let month = match parts.get(1) {
        Some(month) => Some(number(month, 2).filter(|m| (1..=12).contains(m))? as u8),
        None => None,
    };
    if let Some(day) = parts.get(2) {
        number(day, 2).filter(|d| (1..=31).contains(d))?;
    }
    Some(PartialDate { year, month })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    const MINIMAL: &str = r#"
basics:
  name: Jane Doe
  label: Engineer
  email: jane@example.com
  location: { city: Vienna, countryCode: AT }
  summary: Builds things.
"#;

    fn with_work(work: &str) -> String {
        format!("{MINIMAL}work:\n{work}")
    }

    fn errors(yaml: &str) -> Vec<String> {
        match load_str(yaml) {
            Err(LoadError::Invalid(errors)) => errors,
            other => panic!("expected validation errors, got {other:?}"),
        }
    }

    #[test]
    fn loads_the_real_resume() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/resume.yaml");
        let resume = load_file(path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            resume.basics.location.to_string(),
            "Scharnstein, Upper Austria, Austria"
        );
        assert_eq!(resume.work[0].dates.to_string(), "11/2020 – Present");
        assert!(resume.skill_group("practices").is_some());
        let locations: Vec<Option<&str>> =
            resume.work.iter().map(|w| w.location.as_deref()).collect();
        assert_eq!(locations[0], Some("Scharnstein, Austria (Remote)"));
        assert_eq!(locations[1], Some("Remote (Worldwide)"));
        assert_eq!(locations[2], Some("Vienna, Austria"));
        assert_eq!(
            resume.projects[1].location.as_deref(),
            Some("Barcelona, Spain")
        );
        assert!(resume.work[0].skills.iter().any(|s| s == "Rust"));
    }

    #[test]
    fn inline_markdown_becomes_spans() {
        let resume = load_str(&MINIMAL.replace(
            "Builds things.",
            r#"Builds **fast** "things" -- see [site](https://example.com)&nbsp;now"#,
        ))
        .unwrap();
        let bold = Style {
            bold: true,
            ..Style::default()
        };
        assert_eq!(
            resume.basics.summary.spans(),
            [
                Span::plain("Builds "),
                Span {
                    text: "fast".into(),
                    style: bold,
                    link: None
                },
                Span::plain(" “things” – see "),
                Span {
                    text: "site".into(),
                    style: Style::default(),
                    link: Some("https://example.com".into())
                },
                Span::plain("\u{a0}now"),
            ]
        );
    }

    #[test]
    fn plain_fields_get_smart_dashes() {
        let resume = load_str(&with_work(
            "  - { x-id: a, position: Dev -- Web, name: Acme, startDate: '2020' }",
        ))
        .unwrap();
        assert_eq!(resume.work[0].position, "Dev – Web");
    }

    #[test]
    fn rejects_block_markdown() {
        let errors = errors(&with_work(
            "  - x-id: a\n    position: Dev\n    name: Acme\n    startDate: '2020'\n    highlights: ['- a list']",
        ));
        assert!(
            errors[0].starts_with("work[0].highlights[0]: only inline Markdown"),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_bad_dates_and_ids() {
        let errors = errors(&with_work(concat!(
            "  - { x-id: a, position: Dev, name: Acme, startDate: '2020-13' }\n",
            "  - { x-id: a, position: Dev, name: Acme, startDate: '2020', endDate: '2019' }\n",
            "  - { x-id: Bad_Id, position: Dev, name: Acme, startDate: '2020' }\n",
        )));
        assert_eq!(
            errors,
            [
                "work[0].startDate: `2020-13` is not a date like 2020, 2020-11 or 2020-11-03",
                "work[1].x-id: duplicate id `a`",
                "work[1]: endDate 2019 is before startDate 2020",
                "work[2].x-id: `Bad_Id` must be kebab-case ([a-z0-9-]+)",
            ]
        );
    }

    #[test]
    fn accepts_ranges_where_a_missing_month_makes_the_order_ambiguous() {
        // "started 2020-06, ended sometime in 2020" is a normal way to write
        // an entry -- not an inverted range, even though a naive comparison
        // of (year, month) tuples would treat the year-only end as earlier.
        let resume = load_str(&with_work(
            "  - { x-id: a, position: Dev, name: Acme, startDate: '2020-06', endDate: '2020' }",
        ))
        .unwrap();
        assert_eq!(resume.work[0].dates.end.unwrap().to_string(), "2020");
    }

    #[test]
    fn rejects_ranges_that_are_invalid_regardless_of_precision() {
        // endDate 2019 (year-only) is before startDate 2020-06 under *every*
        // possible month, unlike the ambiguous case above.
        let errors = errors(&with_work(
            "  - { x-id: a, position: Dev, name: Acme, startDate: '2020-06', endDate: '2019' }",
        ));
        assert_eq!(
            errors,
            ["work[0]: endDate 2019 is before startDate 06/2020"]
        );
    }

    const PLACES_AND_SKILLS: &str = "\
x-places:
  vienna: { city: Vienna, countryCode: AT }
skills:
  - { x-id: code, name: Code, keywords: [Rust, C#] }
";

    fn with_places_and_work(work: &str) -> String {
        format!("{MINIMAL}{PLACES_AND_SKILLS}work:\n{work}")
    }

    #[test]
    fn resolves_places_and_remote() {
        let resume = load_str(&with_places_and_work(concat!(
            "  - { x-id: a, position: Dev, name: A, startDate: '2020', x-place: vienna }\n",
            "  - { x-id: b, position: Dev, name: B, startDate: '2020', x-place: vienna, x-remote: true }\n",
            "  - { x-id: c, position: Dev, name: C, startDate: '2020', x-place: vienna, x-remote: EU }\n",
            "  - { x-id: d, position: Dev, name: D, startDate: '2020', x-remote: true }\n",
            "  - { x-id: e, position: Dev, name: E, startDate: '2020', x-remote: Worldwide }\n",
            "  - { x-id: f, position: Dev, name: F, startDate: '2020', x-place: vienna, x-remote: false }\n",
            "  - { x-id: g, position: Dev, name: G, startDate: '2020' }\n",
        )))
        .unwrap_or_else(|e| panic!("{e}"));
        let locations: Vec<Option<&str>> =
            resume.work.iter().map(|w| w.location.as_deref()).collect();
        assert_eq!(
            locations,
            [
                Some("Vienna, Austria"),
                Some("Vienna, Austria (Remote)"),
                Some("Vienna, Austria (Remote, EU)"),
                Some("Remote"),
                Some("Remote (Worldwide)"),
                Some("Vienna, Austria"),
                None,
            ]
        );
    }

    #[test]
    fn rejects_unknown_places_and_bad_place_definitions() {
        let yaml = format!(
            "{MINIMAL}x-places:\n  Bad_Id: {{ city: X, countryCode: XX }}\nwork:\n{}",
            "  - { x-id: a, position: Dev, name: A, startDate: '2020', x-place: graz }"
        );
        assert_eq!(
            errors(&yaml),
            [
                "x-places.Bad_Id: `Bad_Id` must be kebab-case ([a-z0-9-]+)",
                "x-places.Bad_Id.countryCode: unknown country code `XX` (add it to COUNTRIES in resume-model)",
                "work[0].x-place: unknown place `graz` (defined in x-places: Bad_Id)",
            ]
        );
    }

    #[test]
    fn validates_skill_references() {
        let resume = load_str(&with_places_and_work(
            "  - { x-id: a, position: Dev, name: A, startDate: '2020', x-skills: [Rust, 'C#'] }",
        ))
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resume.work[0].skills, ["Rust", "C#"]);

        let errors = errors(&with_places_and_work(
            "  - { x-id: a, position: Dev, name: A, startDate: '2020', x-skills: [Rust, rust, Rust] }",
        ));
        assert_eq!(
            errors,
            [
                "work[0].x-skills[1]: `rust` is not a keyword in skills[] (known: C#, Rust)",
                "work[0].x-skills[2]: `Rust` is listed twice",
            ]
        );
    }

    #[test]
    fn parses_accent_colors() {
        let resume = load_str(&with_places_and_work(
            "  - { x-id: a, position: Dev, name: A, startDate: '2020', x-color: '#005B96' }",
        ))
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resume.work[0].accent, Some([0x00, 0x5B, 0x96]));

        for bad in ["005B96", "#05B96", "#GG5B96"] {
            let errors = errors(&with_places_and_work(&format!(
                "  - {{ x-id: a, position: Dev, name: A, startDate: '2020', x-color: '{bad}' }}"
            )));
            assert_eq!(
                errors,
                [format!(
                    "work[0].x-color: `{bad}` is not a color like #005B96"
                )]
            );
        }
    }

    #[test]
    fn rejects_unknown_fields_with_location() {
        let error = load_str(&MINIMAL.replace("label:", "lable:")).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("lable"), "{message}");
    }

    #[test]
    fn parses_dates() {
        assert_eq!(
            parse_date("2020"),
            Some(PartialDate {
                year: 2020,
                month: None
            })
        );
        assert_eq!(
            parse_date("2020-11"),
            Some(PartialDate {
                year: 2020,
                month: Some(11)
            })
        );
        assert_eq!(
            parse_date("2020-11-03"),
            Some(PartialDate {
                year: 2020,
                month: Some(11)
            })
        );
        for bad in [
            "20",
            "2020-1",
            "2020-00",
            "2020-11-32",
            "2020-11-03-01",
            "abcd",
        ] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
    }
}
