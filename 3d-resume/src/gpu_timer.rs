//! GPU time per frame from timestamp queries, for the performance overlay:
//! the render pass and, when it runs, the particle simulation's compute
//! pass. Each frame's timestamps are copied into one of a few readback
//! buffers and read a frame or two later, so timing never stalls the GPU.
//! Frames that find no free buffer go untimed.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::gpu::Context;
use crate::stats::GpuTimes;

/// Readback buffer states.
const FREE: u8 = 0;
const PENDING: u8 = 1;
const READY: u8 = 2;
/// Timestamps per frame: the render pass's begin and end, then the compute
/// pass's.
const QUERIES: u32 = 4;
const BYTES: u64 = QUERIES as u64 * 8;
const SLOTS: usize = 3;

pub struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    /// Nanoseconds per timestamp tick.
    period: f32,
    /// The slot this frame is timed into (picked by its first pass).
    current: Option<usize>,
    /// Whether this frame's compute pass wrote its timestamps.
    particles: bool,
}

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    particles: bool,
}

impl GpuTimer {
    /// `None` where the device has no timestamp queries.
    pub fn new(ctx: &Context) -> Option<Self> {
        let device = &ctx.device;
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("frame timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERIES,
        });
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: BYTES,
                usage,
                mapped_at_creation: false,
            })
        };
        let resolve = buffer(
            "timestamps",
            wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        );
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: buffer(
                    "timestamp readback",
                    wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                ),
                state: Arc::new(AtomicU8::new(FREE)),
                particles: false,
            })
            .collect();
        Some(Self {
            queries,
            resolve,
            slots,
            period: ctx.queue.get_timestamp_period(),
            current: None,
            particles: false,
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
        self.particles = true;
        Some(wgpu::ComputePassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(2),
            end_of_pass_write_index: Some(3),
        })
    }

    /// Timestamps for this frame's render pass.
    pub fn render_writes(&mut self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.slot()?;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        })
    }

    /// After the frame's render pass: copies its timestamps to the frame's
    /// readback buffer.
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(index) = self.current else { return };
        // Unwritten queries have no defined value: resolve only those used.
        let count = if self.particles { QUERIES } else { 2 };
        encoder.resolve_query_set(&self.queries, 0..count, &self.resolve, 0);
        let slot = &mut self.slots[index];
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &slot.buffer, 0, BYTES);
        slot.particles = self.particles;
    }

    /// After submitting the frame: starts reading its timestamps back.
    pub fn map(&mut self) {
        let Some(index) = self.current.take() else {
            return;
        };
        self.particles = false;
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
                let ms = |begin: u64, end: u64| {
                    end.saturating_sub(begin) as f64 * f64::from(self.period) / 1e6
                };
                latest = Some(GpuTimes {
                    draw_ms: ms(ticks[0], ticks[1]) as f32,
                    particles_ms: slot.particles.then(|| ms(ticks[2], ticks[3]) as f32),
                });
            }
            slot.buffer.unmap();
            slot.state.store(FREE, Ordering::Release);
        }
        latest
    }

    /// The GPU memory it takes.
    pub fn bytes(&self) -> u64 {
        BYTES * (1 + SLOTS as u64)
    }
}
