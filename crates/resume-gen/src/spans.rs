//! Shared by `latex::rich`, `templates::markdown` and `templates::html`:
//! walking a `RichText`'s spans and wrapping each one for code/italic/bold/
//! link, in that fixed order. Escaping and the wrap syntax differ per format
//! and are supplied as closures, so adding a new [`Style`](resume_model::Style)
//! flag only needs to change this one walk instead of three copies of it.

use resume_model::RichText;

/// `code` receives both the escaped and the raw text: Markdown's code span is
/// verbatim (no escaping), while LaTeX's `\texttt` and HTML's `<code>` still
/// escape their content.
pub fn render(
    text: &RichText,
    escape: impl Fn(&str) -> String,
    code: impl Fn(&str, &str) -> String,
    italic: impl Fn(String) -> String,
    bold: impl Fn(String) -> String,
    link: impl Fn(String, &str) -> String,
) -> String {
    text.spans()
        .iter()
        .map(|span| {
            let escaped = escape(&span.text);
            let mut out = if span.style.code {
                code(&escaped, &span.text)
            } else {
                escaped
            };
            if span.style.italic {
                out = italic(out);
            }
            if span.style.bold {
                out = bold(out);
            }
            if let Some(url) = &span.link {
                out = link(out, url);
            }
            out
        })
        .collect()
}
