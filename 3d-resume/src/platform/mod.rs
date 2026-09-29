//! Per-platform glue: the web page, Android intents, opening links on each
//! platform, the desktop window icon, and screen readers (AccessKit).

#[cfg(accessibility)]
pub mod accessibility;
#[cfg(target_os = "android")]
pub mod android;
#[cfg(not(target_arch = "wasm32"))]
pub mod icon;
pub mod links;
#[cfg(target_arch = "wasm32")]
pub mod web;
