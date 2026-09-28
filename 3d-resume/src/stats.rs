//! The performance overlay (P, or the About panel's toggle): frame rate,
//! frame, CPU and GPU times, whether the app is animating or idle, what it
//! draws, its memory, and the display.

use std::collections::VecDeque;
use std::time::Duration;

use web_time::Instant;

/// How often the overlay's numbers refresh; at rest, the app redraws this
/// often only for the overlay.
pub const REFRESH: Duration = Duration::from_millis(500);
/// The frame rate and averages cover this long.
const WINDOW: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
struct Frame {
    at: Instant,
    cpu_ms: f32,
    wait_ms: f32,
    /// Drawn because the frame before it animated: the time since that one
    /// is a frame time. (After an idle pause it's just the pause.)
    continued: bool,
}

/// GPU time of a frame's passes (timestamp queries), in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuTimes {
    pub draw_ms: f32,
    /// The particle simulation, when it ran that frame.
    pub particles_ms: Option<f32>,
}

/// The recent frames, and when the overlay's text last refreshed.
#[derive(Default)]
pub struct Stats {
    frames: VecDeque<Frame>,
    gpu: Option<GpuTimes>,
    refreshed: Option<Instant>,
}

/// What the overlay shows about frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub fps: usize,
    /// Average and longest time between frames while animating.
    pub frame_ms: Option<(f32, f32)>,
    /// Average CPU time per frame (updating, encoding, submitting).
    pub cpu_ms: Option<f32>,
    /// Average time per frame spent waiting for the display: getting the
    /// next surface texture and presenting it (vsync, or a throttled window).
    pub wait_ms: Option<f32>,
    pub gpu: Option<GpuTimes>,
}

impl Stats {
    /// A presented frame, the CPU time it took, how long it waited for the
    /// display, and whether it `continued` an animation (the frame before
    /// asked for it) rather than following an idle pause.
    pub fn frame(&mut self, at: Instant, cpu_ms: f32, wait_ms: f32, continued: bool) {
        self.frames.push_back(Frame {
            at,
            cpu_ms,
            wait_ms,
            continued,
        });
        while self
            .frames
            .front()
            .is_some_and(|f| at.saturating_duration_since(f.at) > WINDOW)
        {
            self.frames.pop_front();
        }
    }

    /// GPU times of a frame, as they come back (a few frames late).
    pub fn gpu(&mut self, times: GpuTimes) {
        self.gpu = Some(times);
    }

    /// Whether the overlay's text is due for a refresh; if so, it counts as
    /// refreshed now.
    pub fn refresh_due(&mut self, now: Instant) -> bool {
        let due = self.refreshed.is_none_or(|at| now >= at + REFRESH);
        if due {
            self.refreshed = Some(now);
        }
        due
    }

    /// When the next refresh is due (the app wakes up for it at rest).
    pub fn next_refresh(&self, now: Instant) -> Instant {
        self.refreshed.map_or(now, |at| at + REFRESH)
    }

    pub fn summary(&self, now: Instant) -> Summary {
        let recent: Vec<&Frame> = self
            .frames
            .iter()
            .filter(|f| now.saturating_duration_since(f.at) <= WINDOW)
            .collect();
        let intervals: Vec<f32> = recent
            .windows(2)
            .filter(|pair| pair[1].continued)
            .map(|pair| pair[1].at.saturating_duration_since(pair[0].at))
            .map(|gap| gap.as_secs_f32() * 1000.0)
            .collect();
        let average = |values: &[f32]| {
            (!values.is_empty()).then(|| values.iter().sum::<f32>() / values.len() as f32)
        };
        let cpu: Vec<f32> = recent.iter().map(|f| f.cpu_ms).collect();
        let wait: Vec<f32> = recent.iter().map(|f| f.wait_ms).collect();
        Summary {
            fps: recent.len(),
            frame_ms: average(&intervals)
                .map(|avg| (avg, intervals.iter().copied().fold(0.0, f32::max))),
            cpu_ms: average(&cpu),
            wait_ms: average(&wait),
            gpu: self.gpu,
        }
    }
}

/// Instances drawn per frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DrawCounts {
    pub glyphs: u32,
    pub shapes: u32,
    pub lines: u32,
    pub particles: u32,
}

/// Everything else the overlay shows.
pub struct Info<'a> {
    /// Whether something moves (the timeline, the particles), so the app
    /// draws continuously; otherwise it only redraws for the overlay.
    pub animating: bool,
    /// Whether this GPU has timestamp queries.
    pub gpu_timing: bool,
    pub counts: DrawCounts,
    /// The app's GPU buffers and textures, and the window's swap chain.
    pub gpu_bytes: u64,
    pub surface_bytes: u64,
    pub size: [u32; 2],
    pub session: &'a str,
}

