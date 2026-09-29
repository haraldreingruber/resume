//! README.md and the plain HTML page, rendered with minijinja templates.
//!
//! Templates get the [`Resume`] as context plus filters for formatting:
//! `md` / `html` (plain strings or rich text), `dates`, `heading` (project),
//! `title` and `courses` (education), `location`, `language`, and
//! `labeled_profiles` (basics -> `[{label, url}]`).

use minijinja::value::ViaDeserialize;
use minijinja::{AutoEscape, Environment, Value, context};
use resume_model::{Basics, DateRange, Education, Language, Location, Project, Resume, RichText};
use serde::{Deserialize, Serialize};

use resume_model::about::{ABOUT, SHADERS_URL};
use resume_model::site::{PDF_FILE, SITE_URL};

pub fn readme(resume: &Resume) -> anyhow::Result<String> {
    render("README.md.j2", resume)
}

pub fn plain_html(resume: &Resume) -> anyhow::Result<String> {
    render("plain.html.j2", resume)
}

fn render(name: &str, resume: &Resume) -> anyhow::Result<String> {
    let env = environment();
    let template = env.get_template(name)?;
    let ctx = context! {
        resume => Value::from_serialize(resume),
        site_url => SITE_URL,
        pdf_file => PDF_FILE,
        about => Value::from_serialize(&ABOUT),
        about_sections => Value::from_serialize(about_sections()),
    };
    let mut out = template.render(ctx)?;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// The About sections with their text split into spans, the technique
/// names linking to their shaders.
fn about_sections() -> Vec<AboutSection> {
    ABOUT
        .sections
        .iter()
        .map(|section| AboutSection {
            heading: section.heading,
            spans: section
                .spans()
                .into_iter()
                .map(|span| AboutSpan {
                    text: span.text,
                    url: span.shader.map(|file| format!("{SHADERS_URL}{file}")),
                })
                .collect(),
        })
        .collect()
}

#[derive(Serialize)]
struct AboutSection {
    heading: &'static str,
    spans: Vec<AboutSpan>,
}

#[derive(Serialize)]
struct AboutSpan {
    text: &'static str,
    url: Option<String>,
}

fn environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
    env.set_auto_escape_callback(|name| {
        if name.contains(".html") {
            AutoEscape::Html
        } else {
            AutoEscape::None
        }
    });
    env.add_template("README.md.j2", include_str!("../templates/README.md.j2"))
        .expect("valid README template");
    env.add_template("plain.html.j2", include_str!("../templates/plain.html.j2"))
        .expect("valid HTML template");

    env.add_filter("md", |text: ViaDeserialize<Text>| match text.0 {
        Text::Plain(text) => markdown_escape(&text),
        Text::Rich(text) => markdown(&text),
    });
    env.add_filter("html", |text: ViaDeserialize<Text>| {
        Value::from_safe_string(match text.0 {
            Text::Plain(text) => html_escape(&text),
            Text::Rich(text) => html(&text),
        })
    });
    env.add_filter("dates", |dates: ViaDeserialize<DateRange>| {
        dates.to_string()
    });
    env.add_filter("heading", |project: ViaDeserialize<Project>| {
        project.heading()
    });
    env.add_filter("title", |education: ViaDeserialize<Education>| {
        education.title()
    });
    env.add_filter("courses", |education: ViaDeserialize<Education>| {
        education.courses_sentence()
    });
    env.add_filter("location", |location: ViaDeserialize<Location>| {
        location.to_string()
    });
    env.add_filter("language", |language: ViaDeserialize<Language>| {
        language.to_string()
    });
    env.add_filter("labeled_profiles", |basics: ViaDeserialize<Basics>| {
        let links: Vec<ProfileLink> = basics
            .labeled_profiles()
            .into_iter()
            .map(|(label, profile)| ProfileLink {
                label,
                url: profile.url.clone(),
            })
            .collect();
        Value::from_serialize(links)
    });
    env
}

/// A profile link with a label that disambiguates repeated networks.
#[derive(Serialize)]
struct ProfileLink {
    label: String,
    url: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Text {
    Plain(String),
    Rich(RichText),
}

fn markdown(text: &RichText) -> String {
    crate::spans::render(
        text,
        markdown_escape,
        |_escaped, raw| format!("`{raw}`"),
        |s| format!("*{s}*"),
        |s| format!("**{s}**"),
        |s, url| format!("[{s}]({url})"),
    )
}

/// Escapes characters with inline meaning in (GitHub-flavored) Markdown,
/// including `|` for table cells.
fn markdown_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '>' | '|') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn html(text: &RichText) -> String {
    crate::spans::render(
        text,
        html_escape,
        |escaped, _raw| format!("<code>{escaped}</code>"),
        |s| format!("<em>{s}</em>"),
        |s| format!("<strong>{s}</strong>"),
        |s, url| format!("<a href=\"{}\">{s}</a>", html_escape(url)),
    )
}

/// Delegates to minijinja's own HTML escaper -- the same one that escapes
/// every un-filtered `{{ }}` in plain.html.j2 via the auto-escape callback
/// above -- so plain and rich-text fields are escaped by exactly one rule
/// instead of two independently hand-maintained copies of it.
fn html_escape(text: &str) -> String {
    minijinja::HtmlEscape(text).to_string()
}

#[cfg(test)]
mod tests {
    use resume_model::{Span, Style};

    use super::*;

    fn sample() -> RichText {
        RichText(vec![
            Span::plain("a *b* <c> "),
            Span {
                text: "d".into(),
                style: Style {
                    italic: true,
                    ..Style::default()
                },
                link: Some("https://x.org/?a=1&b=2".into()),
            },
        ])
    }

    #[test]
    fn renders_markdown() {
        assert_eq!(
            markdown(&sample()),
            "a \\*b\\* \\<c\\> [*d*](https://x.org/?a=1&b=2)"
        );
    }

    #[test]
    fn renders_html() {
        assert_eq!(
            html(&sample()),
            "a *b* &lt;c&gt; <a href=\"https:&#x2f;&#x2f;x.org&#x2f;?a=1&amp;b=2\"><em>d</em></a>"
        );
    }

    fn code_sample() -> RichText {
        RichText(vec![Span {
            text: "a*b*<c>".into(),
            style: Style {
                code: true,
                ..Style::default()
            },
            link: None,
        }])
    }

    #[test]
    fn renders_markdown_code_span_verbatim() {
        // Markdown code spans are verbatim: no escaping inside backticks.
        assert_eq!(markdown(&code_sample()), "`a*b*<c>`");
    }

    #[test]
    fn renders_html_code_span_escaped() {
        assert_eq!(html(&code_sample()), "<code>a*b*&lt;c&gt;</code>");
    }
}
