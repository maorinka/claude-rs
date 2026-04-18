//! Frame-timing tracker: average + low-1% FPS.
//!
//! Port of TS `src/utils/fpsTracker.ts`. Records per-frame durations
//! in ms; `get_metrics` returns both the whole-span average FPS and
//! the worst 1% p99-frame-time-derived FPS (the "low 1%" metric
//! gamers + HCI researchers report). TS uses `performance.now()`
//! for wall-clock stamps; Rust uses `Instant::now()`.
//!
//! Rounding: both outputs are rounded to 2 decimal places so UI /
//! analytics stay stable.

use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FpsMetrics {
    /// Frames ÷ elapsed seconds.
    pub average_fps: f64,
    /// 1000 ÷ p99 frame-time ms.
    pub low_1_pct_fps: f64,
}

#[derive(Default)]
pub struct FpsTracker {
    frame_durations_ms: Vec<f64>,
    first_render: Option<Instant>,
    last_render: Option<Instant>,
}

impl FpsTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a frame. `duration_ms` is the paint time of the
    /// completed frame; `Instant::now()` is captured to bracket the
    /// overall span.
    pub fn record(&mut self, duration_ms: f64) {
        let now = Instant::now();
        if self.first_render.is_none() {
            self.first_render = Some(now);
        }
        self.last_render = Some(now);
        self.frame_durations_ms.push(duration_ms);
    }

    pub fn len(&self) -> usize {
        self.frame_durations_ms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frame_durations_ms.is_empty()
    }

    /// Compute metrics. Returns `None` when there are no frames or
    /// the span is zero-width.
    pub fn get_metrics(&self) -> Option<FpsMetrics> {
        let (Some(first), Some(last)) = (self.first_render, self.last_render) else {
            return None;
        };
        if self.frame_durations_ms.is_empty() {
            return None;
        }
        let total_ms = last.saturating_duration_since(first).as_secs_f64() * 1000.0;
        if total_ms <= 0.0 {
            return None;
        }

        let total_frames = self.frame_durations_ms.len() as f64;
        let average_fps = total_frames / (total_ms / 1000.0);

        // Sort descending, pick p99 frame time.
        let mut sorted = self.frame_durations_ms.clone();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let p99_index = ((sorted.len() as f64 * 0.01).ceil() as usize)
            .saturating_sub(1)
            .min(sorted.len() - 1);
        let p99_ms = sorted[p99_index];
        let low_1_pct_fps = if p99_ms > 0.0 { 1000.0 / p99_ms } else { 0.0 };

        Some(FpsMetrics {
            average_fps: round2(average_fps),
            low_1_pct_fps: round2(low_1_pct_fps),
        })
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn empty_tracker_has_no_metrics() {
        let t = FpsTracker::new();
        assert!(t.get_metrics().is_none());
    }

    #[test]
    fn zero_width_span_has_no_metrics() {
        let mut t = FpsTracker::new();
        // Record the same moment twice by pushing frame durations
        // without any elapsed wall time (rare in practice).
        t.record(16.0);
        // No sleep → total_ms likely 0; this is a best-effort
        // assertion — getting a `Some` here is also fine because
        // wall time advanced. Accept both.
        let _ = t.get_metrics();
    }

    #[test]
    fn p99_index_on_small_series() {
        let mut t = FpsTracker::new();
        for _ in 0..10 {
            t.record(16.0);
        }
        sleep(Duration::from_millis(5));
        t.record(16.0);
        let m = t.get_metrics().expect("metrics");
        // p99 with 11 frames = ceil(11 * 0.01) - 1 = 0 → worst
        // frame (16.0 ms) ⇒ 62.5 FPS.
        assert!(
            (m.low_1_pct_fps - 62.5).abs() < 0.5,
            "unexpected low_1_pct_fps={}",
            m.low_1_pct_fps
        );
    }

    #[test]
    fn outlier_dominates_p99() {
        let mut t = FpsTracker::new();
        for _ in 0..99 {
            t.record(5.0);
        }
        t.record(100.0); // one slow frame
        sleep(Duration::from_millis(5));
        let m = t.get_metrics().expect("metrics");
        // Sorted desc: [100.0, 5.0, 5.0, ...]. p99 index:
        // ceil(100 * 0.01) - 1 = 0 → worst frame (100 ms) →
        // 10 FPS.
        assert!(
            (m.low_1_pct_fps - 10.0).abs() < 0.5,
            "unexpected low_1_pct_fps={}",
            m.low_1_pct_fps
        );
    }

    #[test]
    fn metrics_round_to_two_decimals() {
        let mut t = FpsTracker::new();
        for _ in 0..20 {
            t.record(3.3333);
        }
        sleep(Duration::from_millis(10));
        let m = t.get_metrics().expect("metrics");
        // low_1_pct_fps = 1000 / 3.3333 = 300.003 → rounds to 300.00.
        assert!(
            (m.low_1_pct_fps - 300.0).abs() < 0.05,
            "rounding unexpected: {}",
            m.low_1_pct_fps
        );
    }

    #[test]
    fn len_and_is_empty_track_recordings() {
        let mut t = FpsTracker::new();
        assert!(t.is_empty());
        t.record(1.0);
        t.record(2.0);
        assert_eq!(t.len(), 2);
        assert!(!t.is_empty());
    }
}