/// The overlay's text, one entry per line, its heading first.
pub fn lines(summary: &Summary, info: &Info) -> Vec<String> {
    let mut lines = vec!["Performance".to_owned()];
    let mut frames = format!("{} fps", summary.fps);
    if let Some((average, max)) = summary.frame_ms {
        frames.push_str(&format!(" · frame {average:.1} ms (max {max:.1})"));
    }
    lines.push(frames);
    let cpu = summary
        .cpu_ms
        .map_or_else(|| "CPU –".to_owned(), |ms| format!("CPU {ms:.2} ms"));
    let wait = summary.wait_ms.map_or_else(
        || "display wait –".to_owned(),
        |ms| format!("display wait {ms:.1} ms"),
    );
    lines.push(format!("{cpu} · {wait}"));
    let gpu = match (info.gpu_timing, summary.gpu) {
        (false, _) => "GPU time not available here".to_owned(),
        (true, None) => "GPU –".to_owned(),
        (true, Some(times)) => match times.particles_ms {
            Some(particles) => format!(
                "GPU {:.2} ms draw + {particles:.2} ms particles",
                times.draw_ms
            ),
            None => format!("GPU {:.2} ms draw", times.draw_ms),
        },
    };
    lines.push(gpu);
    lines.push(if info.animating {
        "Rendering: animating".to_owned()
    } else {
        format!(
            "Rendering: on demand, idle (redrawn {}×/s for this overlay)",
            (1000 / REFRESH.as_millis()).max(1)
        )
    });
    let c = info.counts;
    lines.push(format!(
        "Drawn: {} glyphs · {} shapes · {} lines · {} particles",
        thousands(c.glyphs.into()),
        thousands(c.shapes.into()),
        thousands(c.lines.into()),
        thousands(c.particles.into())
    ));
    let mut memory = "Memory: ".to_owned();
    if let Some((label, bytes)) = process_memory() {
        memory.push_str(&format!("{} {label} · ", megabytes(bytes)));
    }
    memory.push_str(&format!(
        "GPU {} + {} swap chain",
        megabytes(info.gpu_bytes),
        megabytes(info.surface_bytes)
    ));
    lines.push(memory);
    let [width, height] = info.size;
    lines.push(format!("{width}×{height} px · {}", info.session));
    lines
}

/// The app's memory: resident memory as the OS counts it (native), or the
/// WebAssembly heap (web; the browser doesn't tell the rest).
#[cfg(not(target_arch = "wasm32"))]
fn process_memory() -> Option<(&'static str, u64)> {
    memory_stats::memory_stats().map(|m| ("process", m.physical_mem as u64))
}

#[cfg(target_arch = "wasm32")]
fn process_memory() -> Option<(&'static str, u64)> {
    let pages = core::arch::wasm32::memory_size(0) as u64;
    Some(("wasm heap", pages * 65536))
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// `16384` → `16,384`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn counts_frames_in_the_last_second_and_ignores_idle_gaps() {
        let t0 = Instant::now();
        let mut stats = Stats::default();
        // An animation at 60 fps, an idle pause, then another short one
        // with a stutter.
        for i in 0..30 {
            stats.frame(t0 + ms(i * 16), 1.0, 14.0, i > 0);
        }
        for (i, at) in [800, 816, 832, 900, 916].into_iter().enumerate() {
            stats.frame(t0 + ms(at), 3.0, 16.0, i > 0);
        }
        let summary = stats.summary(t0 + ms(950));
        assert_eq!(summary.fps, 35);
        let (average, max) = summary.frame_ms.expect("intervals");
        assert!((16.0..=19.0).contains(&average), "{average}");
        // The stutter counts; the idle pause between the animations doesn't.
        assert_eq!(max, 68.0);
        assert_eq!(summary.wait_ms, Some((30.0 * 14.0 + 5.0 * 16.0) / 35.0));
        // Frames older than a second drop out.
        assert_eq!(stats.summary(t0 + ms(1_500)).fps, 5);
        assert_eq!(stats.summary(t0 + ms(3_000)).fps, 0);
    }

    #[test]
    fn refreshes_twice_a_second() {
        let t0 = Instant::now();
        let mut stats = Stats::default();
        assert!(stats.refresh_due(t0));
        assert!(!stats.refresh_due(t0 + ms(100)));
        assert_eq!(stats.next_refresh(t0 + ms(100)), t0 + REFRESH);
        assert!(stats.refresh_due(t0 + REFRESH));
    }

    #[test]
    fn formats_the_overlay() {
        let summary = Summary {
            fps: 60,
            frame_ms: Some((16.7, 18.2)),
            cpu_ms: Some(0.84),
            wait_ms: Some(15.2),
            gpu: Some(GpuTimes {
                draw_ms: 0.42,
                particles_ms: Some(0.1),
            }),
        };
        let info = Info {
            animating: false,
            gpu_timing: true,
            counts: DrawCounts {
                glyphs: 12_345,
                shapes: 890,
                lines: 1_234,
                particles: 16_384,
            },
            gpu_bytes: 6 * 1024 * 1024,
            surface_bytes: 24 * 1024 * 1024,
            size: [2560, 1600],
            session: "Vulkan on NVIDIA GeForce GTX 960M",
        };
        let lines = lines(&summary, &info);
        assert_eq!(lines[0], "Performance");
        assert_eq!(lines[1], "60 fps · frame 16.7 ms (max 18.2)");
        assert_eq!(lines[2], "CPU 0.84 ms · display wait 15.2 ms");
        assert_eq!(lines[3], "GPU 0.42 ms draw + 0.10 ms particles");
        assert!(lines[4].starts_with("Rendering: on demand, idle"));
        assert_eq!(
            lines[5],
            "Drawn: 12,345 glyphs · 890 shapes · 1,234 lines · 16,384 particles"
        );
        assert!(lines[6].ends_with("GPU 6.0 MB + 24.0 MB swap chain"));
        assert_eq!(lines[7], "2560×1600 px · Vulkan on NVIDIA GeForce GTX 960M");
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
