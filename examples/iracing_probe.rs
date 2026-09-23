//! Prints what [`IracingReader`] delivers: connection and session events as they arrive
//! and one line per second with the frame count, value ranges and wake-ups. Run it next to
//! iRacing or `cargo run --example fake_iracing`.
//!
//! ```text
//! cargo run --example iracing_probe -- [seconds (default 30)] [wake divisor (default 1)]
//! ```
//!
//! Event times are Unix seconds, to line them up with when a sim was started or killed.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use ito::telemetry::iracing::IracingReader;
use ito::telemetry::{TelemetryEvent, TelemetryFrame};

fn main() {
    let mut args = std::env::args().skip(1);
    let secs: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(30.0);
    let divisor: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1);

    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    let mut reader = IracingReader::spawn(Box::new(move || {
        counter.fetch_add(1, Ordering::Relaxed);
    }));
    reader.set_wake_divisor(divisor);
    println!("{:.3}  probe started, {secs} s, wake divisor {divisor}", unix_now());

    let start = Instant::now();
    let mut second = Summary::default();
    let mut next_report = 1.0;
    while start.elapsed().as_secs_f64() < secs {
        std::thread::sleep(Duration::from_millis(20));
        for event in reader.drain() {
            match event {
                TelemetryEvent::Frame(f) => second.add(&f),
                other => println!("{:.3}  {other:?}", unix_now()),
            }
        }
        if start.elapsed().as_secs_f64() >= next_report {
            println!("{next_report:>8.0}s  {second}  wakes {}", wakes.swap(0, Ordering::Relaxed));
            second = Summary::default();
            next_report += 1.0;
        }
    }

    let stopping = Instant::now();
    reader.stop();
    println!("{:.3}  stop() returned in {:.1} ms", unix_now(), stopping.elapsed().as_secs_f64() * 1000.0);
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Frames received in one second.
#[derive(Debug, Default)]
struct Summary {
    frames: usize,
    driving: usize,
    throttle: Range,
    brake: Range,
    pct: Range,
    time: Range,
    last_time: Option<f64>,
    /// Largest `SessionTime` step between consecutive frames (1/60 s when none were missed).
    max_step: f64,
}

impl Summary {
    fn add(&mut self, f: &TelemetryFrame) {
        self.frames += 1;
        self.driving += usize::from(f.on_track);
        self.throttle.add(f64::from(f.throttle));
        self.brake.add(f64::from(f.brake));
        self.pct.add(f.lap_dist_pct);
        self.time.add(f.session_time);
        if let Some(last) = self.last_time.replace(f.session_time) {
            self.max_step = self.max_step.max(f.session_time - last);
        }
    }
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.frames == 0 {
            return write!(f, "no frames");
        }
        write!(
            f,
            "{:3} frames ({:3} driving)  throttle {}  brake {}  pct {}  session time {}  max step {:.3} s",
            self.frames, self.driving, self.throttle, self.brake, self.pct, self.time, self.max_step
        )
    }
}

/// Smallest and largest value seen.
#[derive(Debug, Default)]
struct Range(Option<(f64, f64)>);

impl Range {
    fn add(&mut self, v: f64) {
        let (lo, hi) = self.0.unwrap_or((v, v));
        self.0 = Some((lo.min(v), hi.max(v)));
    }
}

impl std::fmt::Display for Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some((lo, hi)) => write!(f, "{lo:.3}..{hi:.3}"),
            None => write!(f, "-"),
        }
    }
}
