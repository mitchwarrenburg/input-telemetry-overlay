//! Renders the brake point window in each of its states, full and compact, over a
//! backdrop like a sim's, and saves them as one PNG.
//!
//! ```text
//! cargo run --example cue_preview -- [out.png]    # default: <temp>/ito-cue-preview.png
//! cargo run --example cue_preview -- --docs DIR   # the README's images, see-through
//! ```

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, ColorImage, Event, Rect, UiBuilder, Vec2, pos2, vec2};
use ito::cue::{CueMode, CueState, FinalPeak, Flash, Grade, Pip, Verdict};
use ito::ui::cue_view::{self, CueFrame};
use ito::ui::overlay::{Chrome, MARGIN};
use ito::ui::theme::{self, Weight};

const COLUMN: f32 = 400.0;
const LABEL_H: f32 = 16.0;
const SETTLE_FRAMES: u32 = 3;

struct Case {
    name: &'static str,
    state: CueState,
    compact: bool,
    /// Background opacity, 0..1.
    bg: f32,
    panel: Vec2,
    pop: f32,
    pulse: f32,
}

impl Case {
    fn new(name: &'static str, state: CueState) -> Self {
        Self { name, state, compact: false, bg: 0.8, panel: cue_view::DEFAULT_PANEL, pop: 1.0, pulse: 0.0 }
    }

    fn compact(mut self) -> Self {
        self.compact = true;
        self.panel = vec2(360.0, cue_view::COMPACT_HEIGHT);
        self
    }

    fn size(mut self, w: f32, h: f32) -> Self {
        self.panel = vec2(w, h);
        self
    }
}

fn pips(current: usize, graded: &[Option<Grade>]) -> Vec<Pip> {
    graded.iter().enumerate().map(|(i, &grade)| Pip { grade, stale: i >= current, current: i == current }).collect()
}

fn verdict(grade: Grade, dt: f64, pending: bool, current: bool, peak: Option<f32>) -> Verdict {
    Verdict { zone_no: 3, grade, dt: Some(dt), dm: Some(dt * 55.0), pending, current, peak, target: 0.82 }
}

fn base(mode: CueMode) -> CueState {
    let graded = [Some(Grade::Good), Some(Grade::Late), None, Some(Grade::Perfect), Some(Grade::Early), None];
    CueState {
        mode,
        beat: 0,
        fill: 0.0,
        join: 0.0,
        zone_no: 3,
        zone_count: 6,
        until_brake: 5.0,
        dist: 240.0,
        target: Some(0.82),
        live: 0.0,
        peak: None,
        flash: None,
        final_peak: None,
        verdict: Some(verdict(Grade::Late, 0.14, false, false, Some(0.79))),
        pips: pips(2, &graded),
        cue_zones: (0..6).collect(),
        marks: Vec::new(),
    }
}

fn counting(beat: u8, fill: f32) -> CueState {
    CueState { beat, fill, until_brake: 1.0, ..base(CueMode::Countdown) }
}

