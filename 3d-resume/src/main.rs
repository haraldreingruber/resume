//! WebGPU 3D resume: one crate for the browser (wasm + WebGPU) and the
//! native desktop app (DX12 / Vulkan / Metal via wgpu).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod content;
mod gpu;
mod renderer;
mod scene;
mod text;
mod timeline;
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
    let app = app::App::new(&event_loop, start_station());

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

/// Deep link into the timeline: `?station=N` on the web, `--station N` natively.
fn start_station() -> usize {
    #[cfg(target_arch = "wasm32")]
    let value = web::query_param("station");
    #[cfg(not(target_arch = "wasm32"))]
    let value = {
        let args: Vec<String> = std::env::args().collect();
        args.iter()
            .position(|a| a == "--station")
            .and_then(|i| args.get(i + 1).cloned())
    };
    value.and_then(|v| v.parse().ok()).unwrap_or(0)
}
