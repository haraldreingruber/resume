//! Everything that talks to the GPU (wgpu): the device and surface, the
//! frame's passes and pipelines, and the instances they draw (MSDF text,
//! SDF shapes, lines, particles), plus bloom.

pub mod bloom;
pub mod gpu;
pub mod lines;
pub mod particles;
pub mod renderer;
pub mod shapes;
pub mod text;
