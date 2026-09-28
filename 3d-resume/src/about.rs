//! The About panel's live parts: which graphics backend and GPU this session
//! renders with, and what screen readers hear when the panel opens. The
//! panel's text comes from `resume_model::about` (shared with plain.html);
//! `ui` lays it out.

use resume_model::about::ABOUT;

/// The backend and GPU, e.g. "Vulkan on NVIDIA GeForce GTX 960M". Browsers
/// often keep the GPU to themselves: then just "WebGPU in this browser".
pub fn session(info: &wgpu::AdapterInfo) -> String {
    let backend = match info.backend {
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Metal => "Metal",
        wgpu::Backend::Dx12 => "DirectX 12",
        wgpu::Backend::Gl if cfg!(target_os = "android") => "OpenGL ES",
        wgpu::Backend::Gl => "OpenGL",
        wgpu::Backend::BrowserWebGpu => "WebGPU in this browser",
        wgpu::Backend::Noop => "no GPU",
    };
    let mut session = match info.name.trim() {
        "" => backend.to_owned(),
        name => format!("{backend} on {name}"),
    };
    if info.device_type == wgpu::DeviceType::Cpu {
        session.push_str(" (software rendering)");
    }
    session
}

/// The panel as one announcement for screen readers.
pub fn announcement(session: &str) -> String {
    let mut text = format!("{}.", ABOUT.title);
    for section in ABOUT.sections {
        text.push_str(&format!(" {}: {}", section.heading, section.text));
    }
    text.push_str(&format!(
        " This session: {session}. Tab reaches the link: {}. Escape closes \
         this panel.",
        ABOUT.source_label
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(backend: wgpu::Backend, name: &str, device_type: wgpu::DeviceType) -> String {
        session(&wgpu::AdapterInfo {
            name: name.to_owned(),
            ..wgpu::AdapterInfo::new(device_type, backend)
        })
    }

    #[test]
    fn names_the_backend_and_the_gpu() {
        use wgpu::{Backend, DeviceType};
        assert_eq!(
            adapter(
                Backend::Vulkan,
                "NVIDIA GeForce GTX 960M",
                DeviceType::DiscreteGpu
            ),
            "Vulkan on NVIDIA GeForce GTX 960M"
        );
        assert_eq!(
            adapter(Backend::Dx12, "", DeviceType::IntegratedGpu),
            "DirectX 12"
        );
        assert_eq!(
            adapter(Backend::BrowserWebGpu, "", DeviceType::Other),
            "WebGPU in this browser"
        );
        assert_eq!(
            adapter(
                Backend::Vulkan,
                "llvmpipe (LLVM 20.1.2, 256 bits)",
                DeviceType::Cpu
            ),
            "Vulkan on llvmpipe (LLVM 20.1.2, 256 bits) (software rendering)"
        );
    }

    #[test]
    fn the_announcement_reads_the_whole_panel() {
        let text = announcement("Metal on Apple M2");
        for section in ABOUT.sections {
            assert!(text.contains(section.heading) && text.contains(section.text));
        }
        assert!(text.contains("This session: Metal on Apple M2."));
    }
}
