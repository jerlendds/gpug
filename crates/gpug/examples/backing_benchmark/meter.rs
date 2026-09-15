use std::collections::VecDeque;
use std::time::Duration;

/// Rolling CPU frame-completion cadence, independent of layout simulation ticks.
pub(super) struct FrameMeter {
    intervals: VecDeque<f64>,
}

impl Default for FrameMeter {
    fn default() -> Self {
        Self {
            intervals: VecDeque::with_capacity(30),
        }
    }
}

impl FrameMeter {
    pub(super) fn record(&mut self, interval: Duration) {
        if interval.is_zero() {
            return;
        }
        if self.intervals.len() == 30 {
            self.intervals.pop_front();
        }
        self.intervals.push_back(interval.as_secs_f64());
    }

    pub(super) fn fps(&self) -> Option<f64> {
        (!self.intervals.is_empty())
            .then(|| self.intervals.len() as f64 / self.intervals.iter().sum::<f64>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_advances_without_any_layout_ticks_and_tracks_recent_frames() {
        let mut meter = FrameMeter::default();
        assert_eq!(meter.fps(), None);
        meter.record(Duration::ZERO);
        assert_eq!(meter.fps(), None);
        for _ in 0..30 {
            meter.record(Duration::from_millis(16));
        }
        assert!((meter.fps().unwrap() - 62.5).abs() < 1e-9);
        for _ in 0..30 {
            meter.record(Duration::from_millis(40));
        }
        assert!((meter.fps().unwrap() - 25.).abs() < 1e-9);
        assert_eq!(meter.intervals.len(), 30);
    }
}
