//! Scroll position along the timeline, in station units (0 = intro).
//! Input moves the target; the position follows with exponential smoothing.

pub struct Timeline {
    position: f32,
    target: f32,
    last: f32,
}

/// Higher is snappier; 1/s.
const SMOOTHING: f32 = 5.0;

impl Timeline {
    #[cfg(test)]
    pub fn new(stations: usize) -> Self {
        Self::starting_at(stations, 0)
    }

    /// Starts at `station` (clamped), without animating there.
    pub fn starting_at(stations: usize, station: usize) -> Self {
        let last = stations.saturating_sub(1) as f32;
        let start = (station as f32).min(last);
        Self {
            position: start,
            target: start,
            last,
        }
    }

    pub fn position(&self) -> f32 {
        self.position
    }

    /// Continuous movement, e.g. wheel or touch; positive moves forward.
    pub fn scroll(&mut self, delta: f32) {
        self.target = (self.target + delta).clamp(0.0, self.last);
    }

    /// Jumps to the next (`1`) or previous (`-1`) station.
    pub fn step(&mut self, direction: i32) {
        let current = self.target.round();
        self.target = (current + direction as f32).clamp(0.0, self.last);
    }

    pub fn go_to_start(&mut self) {
        self.target = 0.0;
    }

    pub fn go_to_end(&mut self) {
        self.target = self.last;
    }

    /// Advances the animation; returns whether it is still moving.
    pub fn update(&mut self, dt: f32) -> bool {
        let difference = self.target - self.position;
        if difference.abs() < 1e-4 {
            self.position = self.target;
            return false;
        }
        self.position += difference * (1.0 - (-dt * SMOOTHING).exp());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_and_converges() {
        let mut timeline = Timeline::new(3);
        timeline.scroll(10.0);
        while timeline.update(1.0 / 60.0) {}
        assert_eq!(timeline.position(), 2.0);
        timeline.step(1);
        assert!(!timeline.update(1.0 / 60.0));
        timeline.step(-1);
        while timeline.update(1.0 / 60.0) {}
        assert_eq!(timeline.position(), 1.0);
    }
}
