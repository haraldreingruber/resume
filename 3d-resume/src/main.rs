//! WebGPU 3D resume: one crate for the browser (wasm + WebGPU) and the
//! native desktop app (DX12 / Vulkan / Metal via wgpu).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod content;
mod focus;
mod gpu;
mod links;
mod renderer;
mod scene;
#[cfg(not(target_arch = "wasm32"))]
mod screenshot;
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

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(dir) = arg("--screenshots") {
        if let Err(error) = screenshots(dir) {
            log::error!("screenshots failed: {error}");
            std::process::exit(1);
        }
        return;
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
    app::Options {
        station: arg("--station"),
        reduced_motion: std::env::args().any(|a| a == "--reduced-motion"),
    }
}

/// `--screenshots <dir> [--station <id|index>] [--size 1280x800] [--focus <n>]`:
/// renders the stations (or one) headlessly to PNG files and exits.
#[cfg(not(target_arch = "wasm32"))]
fn screenshots(dir: String) -> Result<(), String> {
    let size = match arg("--size") {
        Some(size) => screenshot::parse_size(&size)
            .ok_or_else(|| format!("--size `{size}`: expected WIDTHxHEIGHT, e.g. 1280x800"))?,
        None => screenshot::DEFAULT_SIZE,
    };
    let focus = arg("--focus")
        .map(|n| {
            n.parse()
                .map_err(|_| format!("--focus `{n}`: expected a number"))
        })
        .transpose()?;
    screenshot::run(&screenshot::Request {
        dir: dir.into(),
        station: arg("--station"),
        size,
        focus,
    })
}

/// The value after `name` on the command line, e.g. `dedalus` in
/// `--station dedalus`.
#[cfg(not(target_arch = "wasm32"))]
fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip_while(|a| a != name);
    args.next()?;
    args.next()
}
