//! Headless screenshots (`--screenshots <dir>`): renders stations offscreen
//! to PNG files, without a window or event loop, so it also runs on machines
//! without a display, e.g. CI with Mesa's software Vulkan driver
//! (`WGPU_ADAPTER_NAME=llvmpipe`). Each image shows a station at rest, as the
//! app shows it, including the native buttons, plus two frames of the
//! particle intro and one with the About panel open. The particles run in fixed frame steps, so the images are
//! deterministic.

use std::path::{Path, PathBuf};

use crate::about;
use crate::focus;
use crate::gpu::Context;
use crate::intro::{self, Intro};
use crate::renderer::Renderer;
use crate::scene::{Metrics, Scene};
use crate::stats::{self, Stats};
use crate::ui::{self, Insets, Panel, UiLayer};

/// The native window's default (logical) size.
pub const DEFAULT_SIZE: [u32; 2] = [1280, 800];

/// An image counts as rendered if at least this fraction of its pixels is
/// bright (text); catches frames that come out blank.
const MIN_BRIGHT: f64 = 0.001;
/// Simulated frame length (seconds) for the particles.
const FRAME: f32 = 1.0 / 60.0;
/// When the default set shows the particles forming the name.
const ASSEMBLING_AT: f32 = 1.5;
/// Where the default set shows them scattering.
const SCATTERING_AT: f32 = 0.3;

pub struct Request {
    pub dir: PathBuf,
    /// One station (id or index) instead of all of them.
    pub station: Option<String>,
    /// One timeline position instead, e.g. 0.3 (between intro and station 1).
    pub position: Option<f32>,
    /// Stops the particle assembly at this time (seconds) instead of letting
    /// the particles come to rest; implies the intro if nothing else is set.
    pub time: Option<f32>,
    pub size: [u32; 2],
    /// Device pixels per logical pixel, e.g. 3 on a phone (sizes the buttons).
    pub scale: f32,
    /// Gives keyboard focus to the n-th focus target (shows its focus ring).
    pub focus: Option<usize>,
    /// Shows the About panel.
    pub about: bool,
    /// Shows the performance overlay (with the headless GPU's timings).
    pub stats: bool,
}

/// One image: where on the timeline, until when the particles run (`None`:
/// until they come to rest), which focus target has keyboard focus, and
/// whether the About panel is open.
struct Frame {
    name: String,
    position: f32,
    until: Option<f32>,
    focus: Option<usize>,
    about: bool,
}

/// The images to take, in simulation order.
fn frames(scene: &Scene, request: &Request) -> Result<Vec<Frame>, String> {
    let station = |i: usize| Frame {
        name: format!("{i:02}-{}", scene.station_id(i).unwrap_or("station")),
        position: i as f32,
        until: request.time,
        focus: request.focus,
        about: request.about,
    };
    if let Some(position) = request.position {
        return Ok(vec![Frame {
            name: format!("position-{position}"),
            position,
            until: request.time,
            focus: request.focus,
            about: request.about,
        }]);
    }
    if let Some(key) = &request.station {
        let index = scene
            .station_index(key)
            .ok_or_else(|| format!("unknown station `{key}`"))?;
        return Ok(vec![station(index)]);
    }
    if request.time.is_some() {
        return Ok(vec![station(0)]);
    }
    let mut frames = vec![
        Frame {
            name: "00-intro-assembling".to_owned(),
            position: 0.0,
            until: Some(ASSEMBLING_AT),
            focus: None,
            about: false,
        },
        station(0),
        Frame {
            name: "00-about".to_owned(),
            about: true,
            ..station(0)
        },
        Frame {
            name: "00-intro-scattering".to_owned(),
            position: SCATTERING_AT,
            until: None,
            focus: None,
            about: false,
        },
    ];
    frames.extend((1..scene.station_count()).map(station));
    // The skill map with its first entry focused: connections highlighted.
    let skills = scene.skills_station();
    let at = frames
        .iter()
        .position(|f| f.position == skills as f32)
        .map_or(frames.len(), |i| i + 1);
    frames.insert(
        at,
        Frame {
            name: format!("{skills:02}-skills-related"),
            focus: Some(0),
            ..station(skills)
        },
    );
    Ok(frames)
}

/// Runs the particles in fixed frame steps until `until` or until they rest.
fn simulate(
    renderer: &mut Renderer,
    ctx: &Context,
    intro: &mut Intro,
    position: f32,
    until: Option<f32>,
) {
    let scatter = intro::scatter(position);
    while until.is_none_or(|t| intro.time() < t) {
        match intro.advance(FRAME, scatter, None) {
            Some(step) => renderer.step_particles(ctx, &step),
            None => break,
        }
    }
    renderer.set_title_opacity(ctx, intro.title_opacity());
}