fn cases() -> Vec<Case> {
    let braking = |grade: Grade, dt: f64, alpha: f32, fill: f32| CueState {
        fill,
        live: 0.64,
        peak: Some(0.7),
        flash: (alpha > 0.0).then_some(Flash { grade, alpha }),
        verdict: Some(verdict(grade, dt, false, true, Some(0.7))),
        until_brake: -0.3,
        ..base(CueMode::Braking)
    };
    let mut noref = base(CueMode::NoReference);
    (noref.target, noref.verdict, noref.pips) = (None, None, Vec::new());
    let mut first = base(CueMode::Idle);
    first.verdict = None;
    let final_peak =
        CueState { final_peak: Some(FinalPeak { peak: 0.79, target: 0.82, age: 1.0 }), ..base(CueMode::Idle) };
    vec![
        Case::new("idle", base(CueMode::Idle)),
        Case::new("idle, no timing yet", first),
        Case::new("3", counting(3, 0.2)),
        Case::new("3 (popping in)", counting(3, 0.02)).with_pop(0.3),
        Case::new("2", counting(2, 0.55)),
        Case::new("1", counting(1, 0.9)),
        Case::new("1, joined part-way", CueState { join: 0.4, ..counting(1, 0.8) }),
        Case::new("brake", CueState { fill: 1.0, until_brake: -0.01, ..base(CueMode::Brake) }).with_pulse(1.0),
        Case::new(
            "brake, late (pending)",
            CueState {
                fill: 1.0,
                until_brake: -0.12,
                verdict: Some(verdict(Grade::Late, 0.12, true, true, None)),
                ..base(CueMode::Brake)
            },
        ),
        Case::new("good", braking(Grade::Good, 0.05, 1.0, 1.0)),
        Case::new("perfect", braking(Grade::Perfect, 0.01, 1.0, 1.0)),
        Case::new("early, stops short", braking(Grade::Early, -0.15, 1.0, 0.85)),
        Case::new("braking, after the flash", braking(Grade::Good, 0.05, 0.0, 1.0)),
        Case::new("no reference", noref.clone()),
        Case::new("background 20%", counting(2, 0.55)).bg(0.2),
        Case::new("smallest", counting(1, 0.9)).size(cue_view::MIN_PANEL.x, cue_view::MIN_PANEL.y),
        Case::new("big", braking(Grade::Perfect, 0.0, 1.0, 1.0)).size(560.0, 150.0),
        Case::new("compact: idle", base(CueMode::Idle)).compact(),
        Case::new("compact: 2", counting(2, 0.55)).compact(),
        Case::new("compact: braking", CueState { live: 0.45, ..braking(Grade::Good, 0.05, 1.0, 1.0) }).compact(),
        Case::new("compact: final", final_peak).compact(),
        Case::new("compact: perfect", braking(Grade::Perfect, 0.0, 0.7, 1.0)).compact(),
        Case::new("compact: no reference", noref).compact(),
        Case::new("compact: smallest", counting(1, 0.9))
            .compact()
            .size(cue_view::MIN_COMPACT.x, cue_view::MIN_COMPACT.y),
        Case::new("compact: background 0%", CueState { live: 0.45, ..braking(Grade::Late, 0.2, 0.0, 1.0) })
            .compact()
            .bg(0.0),
    ]
}

impl Case {
    fn with_pop(mut self, pop: f32) -> Self {
        self.pop = pop;
        self
    }

    fn with_pulse(mut self, pulse: f32) -> Self {
        self.pulse = pulse;
        self
    }

    fn bg(mut self, bg: f32) -> Self {
        self.bg = bg;
        self
    }

    fn window(&self) -> Vec2 {
        self.panel + Vec2::splat(2.0 * MARGIN)
    }
}

/// Cases in three columns (seven, seven, the rest), each with its label above.
fn layout(cases: &[Case]) -> (Vec<Rect>, Vec2) {
    let mut rects = Vec::new();
    let mut y = [8.0_f32; 3];
    for (i, case) in cases.iter().enumerate() {
        let col = (i / 7).min(2);
        let size = case.window();
        rects.push(Rect::from_min_size(pos2(8.0 + col as f32 * COLUMN, y[col] + LABEL_H), size));
        y[col] += LABEL_H + size.y + 6.0;
    }
    (rects, vec2(2.0 * COLUMN + 600.0, y.into_iter().fold(0.0, f32::max) + 8.0))
}

/// Something like a sim behind the windows: sky, horizon, track.
fn backdrop(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 0.0, Color32::from_rgb(0x0b, 0x0f, 0x12));
    let band = |y0: f32, y1: f32, color: Color32| {
        painter.rect_filled(Rect::from_x_y_ranges(rect.x_range(), y0..=y1), 0.0, color);
    };
    let mut y = rect.top();
    while y < rect.bottom() {
        band(y, y + 60.0, Color32::from_rgb(120, 168, 214));
        band(y + 60.0, y + 90.0, Color32::from_rgb(88, 120, 70));
        band(y + 90.0, y + 140.0, Color32::from_rgb(70, 72, 76));
        band(y + 112.0, y + 116.0, Color32::from_rgb(235, 235, 235));
        y += 140.0;
    }
}

