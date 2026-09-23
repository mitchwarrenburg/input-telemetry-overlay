//! The live side of the graph: telemetry frames (from iRacing or the demo driver)
//! turned into a trace and the car's current position.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::demo::SimulatedDriver;
use crate::lap::Lap;
use crate::telemetry::TelemetryFrame;
use crate::trace::{DistanceTracker, LiveSample, LiveTrace, Progress};
use crate::ui::graph::CarNow;

/// Track length when neither the session nor the reference knows it, metres.
pub const FALLBACK_TRACK_LENGTH_M: f64 = 5000.0;
/// History kept behind the car: the widest window any setting can show.
const KEEP_S: f64 = 25.0;
const KEEP_M: f64 = 1600.0;

/// The live trace, the distance tracker that feeds it and where the car is now.
#[derive(Debug)]
pub struct LiveFeed {
    trace: LiveTrace,
    tracker: DistanceTracker,
    track_length: f64,
    now: Option<CarNow>,
}

impl Default for LiveFeed {
    fn default() -> Self {
        Self {
            trace: LiveTrace::new(),
            tracker: DistanceTracker::new(),
            track_length: FALLBACK_TRACK_LENGTH_M,
            now: None,
        }
    }
}

impl LiveFeed {
    pub fn trace(&self) -> &LiveTrace {
        &self.trace
    }

    pub fn now(&self) -> Option<CarNow> {
        self.now
    }

    /// Metres per lap used for the trace's distances.
    pub fn track_length(&self) -> f64 {
        self.track_length
    }

    /// Forgets the trace and the car.
    pub fn clear(&mut self) {
        self.trace.clear();
        self.tracker.reset();
        self.now = None;
    }

    /// Distances computed with another length don't line up: a change clears the trace.
    pub fn set_track_length(&mut self, metres: f64) {
        if (metres - self.track_length).abs() > 0.01 {
            self.track_length = metres;
            self.clear();
        }
    }

    pub fn push(&mut self, f: &TelemetryFrame) {
        if !f.on_track {
            self.tracker.gap();
            return;
        }
        let lap_pos = match self.tracker.update(f.session_time, f.lap_dist_pct) {
            Progress::Invalid => return,
            Progress::Reset(p) => {
                self.trace.clear();
                p
            }
            Progress::Continuous(p) => p,
        };
        let d = lap_pos * self.track_length;
        self.trace.push(LiveSample { t: f.session_time, d, brake: f.brake, throttle: f.throttle });
        self.trace.prune(f.session_time - KEEP_S, d - KEEP_M);
        self.now = Some(CarNow { t: f.session_time, lap_pos, throttle: f.throttle, brake: f.brake });
    }

    /// Metres driven since the trace started.
    fn distance(&self) -> f64 {
        self.now.map_or(0.0, |n| n.lap_pos * self.track_length)
    }
}

/// The simulated driver, stepped by wall-clock time.
pub struct Demo {
    driver: SimulatedDriver,
    last: Instant,
    /// Time not yet stepped, seconds.
    carry: f64,
}

impl Demo {
    /// Lap distance the demo starts at, so the first frame already shows a braking zone
    /// with history behind it.
    const START_M: f64 = 3800.0;
    /// At most this much time is caught up after a stall (e.g. a blocked UI thread).
    const MAX_CATCH_UP_S: f64 = 0.5;
    const SEED: u32 = 7;

    /// Starts driving `lap` into `feed` (clearing it), pre-rolled to [`Self::START_M`].
    pub fn start(lap: Arc<Lap>, feed: &mut LiveFeed, now: Instant) -> Self {
        let mut driver = SimulatedDriver::new(lap, Self::SEED);
        feed.clear();
        let step = 1.0 / 60.0;
        let limit = (feed.track_length() / 20.0 / step) as usize; // 20 m/s would be slow
        for _ in 0..limit {
            feed.push(&driver.step(step));
            if feed.distance() >= Self::START_M.min(0.7 * feed.track_length()) {
                break;
            }
        }
        Self { driver, last: now, carry: 0.0 }
    }

    /// Steps the driver up to `now` at `hz` and pushes the frames into `feed`.
    pub fn advance(&mut self, now: Instant, hz: u32, feed: &mut LiveFeed) {
        let step = 1.0 / f64::from(hz.max(1));
        self.carry = (self.carry + now.saturating_duration_since(self.last).as_secs_f64()).min(Self::MAX_CATCH_UP_S);
        self.last = now;
        while self.carry >= step {
            feed.push(&self.driver.step(step));
            self.carry -= step;
        }
    }

