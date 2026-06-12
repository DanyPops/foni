//! Jitter buffer — adaptive timing for streaming TTS playback.
//!
//! Tracks round-trip latency per TTS call, maintains a running average,
//! and decides whether to insert filler audio when a chunk exceeds its budget.
//!
//! Budget = playback duration of the previous chunk.
//! If predicted RTT > budget, the listener would hear silence.
//! Filler covers the gap.

use std::time::Duration;

const SAMPLE_RATE: u32 = 24_000;

/// Exponential moving average weight for RTT prediction.
const EMA_ALPHA: f64 = 0.3;

/// How many samples before the EMA stabilizes.
const MIN_SAMPLES: usize = 2;

/// A single round-trip measurement.
#[derive(Debug, Clone)]
pub struct Trip {
    pub chunk_index: usize,
    pub rtt: Duration,
    pub audio_bytes: usize,
}

impl Trip {
    pub fn playback_secs(&self) -> f64 {
        // 16-bit mono WAV: 2 bytes per sample, minus 44-byte header
        let samples = self.audio_bytes.saturating_sub(44) / 2;
        samples as f64 / SAMPLE_RATE as f64
    }
}

/// Tracks RTT history and predicts next chunk timing.
#[derive(Debug)]
pub struct JitterTracker {
    trips: Vec<Trip>,
    ema_rtt_ms: f64,
    total_filler_ms: f64,
}

/// What to do before playing the next chunk.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Chunk arrived in time — play immediately.
    Play,
    /// Chunk will be late by this much — insert filler first.
    Filler { gap_ms: f64 },
}

impl Default for JitterTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl JitterTracker {
    pub fn new() -> Self {
        Self {
            trips: Vec::new(),
            ema_rtt_ms: 0.0,
            total_filler_ms: 0.0,
        }
    }

    /// Record a completed round-trip.
    pub fn record(&mut self, trip: Trip) {
        let rtt_ms = trip.rtt.as_secs_f64() * 1000.0;

        if self.trips.is_empty() {
            self.ema_rtt_ms = rtt_ms;
        } else {
            self.ema_rtt_ms = EMA_ALPHA * rtt_ms + (1.0 - EMA_ALPHA) * self.ema_rtt_ms;
        }

        self.trips.push(trip);
    }

    /// Predicted RTT for the next chunk (ms).
    pub fn predicted_rtt_ms(&self) -> f64 {
        self.ema_rtt_ms
    }

    /// How many trips recorded so far.
    pub fn sample_count(&self) -> usize {
        self.trips.len()
    }

    /// Is the prediction stable enough to trust?
    pub fn is_stable(&self) -> bool {
        self.trips.len() >= MIN_SAMPLES
    }

    /// Given the previous chunk's playback duration, decide if we need filler.
    /// Budget = playback_secs of the previous chunk (time we have before silence).
    pub fn decide(&self, budget_secs: f64) -> Action {
        if !self.is_stable() {
            return Action::Play;
        }

        let budget_ms = budget_secs * 1000.0;
        let predicted = self.ema_rtt_ms;

        if predicted > budget_ms {
            Action::Filler {
                gap_ms: predicted - budget_ms,
            }
        } else {
            Action::Play
        }
    }

    /// Total filler inserted so far (ms).
    pub fn total_filler_ms(&self) -> f64 {
        self.total_filler_ms
    }

    /// Record that filler was inserted.
    pub fn record_filler(&mut self, ms: f64) {
        self.total_filler_ms += ms;
    }

    /// Average RTT across all trips (ms).
    pub fn mean_rtt_ms(&self) -> f64 {
        if self.trips.is_empty() {
            return 0.0;
        }
        let sum: f64 = self
            .trips
            .iter()
            .map(|t| t.rtt.as_secs_f64() * 1000.0)
            .sum();
        sum / self.trips.len() as f64
    }

    /// Worst RTT seen (ms).
    pub fn max_rtt_ms(&self) -> f64 {
        self.trips
            .iter()
            .map(|t| t.rtt.as_secs_f64() * 1000.0)
            .fold(0.0f64, f64::max)
    }
}

/// Generate silence filler audio (samples).
pub fn silence_filler(duration_ms: f64) -> Vec<f32> {
    let n = (SAMPLE_RATE as f64 * duration_ms / 1000.0) as usize;
    vec![0.0f32; n]
}

