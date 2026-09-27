//! WebGPU 3D resume: one crate for the browser (wasm + WebGPU) and the
//! native desktop app (DX12 / Vulkan / Metal via wgpu).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod content;
mod gpu;
mod links;
mod renderer;
mod scene;
mod shapes;
mod text;
mod timeline;
mod ui;
#[cfg(target_arch = "wasm32")]
mod web;

use winit::event_loop::EventLoop;

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_hal=warn,wgpu_core=warn"),
    )
    .init();
    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
        let _ = console_log::init_with_level(log::Level::Info);
    }

    let event_loop = EventLoop::<app::AppEvent>::with_user_event()
        .build()
        .expect("create event loop");
    let app = app::App::new(&event_loop, options());

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        event_loop.run_app(&mut app).expect("event loop");
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(app);
    }
}

/// Deep link and reduced motion from the page URL / media query.
#[cfg(target_arch = "wasm32")]
fn options() -> app::Options {
    app::Options {
        station: web::query_param("station"),
        reduced_motion: web::prefers_reduced_motion(),
    }
}

/// `--station <id|index>` and `--reduced-motion`.
#[cfg(not(target_arch = "wasm32"))]
fn options() -> app::Options {
    let args: Vec<String> = std::env::args().collect();
    app::Options {
        station: args
            .iter()
            .position(|a| a == "--station")
            .and_then(|i| args.get(i + 1).cloned()),
        reduced_motion: args.iter().any(|a| a == "--reduced-motion"),
    }
}
