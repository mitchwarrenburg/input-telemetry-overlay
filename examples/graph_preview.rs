//! Renders the throttle/brake graph the way the overlay will: the bundled sample lap as
//! the reference and the simulated driver as live input, pre-rolled to 3800 m.
//!
//! ```text
//! cargo run --example graph_preview                  # animated, distance and time axis
//! cargo run --example graph_preview -- --shots DIR   # saves PNGs of each case, then exits
//! ```

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, ColorImage, CornerRadius, Event, Rect, Stroke, StrokeKind, Vec2, pos2, vec2,
};
use ito::demo::{SimulatedDriver, demo_session, sample_lap};
use ito::lap::Lap;
use ito::settings::{Axis, LabelMode, Settings};
use ito::trace::{DistanceTracker, LiveSample, LiveTrace, Progress};
use ito::ui::graph::{self, CarNow, GraphScene, LabelOptions};
use ito::ui::theme::{self, Weight};

/// The page behind the overlay in the prototype.
const BACKDROP: Color32 = Color32::from_rgb(0x0b, 0x0f, 0x12);
const HEADER_HEIGHT: f32 = 26.0;
const MARGIN: f32 = 10.0;
/// Frames to render a case before capturing it (fonts load, layout settles).
const SETTLE_FRAMES: u32 = 3;
const STEP: f64 = 1.0 / 60.0;

/// The simulated driver feeding a live trace, as the app does with iRacing frames.
struct Sim {
    driver: SimulatedDriver,
    tracker: DistanceTracker,
    trace: LiveTrace,
    now: Option<CarNow>,
    track_length: f64,
}

impl Sim {
    /// Drives until the car is `metres` into the session.
    fn rolled_to(lap: &Arc<Lap>, track_length: f64, metres: f64) -> Self {
        let mut sim = Self {
            driver: SimulatedDriver::new(Arc::clone(lap), 11),
            tracker: DistanceTracker::new(),
            trace: LiveTrace::new(),
            now: None,
            track_length,
        };
        while sim.now.is_none_or(|n| n.lap_pos * track_length < metres) {
            sim.step();
        }
        sim
    }

    fn step(&mut self) {
        let f = self.driver.step(STEP);
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
        // The widest window any setting can show.
        self.trace.prune(f.session_time - 25.0, d - 1600.0);
        self.now = Some(CarNow { t: f.session_time, lap_pos, throttle: f.throttle, brake: f.brake });
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Driving,
    /// Driving just after start/finish.
    StartFinish,
    NoReference,
    Waiting,
}

struct Case {
    name: &'static str,
    size: Vec2,
    axis: Axis,
    state: State,
}

const CASES: [Case; 8] = [
    Case { name: "680x170_distance", size: vec2(680.0, 170.0), axis: Axis::Distance, state: State::Driving },
    Case { name: "680x170_time", size: vec2(680.0, 170.0), axis: Axis::Time, state: State::Driving },
    Case { name: "300x190_distance", size: vec2(300.0, 190.0), axis: Axis::Distance, state: State::Driving },
    Case { name: "300x190_time", size: vec2(300.0, 190.0), axis: Axis::Time, state: State::Driving },
    Case { name: "680x100_distance", size: vec2(680.0, 100.0), axis: Axis::Distance, state: State::Driving },
    Case { name: "680x170_start_finish", size: vec2(680.0, 170.0), axis: Axis::Distance, state: State::StartFinish },
    Case { name: "680x170_no_reference", size: vec2(680.0, 170.0), axis: Axis::Distance, state: State::NoReference },
    Case { name: "680x170_waiting", size: vec2(680.0, 170.0), axis: Axis::Distance, state: State::Waiting },
];

/// Screenshot mode: which case is showing and whether its capture is on the way.
struct Shots {
    dir: PathBuf,
    case: usize,
    frames: u32,
    requested: bool,
}

struct Preview {
    lap: Arc<Lap>,
    driving: Sim,
    start_finish: Sim,
    shots: Option<Shots>,
    /// Live mode: simulated time not yet stepped.
    backlog: f64,
}

impl Preview {
    fn new(shots: Option<PathBuf>) -> Self {
        let lap = Arc::new(sample_lap());
        let track_length = demo_session(&lap).track_length_m.expect("the demo session has a track length");
        Self {
            driving: Sim::rolled_to(&lap, track_length, 3800.0),
            start_finish: Sim::rolled_to(&lap, track_length, track_length + 120.0),
            lap,
            shots: shots.map(|dir| Shots { dir, case: 0, frames: 0, requested: false }),
            backlog: 0.0,
        }
    }

