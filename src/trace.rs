//! The live side: turning telemetry frames into a continuous trace, and the brake
//! events (with their peaks) inside it.

use std::collections::VecDeque;

use crate::lap::{BRAKE_OFF, BRAKE_ON};

/// Most samples a trace keeps, whatever [`LiveTrace::prune`] allows: 200 s at 60 Hz, which
/// holds the widest distance window (1600 m) down to 8 m/s, below pit-lane limits. Bounds a
/// car that sits still with its pedals moving; a still car with steady pedals costs two
/// samples (see [`LiveTrace::push`]).
pub const MAX_SAMPLES: usize = 12_000;
/// Samples within this many metres of each other are the car standing still.
const STILL_M: f64 = 0.05;
/// Pedal changes smaller than this (0..1) don't show on the graph.
const STILL_PEDAL: f32 = 0.002;

/// One live sample on the graph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveSample {
    /// Session seconds.
    pub t: f64,
    /// Cumulative metres: (laps + lap fraction) × track length. Continuous across S/F.
    pub d: f64,
    pub brake: f32,
    pub throttle: f32,
}

/// A braking event and the highest pedal value reached in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrakeEvent {
    pub peak: f32,
    pub peak_t: f64,
    pub peak_d: f64,
    /// Still on the brake: `peak` is the running maximum.
    pub active: bool,
}

/// Rolling buffer of live samples plus the brake events inside it.
#[derive(Debug, Default, Clone)]
pub struct LiveTrace {
    samples: VecDeque<LiveSample>,
    events: VecDeque<BrakeEvent>,
}

