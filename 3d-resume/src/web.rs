//! Browser glue: the page's canvas, the plain-HTML fallback, the address
//! bar and keyboard focus leaving the canvas.

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{HtmlCanvasElement, KeyboardEvent};
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;

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

/// Whether the page URL has a query parameter, with or without a value
/// (`?stats`).
pub fn has_query_param(name: &str) -> bool {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .is_some_and(|search| {
            search
                .trim_start_matches('?')
                .split('&')
                .any(|pair| pair.split('=').next() == Some(name))
        })
}

/// Shows the station in view in the address bar (`?station=<id>`, or no
/// parameter for the intro), keeping other parameters. Replaces the current
/// history entry, so Back still leaves the page.
pub fn show_station(id: Option<&str>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(url) = window
        .location()
        .href()
        .ok()
        .and_then(|href| web_sys::Url::new(&href).ok())
    else {
        return;
    };
    let params = url.search_params();
    match id {
        Some(id) => params.set("station", id),
        None => params.delete("station"),
    }
    if let Ok(history) = window.history() {
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&url.href()));
    }
}

/// Lets Tab move keyboard focus out of the canvas. winit prevents the
/// default action of every key pressed on the canvas, which would trap focus
/// there. This capture-phase listener runs first and stops Tab presses that
/// should leave (`leaves`: [Tab, Shift+Tab], kept up to date by the app)
/// from reaching winit, so the browser moves focus as usual.
pub fn release_tab_at_edges(leaves: Rc<Cell<[bool; 2]>>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let listener = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
        let on_canvas = event
            .target()
            .is_some_and(|target| target.has_type::<HtmlCanvasElement>());
        if on_canvas && event.key() == "Tab" && leaves.get()[usize::from(event.shift_key())] {
            event.stop_propagation();
        }
    });
    let _ = window.add_event_listener_with_callback_and_bool(
        "keydown",
        listener.as_ref().unchecked_ref(),
        true,
    );
    // Lives as long as the page.
    listener.forget();
}

/// Hands the page nav's clicks to the app:
/// - the Skills link (`#skills-link`) flies to the skill map in place,
///   instead of reloading the page at `?station=skills` (its `href`, the
///   fallback);
/// - the About button (`#about-button`) opens or closes the About panel.
///
/// Both hand keyboard focus back to the canvas, where Tab reaches the
/// panel's link and Esc closes it.
pub fn forward_nav_clicks(proxy: EventLoopProxy<AppEvent>) {
    forward_clicks("skills-link", proxy.clone(), || AppEvent::ToggleSkills);
    forward_clicks("about-button", proxy, || AppEvent::ToggleAbout);
}

fn forward_clicks(id: &str, proxy: EventLoopProxy<AppEvent>, event: fn() -> AppEvent) {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
    else {
        return;
    };
    let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |click: web_sys::Event| {
        click.prevent_default();
        let _ = proxy.send_event(event());
        if let Some(canvas) = canvas() {
            let _ = canvas.focus();
        }
    });
    let _ = element.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
    // Lives as long as the page.
    listener.forget();
}

/// How far (physical pixels) the page's nav reaches up from the bottom of
/// the window, so the About panel can sit above it.
pub fn nav_height() -> f32 {
    let Some(window) = web_sys::window() else {
        return 0.0;
    };
    let top = window
        .document()
        .and_then(|document| document.query_selector("nav").ok().flatten())
        .map(|nav| nav.get_bounding_client_rect().top());
    let height = window.inner_height().ok().and_then(|h| h.as_f64());
    match (top, height) {
        (Some(top), Some(height)) => ((height - top) * window.device_pixel_ratio()).max(0.0) as f32,
        _ => 0.0,
    }
}

/// Keeps the About button's `aria-expanded` in step with the panel.
pub fn show_about_expanded(open: bool) {
    if let Some(button) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("about-button"))
    {
        let _ = button.set_attribute("aria-expanded", if open { "true" } else { "false" });
    }
}

/// Enters or leaves fullscreen for the whole page, so the HTML nav stays
/// visible. Browsers only allow it during a user gesture (click, key press);
/// they leave fullscreen on Esc themselves.
pub fn toggle_fullscreen() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if document.fullscreen_element().is_some() {
        document.exit_fullscreen();
    } else if let Some(page) = document.document_element()
        && let Err(error) = page.request_fullscreen()
    {
        // E.g. iPhone Safari, which has no fullscreen for pages.
        log::warn!("fullscreen not available: {error:?}");
    }
}

/// Speaks `text` to screen readers through the page's polite live region
/// (`#live`). The same text again is still announced.
pub fn announce(text: &str) {
    let Some(live) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("live"))
    else {
        return;
    };
    // Unchanged content isn't announced again: vary it invisibly.
    let text = if live.text_content().as_deref() == Some(text) {
        format!("{text}\u{a0}")
    } else {
        text.to_owned()
    };
    live.set_text_content(Some(&text));
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

/// Whether the main pointer is a finger (phones, tablets): hints then talk
/// about taps instead of keys.
pub fn coarse_pointer() -> bool {
    media_matches("(pointer: coarse)")
}

/// Whether the user asked the OS/browser to minimize animations.
pub fn prefers_reduced_motion() -> bool {
    media_matches("(prefers-reduced-motion: reduce)")
}

fn media_matches(query: &str) -> bool {
    web_sys::window()
        .and_then(|window| window.match_media(query).ok().flatten())
        .is_some_and(|list| list.matches())
}
