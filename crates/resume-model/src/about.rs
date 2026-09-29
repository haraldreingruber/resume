//! "How this resume is built": the technology behind the 3D resume. Shown
//! by the 3D app's About panel (the ⓘ button or the I key, on every
//! platform) and by the matching section at the end of the plain HTML page.
//! Technique names link to the shaders that implement them.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct About {
    pub title: &'static str,
    pub sections: &'static [Section],
    pub source_label: &'static str,
    pub source_url: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Section {
    pub heading: &'static str,
    pub text: &'static str,
    /// Phrases of `text` that link to a shader, in the order they appear.
    pub shaders: &'static [ShaderLink],
}

/// A phrase that links to the shader implementing it.
#[derive(Debug, Serialize)]
pub struct ShaderLink {
    pub phrase: &'static str,
    /// File name in `3d-resume/shaders/`.
    pub file: &'static str,
}

/// A piece of a section's text, linking to a shader or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub text: &'static str,
    pub shader: Option<&'static str>,
}

/// Where the shaders are published, with a trailing slash.
pub const SHADERS_URL: &str =
    "https://github.com/haraldreingruber/resume/blob/main/3d-resume/shaders/";

pub const ABOUT: About = About {
    title: "How this resume is built",
    sections: &[
        Section {
            heading: "Platforms & backends",
            text: "One Rust codebase, drawn with wgpu: WebAssembly on WebGPU in the browser; \
                   DirectX 12, Metal or Vulkan in the Windows, macOS and Linux apps; Vulkan \
                   or OpenGL ES on Android, and Metal on iOS.",
            shaders: &[],
        },
        Section {
            heading: "Rendering techniques",
            text: "Text comes from multi-channel signed distance fields (MSDF) baked at \
                   build time, so it stays sharp at any distance. Cards, chips and dots are \
                   instanced SDF shapes, and the skill map's edges instanced line segments. \
                   A compute shader moves the intro's 16,384 particles, and an HDR bloom \
                   pass lets bright text and particles glow. Frames are drawn only while \
                   something moves: at rest, the GPU is idle.",
            shaders: &[
                ShaderLink {
                    phrase: "multi-channel signed distance fields (MSDF)",
                    file: "text.wgsl",
                },
                ShaderLink {
                    phrase: "instanced SDF shapes",
                    file: "shapes.wgsl",
                },
                ShaderLink {
                    phrase: "instanced line segments",
                    file: "lines.wgsl",
                },
                ShaderLink {
                    phrase: "compute shader",
                    file: "particles_sim.wgsl",
                },
                ShaderLink {
                    phrase: "HDR bloom pass",
                    file: "bloom.wgsl",
                },
            ],
        },
    ],
    source_label: "Source code on GitHub",
    source_url: "https://github.com/haraldreingruber/resume",
};

impl About {
    /// Every string it displays (for the 3D app's glyph atlas).
    pub fn texts(&self) -> impl Iterator<Item = &'static str> {
        [self.title, self.source_label].into_iter().chain(
            self.sections
                .iter()
                .flat_map(|section| [section.heading, section.text]),
        )
    }

    /// Every shader link, in reading order.
    pub fn shaders(&self) -> impl Iterator<Item = &'static ShaderLink> {
        self.sections.iter().flat_map(|section| section.shaders)
    }
}

impl Section {
    /// Its text split into plain pieces and the phrases that link to a
    /// shader.
    pub fn spans(&self) -> Vec<Span> {
        let mut spans = Vec::new();
        let mut rest = self.text;
        for link in self.shaders {
            let Some(at) = rest.find(link.phrase) else {
                continue;
            };
            if at > 0 {
                spans.push(Span {
                    text: &rest[..at],
                    shader: None,
                });
            }
            spans.push(Span {
                text: link.phrase,
                shader: Some(link.file),
            });
            rest = &rest[at + link.phrase.len()..];
        }
        if !rest.is_empty() {
            spans.push(Span {
                text: rest,
                shader: None,
            });
        }
        spans
    }
}

impl ShaderLink {
    pub fn url(&self) -> String {
        format!("{SHADERS_URL}{}", self.file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shader_phrase_is_found_in_order() {
        for section in ABOUT.sections {
            let spans = section.spans();
            let text: String = spans.iter().map(|span| span.text).collect();
            assert_eq!(text, section.text);
            let linked: Vec<&str> = spans.iter().filter_map(|span| span.shader).collect();
            let files: Vec<&str> = section.shaders.iter().map(|link| link.file).collect();
            assert_eq!(linked, files, "{}: a phrase is missing", section.heading);
        }
    }

    #[test]
    fn links_point_into_the_shaders_folder() {
        let link = &ABOUT.sections[1].shaders[0];
        assert_eq!(
            link.url(),
            "https://github.com/haraldreingruber/resume/blob/main/3d-resume/shaders/text.wgsl"
        );
    }
}