/// Writes one PNG per frame into `request.dir`.
pub fn run(request: &Request) -> Result<(), String> {
    let [width, height] = request.size;
    let aspect = width as f32 / height as f32;
    let resume = crate::content::resume();
    // The layout the app would pick for this screen shape.
    let mut scene = Scene::with_metrics(&resume, Metrics::for_aspect(aspect));
    let buttons = ui::native_buttons(&mut scene);
    let source = ui::source_link(&mut scene);
    let overlay_switch = ui::overlay_switch(&mut scene);
    let frames = frames(&scene, request)?;

    let ctx = pollster::block_on(context())?;
    let mut renderer = Renderer::new(&ctx, &scene, ctx.compute);
    let mut intro = Intro::new(false);
    let session = about::session(&ctx.adapter);
    let gpu_timing = renderer.set_timing(&ctx, request.stats);
    let mut stats = Stats::default();
    let lens = scene.lens(aspect);
    let projection = UiLayer::projection(width as f32, height as f32);
    let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screenshot"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: ctx.view_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    std::fs::create_dir_all(&request.dir).map_err(|e| format!("{}: {e}", request.dir.display()))?;

    for frame in frames {
        simulate(&mut renderer, &ctx, &mut intro, frame.position, frame.until);
        let station = frame.position.round() as usize;
        let camera = scene.camera(frame.position, &lens);
        if request.stats {
            // A frame to measure first, so the overlay has numbers.
            let start = web_time::Instant::now();
            renderer.draw(&ctx, &view, &camera, projection);
            let cpu = start.elapsed();
            // Waits for the GPU, so its timestamps are ready.
            read_back(&ctx, &target)?;
            stats.frame(
                web_time::Instant::now(),
                cpu.as_secs_f32() * 1000.0,
                0.0,
                false,
            );
            if let Some(times) = renderer.gpu_times(&ctx) {
                stats.gpu(times);
            }
        }
        let overlay = request.stats.then(|| {
            let info = stats::Info {
                animating: false,
                gpu_timing,
                counts: renderer.counts(),
                gpu_bytes: renderer.gpu_bytes(),
                surface_bytes: u64::from(width) * u64::from(height) * 4,
                size: [width, height],
                session: &session,
            };
            stats::lines(&stats.summary(web_time::Instant::now()), &info)
        });
        let panel = frame.about.then(|| Panel {
            session: &session,
            source,
            overlay: overlay_switch,
            overlay_shown: request.stats,
        });
        let size = [width as f32, height as f32];
        let ui = UiLayer::new(
            size,
            request.scale,
            Insets::default(),
            &buttons,
            panel.as_ref(),
            overlay.as_deref(),
        );
        renderer.set_ui(&ctx, &ui);
        let links = [source, overlay_switch];
        let panel_links = if frame.about { &links[..] } else { &[] };
        let targets = focus::targets(&scene, station, panel_links, &buttons);
        let focused = frame.focus.and_then(|n| targets.get(n).copied());
        renderer.set_groups(&ctx, None, focused, scene.relations(focused));
        renderer.draw(&ctx, &view, &camera, projection);
        let rgba = read_back(&ctx, &target)?;
        let path = request.dir.join(format!("{}.png", frame.name));
        // Written even if blank, so the failure can be inspected.
        write_png(&path, request.size, &rgba)?;
        check_rendered(&rgba).map_err(|e| format!("{}: {e}", path.display()))?;
        log::info!("wrote {}", path.display());
    }
    Ok(())
}

/// Parses `--size`, e.g. `1280x800`.
pub fn parse_size(text: &str) -> Option<[u32; 2]> {
    let (width, height) = text.split_once('x')?;
    let size = [width.parse().ok()?, height.parse().ok()?];
    size.iter()
        .all(|&n| (1..=8192).contains(&n))
        .then_some(size)
}

async fn context() -> Result<Context, String> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
    let adapter = wgpu::util::initialize_adapter_from_env_or_default(&instance, None)
        .await
        .map_err(|e| format!("no GPU adapter: {e}"))?;
    Context::new(&adapter, "screenshots", wgpu::TextureFormat::Rgba8UnormSrgb).await
}

/// Copies the texture to the CPU: tightly packed RGBA rows (sRGB-encoded).
fn read_back(ctx: &Context, texture: &wgpu::Texture) -> Result<Vec<u8>, String> {
    let size = texture.size();
    let row = size.width * 4;
    let padded_row = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot readback"),
        size: u64::from(padded_row * size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: None,
            },
        },
        size,
    );
    ctx.queue.submit([encoder.finish()]);

    let (sender, receiver) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        let _ = sender.send(result);
    });
    ctx.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| e.to_string())?;
    receiver
        .recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let mapped = buffer.get_mapped_range(..).map_err(|e| e.to_string())?;
    Ok(mapped
        .chunks(padded_row as usize)
        .flat_map(|padded| &padded[..row as usize])
        .copied()
        .collect())
}

/// Writes an opaque RGB PNG (the frame's alpha is always 1).
fn write_png(path: &Path, [width, height]: [u32; 2], rgba: &[u8]) -> Result<(), String> {
    let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::create(path).map_err(|e| error(&e))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let rgb: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|[r, g, b, _]| [*r, *g, *b])
        .collect();
    let mut writer = encoder.write_header().map_err(|e| error(&e))?;
    writer.write_image_data(&rgb).map_err(|e| error(&e))?;
    writer.finish().map_err(|e| error(&e))
}

/// Fails for a (nearly) blank image: too few bright (text) pixels.
fn check_rendered(rgba: &[u8]) -> Result<(), String> {
    let pixels = rgba.as_chunks::<4>().0;
    let total = pixels.len();
    let bright = pixels
        .iter()
        .filter(|[r, g, b, _]| [r, g, b].iter().any(|&&c| c > 150))
        .count();
    if (bright as f64) < total as f64 * MIN_BRIGHT {
        return Err(format!(
            "looks blank: only {bright} of {total} pixels are bright"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("1280x800"), Some([1280, 800]));
        assert_eq!(parse_size("0x800"), None);
        assert_eq!(parse_size("1280"), None);
        assert_eq!(parse_size("wide x tall"), None);
    }

    #[test]
    fn rejects_blank_images() {
        let dark = [8u8, 16, 24, 255].repeat(1000);
        assert!(check_rendered(&dark).is_err());
        let mut text = dark.clone();
        text[..40].copy_from_slice(&[240; 40]);
        assert!(check_rendered(&text).is_ok());
    }
}