    /// Paints the overlay panel (background, border, a header title) and the graph.
    fn paint_case(&self, painter: &egui::Painter, outer: Rect, axis: Axis, state: State) {
        let bg_opacity = Settings::default().bg_opacity / 100.0;
        let border = Stroke::new(1.0, theme::alpha(theme::BORDER, 0.08 + 0.2 * bg_opacity));
        painter.rect(
            outer,
            CornerRadius::same(8),
            theme::alpha(theme::SURFACE, bg_opacity),
            border,
            StrokeKind::Inside,
        );
        let panel = outer.shrink(1.0);
        let title = painter.text(
            pos2(panel.left() + 11.0, panel.top() + HEADER_HEIGHT / 2.0),
            Align2::LEFT_CENTER,
            "THROTTLE / BRAKE",
            theme::font(Weight::Bold, 11.0),
            theme::HUD_TEXT,
        );

        let sim = if state == State::StartFinish { &self.start_finish } else { &self.driving };
        let settings = Settings { axis, ..Settings::default() };
        let (behind, ahead) = settings.window_span();
        let scene = GraphScene {
            axis,
            behind,
            ahead,
            track_length: sim.track_length,
            now: sim.now.filter(|_| state != State::Waiting),
            live: &sim.trace,
            reference: (state != State::NoReference).then_some(self.lap.as_ref()),
            ref_opacity: settings.ref_opacity / 100.0,
            labels: LabelOptions { show: settings.labels, mode: LabelMode::Both, min: settings.label_min / 100.0 },
            header_height: HEADER_HEIGHT,
            obstacles: &[title.expand(2.0)],
            message: match state {
                State::NoReference => Some(("No reference lap", "Drop a Garage 61 CSV here")),
                State::Waiting => Some(("Waiting for iRacing", "The overlay starts when you're on track")),
                State::Driving | State::StartFinish => None,
            },
        };
        graph::paint(painter, panel, &scene);
    }

    /// Screenshot mode: shows each case, captures it, saves it, moves on, then closes.
    fn shoot(&mut self, ctx: &egui::Context, painter: &egui::Painter) {
        let Some(shots) = &mut self.shots else { return };
        let Some(case) = CASES.get(shots.case) else { return }; // all saved, closing
        let outer = Rect::from_min_size(pos2(MARGIN, MARGIN), case.size);
        let captured = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        if shots.requested
            && let Some(image) = captured
        {
            let path = shots.dir.join(format!("graph_{}.png", case.name));
            match save_png(&image.region(&outer, Some(ctx.pixels_per_point())), &path) {
                Ok(()) => println!("{}", path.display()),
                Err(e) => eprintln!("{}: {e}", path.display()),
            }
            shots.case += 1;
            shots.frames = 0;
            shots.requested = false;
            if shots.case == CASES.len() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        shots.frames += 1;
        if shots.frames == SETTLE_FRAMES {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            shots.requested = true;
        }
        self.paint_case(painter, outer, case.axis, case.state);
    }

    /// Live mode: advances the driver in real time and shows both axes.
    fn animate(&mut self, ctx: &egui::Context, painter: &egui::Painter) {
        self.backlog += f64::from(ctx.input(|i| i.stable_dt)).min(0.5);
        while self.backlog >= STEP {
            self.driving.step();
            self.backlog -= STEP;
        }
        let size = vec2(680.0, 170.0);
        for (row, axis) in [Axis::Distance, Axis::Time].into_iter().enumerate() {
            let outer = Rect::from_min_size(pos2(MARGIN, MARGIN + row as f32 * (size.y + MARGIN)), size);
            self.paint_case(painter, outer, axis, State::Driving);
        }
    }
}

impl eframe::App for Preview {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let painter = ui.painter().clone();
        if self.shots.is_some() {
            self.shoot(&ctx, &painter);
        } else {
            self.animate(&ctx, &painter);
        }
        ctx.request_repaint();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BACKDROP.to_normalized_gamma_f32()
    }
}

fn save_png(image: &ColorImage, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let [w, h] = image.size;
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), u32::try_from(w)?, u32::try_from(h)?);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(image.as_raw())?;
    Ok(())
}

fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let shots = match (args.next().as_deref(), args.next()) {
        (Some("--shots"), Some(dir)) => Some(PathBuf::from(dir)),
        _ => None,
    };
    if let Some(dir) = &shots {
        std::fs::create_dir_all(dir).map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    }
    let size = if shots.is_some() { vec2(700.0, 210.0) } else { vec2(700.0, 370.0) };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Graph preview")
            .with_inner_size(size)
            .with_resizable(false),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "graph_preview",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(Preview::new(shots)))
        }),
    )
}
