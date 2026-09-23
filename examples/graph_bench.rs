//! Measures what the live graph costs per frame: the trace is fed the way the app feeds
//! it (`LiveFeed::push`), then the graph is painted in a headless egui pass and
//! tessellated, as the overlay does on every repaint.
//!
//! ```text
//! cargo run --release --example graph_bench
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Pos2, Rect, vec2};
use ito::demo::{SimulatedDriver, demo_session, sample_lap};
use ito::lap::Lap;
use ito::settings::{Axis, LabelMode};
use ito::trace::{DistanceTracker, LiveSample, LiveTrace, Progress};
use ito::ui::graph::{self, CarNow, GraphScene, LabelOptions};
use ito::ui::theme;

const HZ: f64 = 60.0;
/// The widest history either axis can show (`app::live`).
const KEEP_S: f64 = 25.0;
const KEEP_M: f64 = 1600.0;
/// Frames painted per case; the median is reported.
const RUNS: usize = 15;

/// A trace fed like `LiveFeed::push`.
struct Feed {
    tracker: DistanceTracker,
    trace: LiveTrace,
    now: Option<CarNow>,
    track_length: f64,
}

impl Feed {
    fn new(track_length: f64) -> Self {
        Self { tracker: DistanceTracker::new(), trace: LiveTrace::new(), now: None, track_length }
    }

    fn push(&mut self, t: f64, pct: f64, throttle: f32, brake: f32) {
        let lap_pos = match self.tracker.update(t, pct) {
            Progress::Invalid => return,
            Progress::Reset(p) => {
                self.trace.clear();
                p
            }
            Progress::Continuous(p) => p,
        };
        let d = lap_pos * self.track_length;
        self.trace.push(LiveSample { t, d, brake, throttle });
        self.trace.prune(t - KEEP_S, d - KEEP_M);
        self.now = Some(CarNow { t, lap_pos, throttle, brake });
    }
}

/// Drives the sample lap for `secs`, then (optionally) sits still for `stop_secs` with the
/// pedals held (`noisy`: blipping the throttle and pumping the brake).
fn scenario(lap: &Arc<Lap>, track_length: f64, secs: f64, stop_secs: f64, noisy: bool) -> Feed {
    let mut feed = Feed::new(track_length);
    let mut driver = SimulatedDriver::new(Arc::clone(lap), 11);
    let mut last = None;
    for _ in 0..(secs * HZ) as usize {
        let f = driver.step(1.0 / HZ);
        feed.push(f.session_time, f.lap_dist_pct, f.throttle, f.brake);
        last = Some(f);
    }
    let Some(stop) = last else { return feed };
    for i in 1..=(stop_secs * HZ) as usize {
        let t = stop.session_time + i as f64 / HZ;
        let (throttle, brake) = if noisy {
            let k = i as f64 / HZ;
            ((0.3 + 0.3 * (k * 7.0).sin()) as f32, (0.5 + 0.5 * (k * 3.1).sin()).max(0.0) as f32)
        } else {
            (0.0, 1.0)
        };
        feed.push(t, stop.lap_dist_pct, throttle, brake);
    }
    feed
}

/// A car crawling at `speed` m/s with the pedals moving (e.g. 1500 m of history on pit road).
fn constant_speed(track_length: f64, speed: f64, secs: f64) -> Feed {
    let mut feed = Feed::new(track_length);
    for i in 0..(secs * HZ) as usize {
        let t = i as f64 / HZ;
        let pct = (t * speed / track_length).rem_euclid(1.0);
        feed.push(t, pct, (0.5 + 0.5 * (t * 0.9).sin()) as f32, (0.5 * (t * 1.3).cos()).max(0.0) as f32);
    }
    feed
}

struct Result {
    samples: usize,
    paint: Duration,
    tessellate: Duration,
    vertices: usize,
}

fn measure(ctx: &egui::Context, feed: &Feed, reference: &Lap, axis: Axis, behind: f64, ahead: f64) -> Result {
    let panel = Rect::from_min_size(Pos2::new(1.0, 1.0), vec2(680.0, 170.0));
    let scene = GraphScene {
        axis,
        behind,
        ahead,
        track_length: feed.track_length,
        now: feed.now,
        live: &feed.trace,
        reference: Some(reference),
        ref_opacity: 0.9,
        labels: LabelOptions { show: true, mode: LabelMode::Both, min: 0.1 },
        header_height: 26.0,
        obstacles: &[],
        message: None,
    };
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(700.0, 200.0))),
        ..Default::default()
    };
    let (mut paint, mut tess, mut vertices) = (Vec::new(), Vec::new(), 0);
    for _ in 0..RUNS {
        let start = Instant::now();
        let mut out = ctx.run_ui(input(), |ui| graph::paint(ui.painter(), panel, &scene));
        paint.push(start.elapsed());
        out.textures_delta.clear();
        let start = Instant::now();
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        tess.push(start.elapsed());
        vertices = prims
            .iter()
            .map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                egui::epaint::Primitive::Callback(_) => 0,
            })
            .sum();
    }
    paint.sort();
    tess.sort();
    Result { samples: feed.trace.len(), paint: paint[RUNS / 2], tessellate: tess[RUNS / 2], vertices }
}

fn main() {
    let lap = Arc::new(sample_lap());
    let track_length = demo_session(&lap).track_length_m.unwrap_or(5796.1);
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    // Fonts load on the first pass.
    let _ = ctx.run_ui(egui::RawInput::default(), |_| {});

    let cases: Vec<(&str, Feed)> = vec![
        ("driving", scenario(&lap, track_length, 70.0, 0.0, false)),
        ("crawl 12 m/s, 100 s", constant_speed(track_length, 12.0, 100.0)),
        ("stopped 2 min", scenario(&lap, track_length, 70.0, 120.0, false)),
        ("stopped 10 min", scenario(&lap, track_length, 70.0, 600.0, false)),
        ("stopped 10 min, pedals moving", scenario(&lap, track_length, 70.0, 600.0, true)),
    ];
    println!("{:<32} {:>6} {:<22} {:>9} {:>11} {:>9}", "case", "trace", "axis", "paint", "tessellate", "vertices");
    for (name, feed) in &cases {
        for (axis, behind, ahead, label) in [
            (Axis::Distance, 500.0, 500.0, "distance 500/500 m"),
            (Axis::Distance, 1500.0, 500.0, "distance 1500/500 m"),
            (Axis::Time, 8.0, 6.0, "time 8/6 s"),
        ] {
            let r = measure(&ctx, feed, &lap, axis, behind, ahead);
            println!(
                "{name:<32} {:>6} {label:<22} {:>7.2}ms {:>9.2}ms {:>9}",
                r.samples,
                r.paint.as_secs_f64() * 1000.0,
                r.tessellate.as_secs_f64() * 1000.0,
                r.vertices
            );
        }
    }
}