/// Generate breath-like filler (low-amplitude noise).
pub fn breath_filler(duration_ms: f64) -> Vec<f32> {
    let n = (SAMPLE_RATE as f64 * duration_ms / 1000.0) as usize;
    let mut rng_state: u32 = 42;
    (0..n)
        .map(|_| {
            rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
            let noise = (rng_state >> 16) as f32 / 65536.0 - 0.5;
            noise * 0.008
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trip(index: usize, rtt_ms: u64, audio_bytes: usize) -> Trip {
        Trip {
            chunk_index: index,
            rtt: Duration::from_millis(rtt_ms),
            audio_bytes,
        }
    }

    // 0.5s of 24kHz mono 16-bit = 12000 samples = 24000 bytes + 44 header
    const HALF_SEC_WAV: usize = 24044;

    #[test]
    fn empty_tracker_predicts_zero() {
        let t = JitterTracker::new();
        assert_eq!(t.predicted_rtt_ms(), 0.0);
        assert_eq!(t.sample_count(), 0);
        assert!(!t.is_stable());
    }

    #[test]
    fn single_trip_sets_ema() {
        let mut t = JitterTracker::new();
        t.record(trip(0, 3000, HALF_SEC_WAV));
        assert!((t.predicted_rtt_ms() - 3000.0).abs() < 1.0);
    }

    #[test]
    fn ema_converges() {
        let mut t = JitterTracker::new();
        t.record(trip(0, 6000, HALF_SEC_WAV));
        t.record(trip(1, 6000, HALF_SEC_WAV));
        t.record(trip(2, 6000, HALF_SEC_WAV));
        assert!((t.predicted_rtt_ms() - 6000.0).abs() < 100.0);
    }

    #[test]
    fn ema_reacts_to_spike() {
        let mut t = JitterTracker::new();
        t.record(trip(0, 3000, HALF_SEC_WAV));
        t.record(trip(1, 3000, HALF_SEC_WAV));
        t.record(trip(2, 9000, HALF_SEC_WAV)); // spike!
        assert!(t.predicted_rtt_ms() > 4000.0, "should react to spike");
        assert!(t.predicted_rtt_ms() < 9000.0, "should be smoothed");
    }

    #[test]
    fn stable_after_min_samples() {
        let mut t = JitterTracker::new();
        t.record(trip(0, 3000, HALF_SEC_WAV));
        assert!(!t.is_stable());
        t.record(trip(1, 3000, HALF_SEC_WAV));
        assert!(t.is_stable());
    }

    #[test]
    fn decide_play_when_fast() {
        let mut t = JitterTracker::new();
        t.record(trip(0, 2000, HALF_SEC_WAV)); // 2s RTT
        t.record(trip(1, 2000, HALF_SEC_WAV));
        // Budget = 5s playback → plenty of time
        assert_eq!(t.decide(5.0), Action::Play);
    }

    #[test]
    fn decide_filler_when_slow() {
        let mut t = JitterTracker::new();
        t.record(trip(0, 6000, HALF_SEC_WAV)); // 6s RTT
        t.record(trip(1, 6000, HALF_SEC_WAV));
        // Budget = 3s playback → will be late by ~3s
        let action = t.decide(3.0);
        match action {
            Action::Filler { gap_ms } => {
                assert!(gap_ms > 2000.0, "gap should be ~3000ms, got {gap_ms}");
            }
            Action::Play => panic!("should need filler"),
        }
    }

    #[test]
    fn decide_play_when_unstable() {
        let mut t = JitterTracker::new();
        t.record(trip(0, 99999, HALF_SEC_WAV));
        // Only 1 sample — not stable, defaults to Play
        assert_eq!(t.decide(0.1), Action::Play);
    }

    #[test]
    fn playback_secs_correct() {
        let t = trip(0, 1000, HALF_SEC_WAV);
        assert!((t.playback_secs() - 0.5).abs() < 0.01);
    }

    #[test]
    fn filler_tracking() {
        let mut t = JitterTracker::new();
        assert_eq!(t.total_filler_ms(), 0.0);
        t.record_filler(500.0);
        t.record_filler(300.0);
        assert!((t.total_filler_ms() - 800.0).abs() < 0.01);
    }

    #[test]
    fn mean_and_max_rtt() {
        let mut t = JitterTracker::new();
        t.record(trip(0, 2000, HALF_SEC_WAV));
        t.record(trip(1, 4000, HALF_SEC_WAV));
        t.record(trip(2, 3000, HALF_SEC_WAV));
        assert!((t.mean_rtt_ms() - 3000.0).abs() < 1.0);
        assert!((t.max_rtt_ms() - 4000.0).abs() < 1.0);
    }

    #[test]
    fn silence_filler_length() {
        let f = silence_filler(500.0);
        assert_eq!(f.len(), 12000); // 0.5s * 24kHz
        assert!(f.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn breath_filler_length_and_quiet() {
        let f = breath_filler(500.0);
        assert_eq!(f.len(), 12000);
        let rms = (f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt();
        assert!(
            rms < 0.01,
            "breath filler should be very quiet, got rms={rms}"
        );
        assert!(
            rms > 0.001,
            "breath filler should not be silent, got rms={rms}"
        );
    }
}
