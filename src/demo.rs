//! Demo mode: a simulated driver used when iRacing isn't running, so the overlay can
//! be positioned and tried out. It replays a lap's position and varies the inputs per
//! brake zone, so the live lines differ from the reference like a real attempt.

use std::sync::Arc;

use crate::lap::{Lap, parse_garage61_csv};
use crate::telemetry::{SessionInfo, TelemetryFrame};

const SAMPLE_CSV: &str = include_str!("../assets/sample-laps/silverstone-gp-ferrari-296-gt3.csv");
const SAMPLE_NAME: &str =
    "Garage 61 - Sample lap - Ferrari 296 GT3 - Silverstone Circuit (Grand Prix) - 01.55.992 - SAMPLE.csv";

/// The bundled lap (Ferrari 296 GT3, Silverstone GP).
pub fn sample_lap() -> Lap {
    parse_garage61_csv(SAMPLE_CSV, SAMPLE_NAME).expect("bundled sample lap parses")
}

/// The session the demo pretends to be in.
pub fn demo_session(lap: &Lap) -> SessionInfo {
    SessionInfo {
        track_display_name: Some("Silverstone Circuit".into()),
        track_config_name: Some("Arena Grand Prix".into()),
        track_name: Some("silverstone 2019 gp".into()),
        track_length_m: Some(5796.1),
        track_latlon: Some((52.068299, -1.023457)),
        car_name: lap.meta.car.clone(),
        car_short_name: lap.meta.car.clone(),
    }
}

/// Small deterministic PRNG (mulberry32), so demo laps are reproducible.
#[derive(Debug, Clone)]
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(1 | t);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
}

fn sample_at(arr: &[f32], f: f64) -> f32 {
    let n = arr.len();
    if f <= 0.0 {
        return arr[0];
    }
    if f >= (n - 1) as f64 {
        return arr[n - 1];
    }
    let i = f.floor() as usize;
    let k = (f - i as f64) as f32;
    arr[i] + (arr[i + 1] - arr[i]) * k
}

pub struct SimulatedDriver {
    lap: Arc<Lap>,
    rng: Rng,
    t: f64,
    lap_start: f64,
    lap_index: u32,
    brake: Vec<f32>,
    throttle: Vec<f32>,
}

impl SimulatedDriver {
    pub fn new(lap: Arc<Lap>, seed: u32) -> Self {
        let mut d =
            Self { lap, rng: Rng(seed), t: 0.0, lap_start: 0.0, lap_index: 0, brake: Vec::new(), throttle: Vec::new() };
        d.vary();
        d
    }

    pub fn lap(&self) -> &Arc<Lap> {
        &self.lap
    }

    /// Completed laps.
    pub fn lap_index(&self) -> u32 {
        self.lap_index
    }

    /// Seconds into the current lap.
    pub fn lap_clock(&self) -> f64 {
        self.t - self.lap_start
    }

    /// New set of inputs for the coming lap.
    fn vary(&mut self) {
        let lap = Arc::clone(&self.lap);
        let n = lap.n();
        let rng = &mut self.rng;
        let mut brake = vec![0.0f32; n];
        let mut throttle = lap.throttle.clone();
        let zones = &lap.zones;
        for (k, z) in zones.iter().enumerate() {
            let lo = if k > 0 { (zones[k - 1].end + z.start) / 2 } else { 0 };
            let hi = if k + 1 < zones.len() { (z.end + zones[k + 1].start) / 2 } else { n - 1 };
            let shift = rng.range(-9.0, 7.0); // samples: brake earlier (−) or later (+)
            let stretch = rng.range(0.9, 1.15); // longer or shorter trail
            let scale = rng.range(0.88, 1.3) as f32; // softer or harder pedal
            let spike = if rng.next() < 0.5 { rng.range(0.04, 0.12) as f32 } else { 0.0 };
            let spike_at = z.start as f64 + shift + 7.0;
            let lag = rng.range(-3.0, 10.0); // throttle pickup earlier (−) or later (+)
            for j in lo..=hi {
                let jf = j as f64;
                let src = z.start as f64 + (jf - shift - z.start as f64) / stretch;
                let mut b = sample_at(&lap.brake, src) * scale;
                if spike > 0.0 {
                    b += spike * (-((jf - spike_at) / 5.0).powi(2)).exp() as f32;
                }
                brake[j] = b.clamp(0.0, 1.0);
                throttle[j] = if brake[j] > 0.05 { 0.0 } else { sample_at(&lap.throttle, jf - lag) };
            }
        }
        // Now and then, a lift on a straight.
        if n > 200 && rng.next() < 0.6 {
            for _ in 0..30 {
                let c = rng.range(0.0, (n - 120) as f64) as usize;
                if throttle[c..c + 90].iter().any(|&v| v < 0.99) {
                    continue;
                }
                let depth = rng.range(0.2, 0.45) as f32;
                for (j, v) in throttle[c..c + 90].iter_mut().enumerate() {
                    let g = (-(((j as f64 - 45.0) / 12.0).powi(2))).exp() as f32;
                    *v = v.min(1.0 - depth * g);
                }
                break;
            }
        }
        self.brake = brake;
        self.throttle = throttle;
    }

    /// Advances by `dt` seconds and returns one telemetry frame.
    pub fn step(&mut self, dt: f64) -> TelemetryFrame {
        self.t += dt;
        let lap_time = self.lap.lap_time;
        let mut tl = self.t - self.lap_start;
        while tl >= lap_time {
            self.lap_start += lap_time;
            tl -= lap_time;
            self.lap_index += 1;
            self.vary();
        }
        let lap = &self.lap;
        let n = lap.n();
        let f = tl * lap.hz;
        let i = (f.floor() as usize).min(n - 1);
        let p0 = lap.pct[i];
        let p1 = if i + 1 < n { lap.pct[i + 1] } else { lap.pct[0] + 1.0 };
        let pct_run = p0 + (p1 - p0) * (f - i as f64);
        TelemetryFrame {
            session_time: self.t,
            lap_dist_pct: pct_run.rem_euclid(1.0),
            throttle: sample_at(&self.throttle, f),
            brake: sample_at(&self.brake, f),
            on_track: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{DistanceTracker, Progress};

    #[test]
    fn sample_lap_and_session_line_up() {
        let lap = sample_lap();
        let s = demo_session(&lap);
        assert_eq!(
            crate::matching::status(&crate::matching::RefInfo::from_lap(&lap), Some(&s)),
            crate::matching::MatchStatus::Match
        );
    }

    #[test]
    fn demo_drives_continuously_over_several_laps() {
        let lap = Arc::new(sample_lap());
        let mut d = SimulatedDriver::new(Arc::clone(&lap), 11);
        let mut tracker = DistanceTracker::new();
        let mut resets = 0;
        let mut last = 0.0;
        let mut braked = false;
        for _ in 0..(60.0 * lap.lap_time * 2.2) as usize {
            let f = d.step(1.0 / 60.0);
            assert!((0.0..=1.0).contains(&f.throttle) && (0.0..=1.0).contains(&f.brake));
            braked |= f.brake > 0.5;
            match tracker.update(f.session_time, f.lap_dist_pct) {
                Progress::Reset(p) => {
                    resets += 1;
                    last = p;
                }
                Progress::Continuous(p) => {
                    assert!(p >= last - 1e-9, "went backwards: {p} < {last}");
                    last = p;
                }
                Progress::Invalid => panic!("demo frames are always on track"),
            }
        }
        assert_eq!(resets, 1, "only the first frame starts a trace");
        assert!(last > 2.0 && d.lap_index() == 2);
        assert!(braked);
    }
}
