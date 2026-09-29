//! Startup timing: when each phase from launch to the first frame finished
//! (building the scene, the window, the GPU, the renderer's atlas, pipelines
//! and particles). Each mark is logged; headless runs print them as a table
//! for CI's job summary.

use std::sync::Mutex;

use web_time::Instant;

/// When the app started, and the marks so far (phase, milliseconds since).
type Marks = (Instant, Vec<(&'static str, f64)>);

static MARKS: Mutex<Option<Marks>> = Mutex::new(None);

/// Starts the clock (the app's first moment); later calls keep it.
pub fn start() {
    let mut marks = MARKS.lock().unwrap_or_else(|e| e.into_inner());
    marks.get_or_insert_with(|| (Instant::now(), Vec::new()));
}

/// Records that `phase` just finished, and logs it.
pub fn mark(phase: &'static str) {
    let mut marks = MARKS.lock().unwrap_or_else(|e| e.into_inner());
    let (started, list) = marks.get_or_insert_with(|| (Instant::now(), Vec::new()));
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    log::info!("startup: {phase} after {ms:.0} ms");
    list.push((phase, ms));
}

/// The marks as a Markdown table: each phase, when it finished and how long
/// it took (headless screenshots write it for CI).
#[cfg(not(target_arch = "wasm32"))]
pub fn table() -> String {
    let marks = MARKS.lock().unwrap_or_else(|e| e.into_inner());
    let list = marks.as_ref().map_or(&[][..], |(_, list)| list.as_slice());
    let mut out = "| Phase | Done after | Took |\n| --- | ---: | ---: |\n".to_owned();
    let mut previous = 0.0;
    for &(phase, ms) in list {
        out.push_str(&format!(
            "| {phase} | {ms:.0} ms | {:.0} ms |\n",
            ms - previous
        ));
        previous = ms;
    }
    out
}