/// The README's images: the window mid-count, and compact while braking.
fn docs_cases() -> Vec<Case> {
    vec![
        Case::new("brake-point", counting(2, 0.55)),
        Case::new(
            "brake-point-compact",
            CueState {
                fill: 1.0,
                live: 0.45,
                peak: Some(0.47),
                flash: Some(Flash { grade: Grade::Good, alpha: 1.0 }),
                verdict: Some(verdict(Grade::Good, 0.05, false, true, Some(0.47))),
                ..base(CueMode::Braking)
            },
        )
        .compact(),
    ]
}

/// The docs cases one under another, unlabelled.
fn docs_layout(cases: &[Case]) -> (Vec<Rect>, Vec2) {
    let mut y = 0.0;
    let rects: Vec<Rect> = cases
        .iter()
        .map(|c| {
            let r = Rect::from_min_size(pos2(0.0, y), c.window());
            y += c.window().y;
            r
        })
        .collect();
    let w = rects.iter().map(|r| r.width()).fold(0.0, f32::max);
    (rects, vec2(w, y))
}

struct Preview {
    /// The board's PNG, or the folder for the README's images.
    out: PathBuf,
    docs: bool,
    frames: u32,
    requested: bool,
}

impl eframe::App for Preview {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let (cases, rects) = if self.docs {
            let cases = docs_cases();
            let rects = docs_layout(&cases).0;
            (cases, rects)
        } else {
            let cases = cases();
            backdrop(ui.painter(), ui.max_rect());
            let rects = layout(&cases).0;
            (cases, rects)
        };
        for (i, (case, rect)) in cases.iter().zip(&rects).enumerate() {
            if !self.docs {
                ui.painter().text(
                    rect.min + vec2(MARGIN, -2.0),
                    Align2::LEFT_BOTTOM,
                    case.name,
                    theme::font(Weight::SemiBold, 11.0),
                    Color32::WHITE,
                );
            }
            let frame = CueFrame {
                state: &case.state,
                chrome: Chrome {
                    opacity: case.bg,
                    locked: false,
                    settings_open: false,
                    reference_time: None,
                    badges: &[],
                    file_hover: false,
                    resizing: false,
                    pulse: case.pulse,
                },
                compact: case.compact,
                contents: 1.0,
                pop: case.pop,
            };
            ui.scope_builder(UiBuilder::new().id_salt(i).max_rect(*rect), |ui| cue_view::show(ui, *rect, &frame));
        }

        self.frames += 1;
        let ctx = ui.ctx().clone();
        if self.frames < SETTLE_FRAMES {
            ctx.request_repaint();
            return;
        }
        if !self.requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        match shot {
            Some(image) if self.docs => {
                for (case, rect) in cases.iter().zip(&rects) {
                    let path = self.out.join(format!("{}.png", case.name));
                    save(&path, &image.region(rect, Some(ctx.pixels_per_point())));
                    println!("Saved {}", path.display());
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(image) => {
                save(&self.out, &image);
                println!("Saved {}", self.out.display());
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None => ctx.request_repaint(),
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
}

fn save(path: &Path, image: &ColorImage) {
    let file = BufWriter::new(File::create(path).expect("create the PNG"));
    let [w, h] = image.size;
    let mut encoder = png::Encoder::new(file, w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    encoder.write_header().and_then(|mut w| w.write_image_data(&rgba)).expect("write the PNG");
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let docs = args.first().is_some_and(|a| a == "--docs");
    let out = match (docs, args.get(usize::from(docs))) {
        (_, Some(path)) => PathBuf::from(path),
        (true, None) => PathBuf::from("docs/images"),
        (false, None) => std::env::temp_dir().join("ito-cue-preview.png"),
    };
    let size = if docs { docs_layout(&docs_cases()).1 } else { layout(&cases()).1 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Brake point preview")
            .with_inner_size(size)
            .with_transparent(docs)
            .with_resizable(false),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "cue_preview",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            cc.egui_ctx.set_pixels_per_point(1.0);
            Ok(Box::new(Preview { out, docs, frames: 0, requested: false }))
        }),
    )
}
