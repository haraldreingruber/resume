//! "How this resume is built": the technology behind the 3D resume. Shown
//! by the 3D app's About panel (the ⓘ button or the I key, on every
//! platform) and by the matching section at the end of the plain HTML page.

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
}

pub const ABOUT: About = About {
    title: "How this resume is built",
    sections: &[
        Section {
            heading: "Platforms & backends",
            text: "One Rust codebase, drawn with wgpu: WebAssembly on WebGPU in the browser; \
                   DirectX 12, Metal or Vulkan in the Windows, macOS and Linux apps; Vulkan \
                   or OpenGL ES on Android, and Metal on iOS.",
        },
        Section {
            heading: "Rendering techniques",
            text: "Text comes from multi-channel signed distance fields (MSDF) baked at \
                   build time, so it stays sharp at any distance. Cards, chips and dots are \
                   instanced SDF shapes, and the skill map's edges instanced line segments. \
                   A compute shader moves the intro's 16,384 particles. Frames are drawn \
                   only while something moves: at rest, the GPU is idle.",
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
}