    /// Delay to pass to `request_repaint_after` for the next frame at `hz`: egui takes
    /// one predicted frame (1/60 s) off the requested delay.
    pub fn repaint_delay(hz: u32) -> Duration {
        Duration::from_secs_f64(1.0 / f64::from(hz.max(1)) + 1.0 / 60.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::sample_lap;

    fn frame(t: f64, pct: f64, brake: f32) -> TelemetryFrame {
        TelemetryFrame { session_time: t, lap_dist_pct: pct, throttle: 1.0 - brake, brake, on_track: true }
    }

    #[test]
    fn frames_build_a_continuous_trace() {
        let mut feed = LiveFeed::default();
        feed.set_track_length(4000.0);
        feed.push(&frame(10.0, 0.9990, 0.0));
        feed.push(&frame(10.0 + 1.0 / 60.0, 0.9998, 0.5));
        feed.push(&frame(10.0 + 2.0 / 60.0, 0.0006, 0.6));
        assert_eq!(feed.trace().len(), 3);
        let now = feed.now().unwrap();
        assert!((now.lap_pos - 1.0006).abs() < 1e-9);
        assert!((feed.trace().last().unwrap().d - 1.0006 * 4000.0).abs() < 1e-6);
        assert_eq!(now.brake, 0.6);
    }

    #[test]
    fn a_jump_or_leaving_the_track_restarts_the_trace() {
        let mut feed = LiveFeed::default();
        feed.push(&frame(1.0, 0.10, 0.0));
        feed.push(&frame(1.0 + 1.0 / 60.0, 0.1001, 0.0));
        feed.push(&frame(1.0 + 2.0 / 60.0, 0.60, 0.0)); // tow
        assert_eq!(feed.trace().len(), 1);

        feed.push(&TelemetryFrame { on_track: false, ..frame(2.0, -1.0, 0.0) });
        assert_eq!(feed.trace().len(), 1, "off-track frames are skipped");
        feed.push(&frame(30.0, 0.20, 0.0));
        assert_eq!(feed.trace().len(), 1, "back on track starts over");
        assert!((feed.now().unwrap().lap_pos - 0.20).abs() < 1e-12);
    }

    #[test]
    fn changing_track_length_clears() {
        let mut feed = LiveFeed::default();
        feed.push(&frame(1.0, 0.1, 0.0));
        feed.set_track_length(FALLBACK_TRACK_LENGTH_M);
        assert_eq!(feed.trace().len(), 1, "same length keeps the trace");
        feed.set_track_length(5796.1);
        assert!(feed.trace().is_empty() && feed.now().is_none());
    }

    #[test]
    fn history_is_pruned_beyond_both_limits() {
        let mut feed = LiveFeed::default();
        // 60 s at 100 m/s: 25 s is 2500 m, so the time limit is the wider one.
        for i in 0..3600 {
            let t = f64::from(i) / 60.0;
            feed.push(&frame(t, (t * 100.0 / FALLBACK_TRACK_LENGTH_M) % 1.0, 0.0));
        }
        let first = feed.trace().samples()[0].t;
        assert!((60.0 - KEEP_S - 0.1..60.0 - KEEP_S + 0.1).contains(&first), "{first}");
    }

    #[test]
    fn a_car_stopped_on_track_keeps_the_trace_bounded() {
        let mut feed = LiveFeed::default();
        let (mut t, mut pct) = (0.0, 0.1);
        for _ in 0..600 {
            feed.push(&frame(t, pct, 0.0)); // 10 s at 30 m/s
            t += 1.0 / 60.0;
            pct += 0.5 / FALLBACK_TRACK_LENGTH_M;
        }
        let driven = feed.trace().len();
        for _ in 0..10 * 60 * 60 {
            feed.push(&frame(t, pct, 0.4)); // 10 minutes on the grid, foot on the brake
            t += 1.0 / 60.0;
        }
        assert_eq!(feed.trace().len(), driven + 2, "the stop's first and latest frame");
        assert!((feed.trace().last().unwrap().t - (t - 1.0 / 60.0)).abs() < 1e-9);
        for i in 0..10 * 60 * 60 {
            feed.push(&frame(t, pct, (i % 5) as f32 / 10.0)); // 10 more, pumping the brake
            t += 1.0 / 60.0;
        }
        assert_eq!(feed.trace().len(), crate::trace::MAX_SAMPLES);
    }

    #[test]
    fn demo_prerolls_and_follows_the_clock() {
        let lap = Arc::new(sample_lap());
        let mut feed = LiveFeed::default();
        feed.set_track_length(5796.1);
        let t0 = Instant::now();
        let mut demo = Demo::start(Arc::clone(&lap), &mut feed, t0);
        let d = feed.distance();
        assert!((3800.0..3900.0).contains(&d), "{d}");
        let history = d - feed.trace().samples()[0].d;
        assert!(history >= 500.0, "{history}");

        let session_time = |feed: &LiveFeed| feed.now().unwrap().t;
        let t = session_time(&feed);
        for k in 1..=10 {
            demo.advance(t0 + Duration::from_millis(100 * k), 60, &mut feed);
        }
        let stepped = session_time(&feed) - t;
        assert!((stepped - 1.0).abs() <= 1.0 / 60.0 + 1e-9, "{stepped}");

        let t = session_time(&feed);
        demo.advance(t0 + Duration::from_secs(61), 30, &mut feed);
        let stepped = session_time(&feed) - t;
        assert!((stepped - 0.5).abs() < 1e-9, "a stall catches up at most half a second: {stepped}");
    }

    #[test]
    fn repaint_delay_allows_for_egui_frame_prediction() {
        assert_eq!(Demo::repaint_delay(60).as_millis(), 33);
        assert_eq!(Demo::repaint_delay(30).as_millis(), 50);
    }
}
