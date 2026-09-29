//! GPU time per frame from timestamp queries, for the performance overlay:
//! the scene's render pass, the bloom passes and, when it runs, the particle
//! simulation's compute pass. Each frame's timestamps are copied into one of
//! a few readback buffers and read a frame or two later, so timing never
//! stalls the GPU. Frames that find no free buffer go untimed.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::debug::stats::GpuTimes;
use crate::render::gpu::Context;

/// Readback buffer states.
const FREE: u8 = 0;
const PENDING: u8 = 1;
const READY: u8 = 2;
const SLOTS: usize = 3;

/// What's timed: each has its own begin/end pair of timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Draw = 0,
    Particles = 1,
    Bloom = 2,
}

const PASSES: usize = 3;
/// A begin/end pair of timestamps.
const PAIR_BYTES: u64 = 16;
/// Where each pair is resolved to: query resolves need this alignment.
const RESOLVE_STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;

pub struct GpuTimer {
    queries: [wgpu::QuerySet; PASSES],
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    /// Nanoseconds per timestamp tick.
    period: f32,
    /// The slot this frame is timed into (picked by its first timed pass).
    current: Option<usize>,
    /// Which passes this frame timed.
    timed: [bool; PASSES],
}

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    timed: [bool; PASSES],
}

impl GpuTimer {
    /// `None` where the device has no timestamp queries.
    pub fn new(ctx: &Context) -> Option<Self> {
        let device = &ctx.device;
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let queries = ["draw", "particles", "bloom"].map(|label| {
            device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some(label),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            })
        });
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let resolve = buffer(
            "timestamps",
            RESOLVE_STRIDE * PASSES as u64,
            wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        );
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: buffer(
                    "timestamp readback",
                    PAIR_BYTES * PASSES as u64,
                    wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                ),
                state: Arc::new(AtomicU8::new(FREE)),
                timed: [false; PASSES],
            })
            .collect();
        Some(Self {
            queries,
            resolve,
            slots,
            period: ctx.queue.get_timestamp_period(),
            current: None,
            timed: [false; PASSES],
        })
    }

    /// The slot for this frame: the one picked already, else a free one.
    fn slot(&mut self) -> Option<usize> {
        if self.current.is_none() {
            self.current = self
                .slots
                .iter()
                .position(|slot| slot.state.load(Ordering::Acquire) == FREE);
        }
        self.current
    }

    /// Timestamps for this frame's particle simulation.
    pub fn compute_writes(&mut self) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        self.slot()?;
        self.timed[Pass::Particles as usize] = true;
        Some(wgpu::ComputePassTimestampWrites {
            query_set: &self.queries[Pass::Particles as usize],
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        })
    }

    /// Timestamps for a render pass that `begins` and/or `ends` what's timed
    /// as `pass` (bloom spans several render passes).
    pub fn render_writes(
        &mut self,
        pass: Pass,
        begins: bool,
        ends: bool,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.slot()?;
        self.timed[pass as usize] = true;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.queries[pass as usize],
            beginning_of_pass_write_index: begins.then_some(0),
            end_of_pass_write_index: ends.then_some(1),
        })
    }

    /// After the frame's passes: copies their timestamps to the frame's
    /// readback buffer.
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(index) = self.current else { return };
        let slot = &mut self.slots[index];
        for pass in 0..PASSES {
            // Unwritten queries have no defined value: resolve only those used.
            if self.timed[pass] {
                let offset = RESOLVE_STRIDE * pass as u64;
                encoder.resolve_query_set(&self.queries[pass], 0..2, &self.resolve, offset);
                let to = PAIR_BYTES * pass as u64;
                encoder.copy_buffer_to_buffer(&self.resolve, offset, &slot.buffer, to, PAIR_BYTES);
            }
        }
        slot.timed = self.timed;
    }

    /// After submitting the frame: starts reading its timestamps back.
    pub fn map(&mut self) {
        let Some(index) = self.current.take() else {
            return;
        };
        self.timed = [false; PASSES];
        let slot = &self.slots[index];
        slot.state.store(PENDING, Ordering::Release);
        let state = slot.state.clone();
        slot.buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                state.store(if result.is_ok() { READY } else { FREE }, Ordering::Release);
            });
    }

    /// The newest timed frame that finished, if any.
    pub fn collect(&mut self, ctx: &Context) -> Option<GpuTimes> {
        // Natively, map callbacks run when the device is polled; browsers
        // run them from their event loop.
        #[cfg(not(target_arch = "wasm32"))]
        let _ = ctx.device.poll(wgpu::PollType::Poll);
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
        let mut latest = None;
        for slot in &self.slots {
            if slot.state.load(Ordering::Acquire) != READY {
                continue;
            }
            if let Ok(data) = slot.buffer.get_mapped_range(..) {
                let ticks: Vec<u64> = data
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|&bytes| u64::from_le_bytes(bytes))
                    .collect();
                let ms = |pass: Pass| {
                    let [begin, end] = [ticks[2 * pass as usize], ticks[2 * pass as usize + 1]];
                    let ns = end.saturating_sub(begin) as f64 * f64::from(self.period);
                    slot.timed[pass as usize].then_some((ns / 1e6) as f32)
                };
                latest = Some(GpuTimes {
                    draw_ms: ms(Pass::Draw).unwrap_or(0.0),
                    particles_ms: ms(Pass::Particles),
                    bloom_ms: ms(Pass::Bloom),
                });
            }
            slot.buffer.unmap();
            slot.state.store(FREE, Ordering::Release);
        }
        latest
    }

    /// The GPU memory its buffers take.
    pub fn bytes(&self) -> u64 {
        RESOLVE_STRIDE * PASSES as u64 + PAIR_BYTES * PASSES as u64 * SLOTS as u64
    }
}
