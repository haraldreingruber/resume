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

/// Opens a link: `mailto:` in place (the mail client takes over), anything
/// else in a new tab, falling back to same-tab navigation if a popup blocker
/// refuses the new tab.
pub fn open_url(url: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let opened = !url.starts_with("mailto:")
        && window
            .open_with_url_and_target(url, "_blank")
            .map_err(|e| format!("{e:?}"))?
            .is_some();
    if !opened {
        window
            .location()
            .set_href(url)
            .map_err(|e| format!("{e:?}"))?;
    }
    Ok(())
}

/// Whether the user asked the OS/browser to minimize animations.
pub fn prefers_reduced_motion() -> bool {
    web_sys::window()
        .and_then(|window| {
            window
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
        })
        .is_some_and(|query| query.matches())
}