impl LiveTrace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.samples.clear();
        self.events.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn last(&self) -> Option<&LiveSample> {
        self.samples.back()
    }

    pub fn samples(&self) -> &VecDeque<LiveSample> {
        &self.samples
    }

    pub fn events(&self) -> impl Iterator<Item = &BrakeEvent> {
        self.events.iter()
    }

    /// Appends a sample. While the car stands still with steady pedals, the stop is kept as
    /// its first sample plus the latest one (moved forward in time), so a parked car doesn't
    /// grow the trace and the time axis still draws a flat line across the stop. Past
    /// [`MAX_SAMPLES`] the oldest samples, and the brake events before them, go.
    pub fn push(&mut self, s: LiveSample) {
        let n = self.samples.len();
        if n >= 2 && still(&self.samples[n - 2], &self.samples[n - 1]) && still(&self.samples[n - 2], &s) {
            self.samples[n - 1] = s;
        } else {
            self.samples.push_back(s);
            if self.samples.len() > MAX_SAMPLES {
                self.samples.pop_front();
                let first_t = self.samples.front().map_or(s.t, |f| f.t);
                while self.events.front().is_some_and(|e| !e.active && e.peak_t < first_t) {
                    self.events.pop_front();
                }
            }
        }
        match self.events.back_mut().filter(|e| e.active) {
            None => {
                if s.brake > BRAKE_ON {
                    self.events.push_back(BrakeEvent { peak: s.brake, peak_t: s.t, peak_d: s.d, active: true });
                }
            }
            Some(ev) => {
                if s.brake > ev.peak {
                    ev.peak = s.brake;
                    ev.peak_t = s.t;
                    ev.peak_d = s.d;
                }
                if s.brake < BRAKE_OFF {
                    ev.active = false;
                }
            }
        }
    }

    /// Forgets samples older than both limits (the widest window either axis can show).
    /// Keeps at least the two newest samples.
    pub fn prune(&mut self, min_t: f64, min_d: f64) {
        while self.samples.len() > 2 {
            let s = self.samples[0];
            if s.t < min_t && s.d < min_d {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        while let Some(e) = self.events.front() {
            if !e.active && e.peak_t < min_t && e.peak_d < min_d {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }

    /// Index of the first sample whose key (`t` or `d`) is `>= value`.
    pub fn first_at_or_after(&self, value: f64, by_time: bool) -> usize {
        self.samples.partition_point(|s| (if by_time { s.t } else { s.d }) < value)
    }
}

/// `b` is where `a` was, with the same pedals, as far as the graph can show.
fn still(a: &LiveSample, b: &LiveSample) -> bool {
    (a.d - b.d).abs() < STILL_M
        && (a.brake - b.brake).abs() < STILL_PEDAL
        && (a.throttle - b.throttle).abs() < STILL_PEDAL
}

/// What a telemetry frame means for the trace.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    /// Not on track (lap fraction invalid): skip the frame.
    Invalid,
    /// Continues the current trace at this cumulative lap position (laps + fraction).
    Continuous(f64),
    /// A jump the trace can't bridge (tow, reset, new session, replay seek):
    /// clear the trace and restart at this position.
    Reset(f64),
}

/// Turns (session time, lap fraction) into a cumulative lap position that stays
/// continuous across start/finish, and flags jumps.
///
/// Doesn't rely on iRacing's `Lap` counter, which can tick a frame before or after
/// `LapDistPct` wraps.
#[derive(Debug, Default, Clone)]
pub struct DistanceTracker {
    last: Option<(f64, f64)>, // (t, pct)
    pos: f64,
    /// Frames were invalid since `last` (garage, tow): the next valid one restarts.
    gap: bool,
}

impl DistanceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Current cumulative position in laps.
    pub fn position(&self) -> f64 {
        self.pos
    }

    /// The car left the track (garage, replay, pit stall): the next frame restarts.
    pub fn gap(&mut self) {
        self.gap = true;
    }

    pub fn update(&mut self, t: f64, pct: f64) -> Progress {
        if !(0.0..=1.0).contains(&pct) || !t.is_finite() {
            self.gap = true;
            return Progress::Invalid;
        }
        let Some((lt, lp)) = self.last.filter(|_| !self.gap) else {
            self.last = Some((t, pct));
            self.gap = false;
            self.pos = pct;
            return Progress::Reset(self.pos);
        };
        let dt = t - lt;
        let mut delta = pct - lp;
        if delta < -0.5 {
            delta += 1.0; // crossed S/F forwards
        } else if delta > 0.5 {
            delta -= 1.0; // backwards over S/F
        }
        // Loose on purpose: ~0.12 laps/s is 120 m/s on a 1 km oval.
        let allowed = (0.005 + dt.max(0.0) * 0.12).min(0.1);
        self.last = Some((t, pct));
        if dt < 0.0 || delta.abs() > allowed {
            self.pos = pct;
            return Progress::Reset(self.pos);
        }
        self.pos += delta;
        Progress::Continuous(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: f64, d: f64, brake: f32) -> LiveSample {
        LiveSample { t, d, brake, throttle: 0.0 }
    }

    #[test]
    fn tracks_brake_event_peaks_with_hysteresis() {
        let mut tr = LiveTrace::new();
        for (i, b) in [0.0, 0.03, 0.2, 0.6, 0.55, 0.04, 0.3, 0.01, 0.0].iter().enumerate() {
            tr.push(s(i as f64, i as f64 * 10.0, *b));
        }
        let ev: Vec<_> = tr.events().copied().collect();
        // 0.04 is above BRAKE_OFF, so 0.2..0.3 is one event; it ends at 0.01.
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].peak, 0.6);
        assert_eq!(ev[0].peak_t, 3.0);
        assert!(!ev[0].active);
    }

    #[test]
    fn active_event_reports_running_max() {
        let mut tr = LiveTrace::new();
        tr.push(s(0.0, 0.0, 0.1));
        tr.push(s(1.0, 1.0, 0.4));
        let e = *tr.events().next().unwrap();
        assert!(e.active);
        assert_eq!(e.peak, 0.4);
    }

    #[test]
    fn prune_drops_only_what_both_limits_exclude() {
        let mut tr = LiveTrace::new();
        for i in 0..100 {
            tr.push(s(i as f64, i as f64, if i == 5 { 0.5 } else { 0.0 }));
        }
        tr.prune(50.0, 10.0); // distance limit keeps more
        assert_eq!(tr.samples()[0].t, 10.0);
        assert_eq!(tr.events().count(), 0);
        assert_eq!(tr.first_at_or_after(20.0, true), 10);
        assert_eq!(tr.first_at_or_after(20.5, false), 11);
    }

    /// Drives 100 samples 1 m apart, then stands still at 60 Hz for `secs` with the pedals
    /// from `pedals(frame)` (throttle, brake), pruning like `LiveFeed::push`.
    fn stop(secs: f64, pedals: impl Fn(usize) -> (f32, f32)) -> LiveTrace {
        let mut tr = LiveTrace::new();
        for i in 0..100 {
            tr.push(s(i as f64 / 60.0, i as f64, 0.0));
        }
        let (t0, d) = (100.0 / 60.0, 99.0);
        for i in 0..(secs * 60.0) as usize {
            let (throttle, brake) = pedals(i);
            let t = t0 + i as f64 / 60.0;
            tr.push(LiveSample { t, d, brake, throttle });
            tr.prune(t - 25.0, d - 1600.0);
        }
        tr
    }

    #[test]
    fn a_stop_keeps_its_first_and_latest_sample() {
        let tr = stop(600.0, |_| (0.0, 0.6));
        assert_eq!(tr.len(), 102, "the drive, then the stop's first and latest sample");
        let (first, last) = (tr.samples()[100], tr.samples()[101]);
        assert_eq!((first.t, first.d, first.brake), (100.0 / 60.0, 99.0, 0.6));
        assert!((last.t - (100.0 / 60.0 + 599.0 + 59.0 / 60.0)).abs() < 1e-9, "latest time: {}", last.t);
        assert_eq!((last.d, last.brake), (99.0, 0.6));
        // On the time axis the stop is one flat segment across the whole window.
        assert_eq!(tr.first_at_or_after(last.t - 8.0, true), 101);
        assert_eq!(tr.events().count(), 1);
    }

    #[test]
    fn a_stop_with_moving_pedals_is_capped() {
        // Blipping the throttle and pumping the brake (one press a second) for 10 minutes.
        let tr = stop(600.0, |i| ((i % 7) as f32 / 10.0, if i % 60 < 30 { 0.8 } else { 0.0 }));
        assert_eq!(tr.len(), MAX_SAMPLES);
        let first = tr.samples()[0].t;
        assert!(first > 600.0 - MAX_SAMPLES as f64 / 60.0, "the oldest went first: {first}");
        assert!(tr.events().all(|e| e.peak_t >= first), "no events from before the trace");
        assert!(tr.events().count() <= MAX_SAMPLES / 60 + 1);
    }

    #[test]
    fn a_creeping_car_keeps_its_progress() {
        let mut tr = LiveTrace::new();
        for i in 0..600 {
            tr.push(s(i as f64 / 60.0, i as f64 * 0.01, 0.0)); // 0.6 m/s
        }
        // A sample every 4-5 cm, and always the newest.
        assert!((120..=160).contains(&tr.len()), "{}", tr.len());
        assert!((tr.last().unwrap().d - 5.99).abs() < 1e-9);
        assert!(tr.samples().iter().zip(tr.samples().iter().skip(1)).all(|(a, b)| b.d - a.d <= STILL_M + 0.01));
    }

    #[test]
    fn distance_is_continuous_across_start_finish() {
        let mut dt = DistanceTracker::new();
        assert_eq!(dt.update(0.0, 0.998), Progress::Reset(0.998));
        let Progress::Continuous(p) = dt.update(1.0 / 60.0, 0.9995) else { panic!() };
        assert!((p - 0.9995).abs() < 1e-12);
        let Progress::Continuous(p) = dt.update(2.0 / 60.0, 0.0005) else { panic!() };
        assert!((p - 1.0005).abs() < 1e-12);
    }

    #[test]
    fn jumps_and_time_reversal_reset() {
        let mut dt = DistanceTracker::new();
        dt.update(10.0, 0.30);
        assert_eq!(dt.update(10.0 + 1.0 / 60.0, 0.80), Progress::Reset(0.80)); // tow / reset
        assert_eq!(dt.update(5.0, 0.81), Progress::Reset(0.81)); // new session
        assert!(matches!(dt.update(5.0 + 1.0 / 60.0, 0.8101), Progress::Continuous(_)));
        assert_eq!(dt.update(5.1, -1.0), Progress::Invalid); // in the garage
        assert_eq!(dt.update(600.0, 0.02), Progress::Reset(0.02)); // back out on track
        assert!(matches!(dt.update(600.0 + 1.0 / 60.0, 0.0201), Progress::Continuous(_)));
    }
}
