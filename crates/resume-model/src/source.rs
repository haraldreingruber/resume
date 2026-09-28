//! Authoring types: the shape of `content/resume.yaml`.
//!
//! A subset of the JSON Resume schema (<https://jsonresume.org/schema>) plus
//! `x-` extension fields. Unknown fields are rejected to catch typos. Doc
//! comments end up in the generated JSON schema, i.e. as editor hover help.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;

/// ISO 8601 date with optional month/day: `2020`, `2020-11` or `2020-11-03`.
const DATE_PATTERN: &str = r"^\d{4}(-\d{2}(-\d{2})?)?$";
/// Accent color, e.g. `#005B96`.
const COLOR_PATTERN: &str = r"^#[0-9A-Fa-f]{6}$";

/// Resume content, following the JSON Resume schema plus `x-` extension fields.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub basics: Basics,
    /// Places referenced by `x-place` in work, projects and education, by id
    /// (kebab-case), e.g. `vienna: { city: Vienna, countryCode: AT }`.
    #[serde(rename = "x-places", default)]
    pub places: BTreeMap<String, Place>,
    /// Work experience, most recent first.
    #[serde(default)]
    pub work: Vec<Work>,
    /// Selected projects and research, most recent first.
    #[serde(default)]
    pub projects: Vec<Project>,
    /// Skill groups; each group's keywords are rendered as tags.
    #[serde(default)]
    pub skills: Vec<Skill>,
    /// Education, most recent first.
    #[serde(default)]
    pub education: Vec<Education>,
    /// Spoken languages.
    #[serde(default)]
    pub languages: Vec<Language>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Basics {
    pub name: String,
    /// Tagline shown below the name.
    pub label: String,
    pub email: String,
    pub location: Location,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Short professional summary (inline Markdown).
    pub summary: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Location {
    pub city: String,
    pub region: Option<String>,
    /// ISO 3166-1 alpha-2 code, e.g. `AT`.
    #[schemars(regex(pattern = r"^[A-Z]{2}$"))]
    pub country_code: Option<String>,
}

/// A place entries can refer to; rendered as "City, Country".
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Place {
    pub city: String,
    /// ISO 3166-1 alpha-2 code, e.g. `AT`.
    #[schemars(regex(pattern = r"^[A-Z]{2}$"))]
    pub country_code: String,
}

/// Remote work: `true` (appends "(Remote)" to the place, or just "Remote"),
/// or a scope such as `Worldwide` ("Remote (Worldwide)").
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Remote {
    Flag(bool),
    Scope(String),
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// E.g. `GitHub`, `LinkedIn`.
    pub network: String,
    pub username: String,
    pub url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Work {
    /// Stable id, unique across the whole resume. The 3D resume's deep link
    /// to this station (`?station=<id>`) -- renaming it breaks saved links.
    #[serde(rename = "x-id")]
    pub id: String,
    /// Job title.
    pub position: String,
    /// Company name.
    pub name: String,
    /// Short name for tight spaces, e.g. the 3D skill map. Defaults to the
    /// company name.
    #[serde(rename = "x-short")]
    pub short: Option<String>,
    /// Id of an entry in `x-places`.
    #[serde(rename = "x-place")]
    pub place: Option<String>,
    #[serde(rename = "x-remote")]
    pub remote: Option<Remote>,
    #[schemars(regex(pattern = DATE_PATTERN))]
    pub start_date: String,
    /// Omit for the current position.
    #[schemars(regex(pattern = DATE_PATTERN))]
    pub end_date: Option<String>,
    /// Condensed one-paragraph version of the highlights (inline Markdown).
    pub summary: Option<String>,
    /// Bullet points (inline Markdown each).
    #[serde(default)]
    pub highlights: Vec<String>,
    /// Skills used, as exact keywords from `skills[].keywords`.
    #[serde(rename = "x-skills", default)]
    pub skills: Vec<String>,
    /// Accent color for this entry in the 3D resume, e.g. `#005B96`.
    #[serde(rename = "x-color")]
    #[schemars(regex(pattern = COLOR_PATTERN))]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Project {
    /// Stable id, unique across the whole resume. The 3D resume's deep link
    /// to this station (`?station=<id>`) -- renaming it breaks saved links.
    #[serde(rename = "x-id")]
    pub id: String,
    /// Kind of project, e.g. `Master's Thesis`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Organization the project was done at.
    pub entity: Option<String>,
    /// Project or thesis title.
    pub name: String,
    /// Short name for tight spaces, e.g. the 3D skill map. Defaults to the
    /// kind of project (or the title).
    #[serde(rename = "x-short")]
    pub short: Option<String>,
    /// What was achieved (inline Markdown).
    pub description: Option<String>,
    /// Id of an entry in `x-places`.
    #[serde(rename = "x-place")]
    pub place: Option<String>,
    #[serde(rename = "x-remote")]
    pub remote: Option<Remote>,
    #[schemars(regex(pattern = DATE_PATTERN))]
    pub start_date: String,
    #[schemars(regex(pattern = DATE_PATTERN))]
    pub end_date: Option<String>,
    /// Skills used, as exact keywords from `skills[].keywords`.
    #[serde(rename = "x-skills", default)]
    pub skills: Vec<String>,
    /// Accent color for this entry in the 3D resume, e.g. `#005B96`.
    #[serde(rename = "x-color")]
    #[schemars(regex(pattern = COLOR_PATTERN))]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    /// Stable id, unique across the whole resume. Referenced from
    /// `latex/main.tex` to place the group.
    #[serde(rename = "x-id")]
    pub id: String,
    /// Group heading, e.g. `Languages`.
    pub name: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Education {
    /// Stable id, unique across the whole resume. The 3D resume's deep link
    /// to this station (`?station=<id>`) -- renaming it breaks saved links.
    #[serde(rename = "x-id")]
    pub id: String,
    /// Degree, e.g. `MSc`.
    pub study_type: Option<String>,
    /// Field of study, e.g. `Computer Science, Visual Computing`.
    pub area: Option<String>,
    pub institution: String,
    /// Id of an entry in `x-places`.
    #[serde(rename = "x-place")]
    pub place: Option<String>,
    #[serde(rename = "x-remote")]
    pub remote: Option<Remote>,
    #[schemars(regex(pattern = DATE_PATTERN))]
    pub start_date: String,
    #[schemars(regex(pattern = DATE_PATTERN))]
    pub end_date: Option<String>,
    /// Focus topics; rendered as one sentence.
    #[serde(default)]
    pub courses: Vec<String>,
    /// Accent color for this entry in the 3D resume, e.g. `#005B96`.
    #[serde(rename = "x-color")]
    #[schemars(regex(pattern = COLOR_PATTERN))]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Language {
    pub language: String,
    /// E.g. `Native`, `Fluent`.
    pub fluency: Option<String>,
}
