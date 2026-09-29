//! Diagnostics and developer tools: the performance overlay's numbers, GPU
//! timestamp queries, and headless screenshots.

pub mod gpu_timer;
#[cfg(not(target_arch = "wasm32"))]
pub mod screenshot;
pub mod stats;
