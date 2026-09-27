//! Where the published resume lives (GitHub Pages, deployed by CI). Shared by
//! the generators (README, plain HTML) and the 3D app's links.

/// Base URL of the published site, with a trailing slash.
pub const SITE_URL: &str = "https://haraldreingruber.github.io/resume/";

/// File name of the PDF, next to the 3D page on the site and next to the
/// native app in release archives.
pub const PDF_FILE: &str = "Harald_Reingruber_resume.pdf";

/// Absolute URL of the PDF on the site.
pub fn pdf_url() -> String {
    format!("{SITE_URL}{PDF_FILE}")
}
