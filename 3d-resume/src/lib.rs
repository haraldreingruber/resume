//! WebGPU 3D resume: one crate for the browser (wasm + WebGPU), the desktop
//! app (DX12 / Vulkan / Metal via wgpu) and the phone builds (Android via
//! `android_main`, iOS through the regular program).

mod about;
#[cfg(target_os = "android")]
mod android;
mod app;
mod content;
mod focus;
mod gpu;
#[cfg(not(target_arch = "wasm32"))]
mod icon;
mod intro;
mod lines;
mod links;
mod particles;
mod renderer;
mod scene;
#[cfg(not(target_arch = "wasm32"))]
mod screenshot;
mod shapes;
mod skillmap;
mod text;
mod timeline;
mod ui;
#[cfg(target_arch = "wasm32")]
mod web;

use winit::event_loop::EventLoop;

/// Starts the app (desktop, iOS or web); see `android_main` for Android.
pub fn run() {
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
    start(event_loop, options());
}

/// Android's entry point, called by `NativeActivity` (via android-activity)
/// on its own thread.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("resume-3d"),
    );
    let event_loop = EventLoop::<app::AppEvent>::with_user_event()
        .with_android_app(android)
        .build()
        .expect("create event loop");
    start(
        event_loop,
        app::Options {
            station: None,
            reduced_motion: false,
        },
    );
}

fn start(event_loop: EventLoop<app::AppEvent>, options: app::Options) {
    let app = app::App::new(&event_loop, options);
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

/// `--screenshots <dir> [--station <id|index> | --position <t>] [--time <s>]
/// [--size 1280x800] [--scale <n>] [--focus <n>] [--about]`: renders the
/// stations, two frames of the particle intro and the About panel (or one
/// frame) headlessly to PNG files and exits.
#[cfg(not(target_arch = "wasm32"))]
fn screenshots(dir: String) -> Result<(), String> {
    let size = match arg("--size") {
        Some(size) => screenshot::parse_size(&size)
            .ok_or_else(|| format!("--size `{size}`: expected WIDTHxHEIGHT, e.g. 1280x800"))?,
        None => screenshot::DEFAULT_SIZE,
    };
    screenshot::run(&screenshot::Request {
        dir: dir.into(),
        station: arg("--station"),
        position: number("--position")?,
        time: number("--time")?,
        size,
        scale: number("--scale")?.unwrap_or(1.0),
        focus: number("--focus")?,
        about: std::env::args().any(|a| a == "--about"),
    })
}

/// The number after `name` on the command line, if given.
#[cfg(not(target_arch = "wasm32"))]
fn number<T: std::str::FromStr>(name: &str) -> Result<Option<T>, String> {
    arg(name)
        .map(|n| {
            n.parse()
                .map_err(|_| format!("{name} `{n}`: expected a number"))
        })
        .transpose()
}

/// The value after `name` on the command line, e.g. `dedalus` in
/// `--station dedalus`.
#[cfg(not(target_arch = "wasm32"))]
fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip_while(|a| a != name);
    args.next()?;
    args.next()
}
