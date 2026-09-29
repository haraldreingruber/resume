//! The particle intro's choreography, on the CPU. On load, the particles
//! swirl in and form the name, then the crisp title fades in over them;
//! scrolling towards the first station scatters them and scrolling back
//! re-forms them; the mouse pushes them aside. The GPU simulation
//! (`particles.rs`) only runs while something changed recently, so an intro
//! at rest costs nothing.

use glam::Vec3;

/// Simulated time by which the particles have formed the name and the crisp
/// title has faded in.
pub const ASSEMBLED: f32 = 2.3;
/// Keep simulating this long after the last change, until the particles
/// have come to rest.
const SETTLE: f32 = 2.5;
/// Longest integration step; a frame runs as many as it needs.
const SUBSTEP: f32 = 1.0 / 120.0;
const MAX_SUBSTEPS: u32 = 12;
/// Timeline position from which the particles are fully scattered.
const SCATTERED_AT: f32 = 0.7;

/// How far the particles are scattered at a timeline position: 0 at the
/// intro, 1 from `SCATTERED_AT` on.
pub fn scatter(position: f32) -> f32 {
    (position / SCATTERED_AT).clamp(0.0, 1.0)
}

/// Opacity of the crisp title: it fades in as the particles settle into the
/// name, and out as soon as they start to scatter.
pub fn title_opacity(time: f32, scatter: f32) -> f32 {
    smoothstep(ASSEMBLED - 0.9, ASSEMBLED, time) * (1.0 - smoothstep(0.02, 0.3, scatter))
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One frame of simulation for the GPU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    /// Simulated time at the start of the frame.
    pub time: f32,
    /// Integration step and count; together they cover the frame.
    pub substep: f32,
    pub substeps: u32,
    pub scatter: f32,
    /// The mouse on the title's plane, while it is near the title.
    pub pointer: Option<Vec3>,
}

pub struct Intro {
    /// Simulated time since the particles started.
    time: f32,
    /// Time since the last change (scatter, pointer, assembly).
    idle: f32,
    scatter: f32,
    pointer: Option<Vec3>,
}

impl Intro {
    /// `assembled`: skip the assembly, e.g. when a deep link starts at
    /// another station (the particles form the name as you scroll back).
    pub fn new(assembled: bool) -> Self {
        Self {
            time: if assembled { ASSEMBLED } else { 0.0 },
            idle: 0.0,
            scatter: 0.0,
            pointer: None,
        }
    }

    /// Simulated time (screenshots stop the assembly at a given time).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn pointer(&self) -> Option<Vec3> {
        self.pointer
    }

    /// Advances by `dt` seconds with the current scatter and pointer. Returns
    /// the simulation step to run, or `None` once everything is at rest.
    pub fn advance(&mut self, dt: f32, scatter: f32, pointer: Option<Vec3>) -> Option<Step> {
        let changed = self.time < ASSEMBLED || scatter != self.scatter || pointer != self.pointer;
        self.scatter = scatter;
        self.pointer = pointer;
        self.idle = if changed { 0.0 } else { self.idle + dt };
        if !self.active() {
            return None;
        }
        // Tolerance: 1/60 s must be 2 steps, not 3 because of rounding.
        let substeps = ((dt / SUBSTEP - 1e-3).ceil() as u32).clamp(1, MAX_SUBSTEPS);
        let step = Step {
            time: self.time,
            substep: dt / substeps as f32,
            substeps,
            scatter,
            pointer,
        };
        self.time += dt;
        Some(step)
    }

    /// Whether the particles are still moving (frames are needed).
    pub fn active(&self) -> bool {
        self.idle <= SETTLE
    }

    pub fn title_opacity(&self) -> f32 {
        title_opacity(self.time, self.scatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    #[test]
    fn scatter_follows_the_timeline() {
        assert_eq!(scatter(0.0), 0.0);
        assert_eq!(scatter(SCATTERED_AT / 2.0), 0.5);
        assert_eq!(scatter(3.0), 1.0);
    }

    #[test]
    fn title_fades_in_when_assembled_and_out_when_scattered() {
        assert_eq!(title_opacity(0.0, 0.0), 0.0);
        assert_eq!(title_opacity(ASSEMBLED, 0.0), 1.0);
        assert_eq!(title_opacity(ASSEMBLED, 0.3), 0.0);
        assert!(title_opacity(ASSEMBLED - 0.45, 0.0) > 0.0);
    }

    #[test]
    fn simulates_until_at_rest_and_wakes_on_changes() {
        let mut intro = Intro::new(false);
        let mut frames = 0;
        while intro.advance(FRAME, 0.0, None).is_some() {
            frames += 1;
            assert!(frames < 1000, "never comes to rest");
        }
        // Assembly plus settling, then nothing to do.
        assert!(intro.time() >= ASSEMBLED + SETTLE - FRAME);
        assert!(!intro.active());
        assert_eq!(intro.advance(FRAME, 0.0, None), None);
        // Scrolling or the mouse wake it up again.
        assert!(intro.advance(FRAME, 0.1, None).is_some());
        let pointer = Some(Vec3::new(0.5, 1.0, 0.0));
        assert!(intro.advance(FRAME, 0.1, pointer).is_some());
    }

    #[test]
    fn frames_are_split_into_substeps() {
        let mut intro = Intro::new(true);
        let step = intro.advance(FRAME, 0.0, None).unwrap();
        assert_eq!(step.substeps, 2);
        let step = intro.advance(0.1, 0.5, None).unwrap();
        assert_eq!(step.substeps, MAX_SUBSTEPS);
        assert!((step.substep * step.substeps as f32 - 0.1).abs() < 1e-6);
        assert!((step.time - ASSEMBLED - FRAME).abs() < 1e-6);
    }
}
