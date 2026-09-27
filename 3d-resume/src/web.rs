//! Browser glue: the page's canvas and the plain-HTML fallback.

use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

/// The `<canvas id="app">` from `web/index.html`.
pub fn canvas() -> Option<HtmlCanvasElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id("app")?
        .dyn_into::<HtmlCanvasElement>()
        .ok()
}

/// Shown when WebGPU exists but can't be initialized (e.g. no adapter).
pub fn show_plain_version() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().replace("plain.html?no-webgpu");
    }
}

/// A query parameter of the page URL, e.g. `station` in `?station=2`.
pub fn query_param(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    search
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}
