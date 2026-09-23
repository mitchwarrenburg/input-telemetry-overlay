//! Renders the settings panel in each tab and state, saves a PNG of each, then exits.
//!
//! `cargo run --example settings_preview -- [out_dir] [--zoom <factor>]`
//! (default out_dir: `<temp>/ito-settings-preview`).

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use eframe::egui::{self, ColorImage, Event, Pos2, UserData, ViewportCommand, pos2, vec2};
use ito::demo;
use ito::lap::Lap;
use ito::library::{Library, LibraryEntry};
use ito::matching::{self, RefInfo};
use ito::settings::{Axis, Settings, SettingsTab};
use ito::telemetry::SessionInfo;
use ito::ui::settings_panel::{self, ConnectionState, PanelContext, RefCard};
use ito::ui::theme;

/// The prototype page's background, behind the panel's rounded corners.
const BACKDROP: [f32; 4] = [11.0 / 255.0, 15.0 / 255.0, 18.0 / 255.0, 1.0];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Laps {
    /// No saved laps, no reference.
    None,
    /// Three saved laps; the matching one is the reference.
    Saved,
    /// Nothing saved; the bundled sample is the reference.
    Bundled,
    /// Enough saved laps to scroll.
    Many,
}

struct Variant {
    name: &'static str,
    settings: Settings,
    laps: Laps,
    /// Session at Spa instead of Silverstone.
    at_spa: bool,
    error: Option<&'static str>,
    file_hover: bool,
    connection: ConnectionState,
    /// Simulated pointer position, points.
    pointer: Option<Pos2>,
    /// Click once at `pointer` before the screenshot.
    click: bool,
    /// Tab key presses before the screenshot (keyboard focus).
    tabs: u32,
    /// Tallest the window may be (a short monitor); the panel scrolls past it.
    max_height: f32,
}

impl Variant {
    fn new(name: &'static str, tab: SettingsTab) -> Self {
        Self {
            name,
            settings: Settings { tab, ..Default::default() },
            laps: Laps::Saved,
            at_spa: false,
            error: None,
            file_hover: false,
            connection: ConnectionState::Demo,
            pointer: None,
            click: false,
            tabs: 0,
            max_height: f32::INFINITY,
        }
    }

    fn tabbing(mut self, presses: u32) -> Self {
        self.tabs = presses;
        self
    }

    fn hovering(mut self, x: f32, y: f32) -> Self {
        self.pointer = Some(pos2(x, y));
        self
    }

    fn clicking(mut self, x: f32, y: f32) -> Self {
        self.click = true;
        self.hovering(x, y)
    }
}

fn variants() -> Vec<Variant> {
    let mut labels_off = Variant::new("labels-off", SettingsTab::Labels);
    labels_off.settings.labels = false;
    let mut time_axis = Variant::new("timing-time", SettingsTab::Timing);
    time_axis.settings.axis = Axis::Time;
    time_axis.settings.ahead_s = 0.0;
    time_axis.settings.update_hz = 30;
    let mut error = Variant::new("reference-error", SettingsTab::Reference);
    error.error = Some("Missing column: Brake. Export the lap from Garage 61 as CSV.");
    error.connection = ConnectionState::Live;
    let mut bundled = Variant::new("reference-bundled", SettingsTab::Reference);
    bundled.laps = Laps::Bundled;
    bundled.at_spa = true;
    bundled.connection = ConnectionState::Live;
    let mut empty = Variant::new("reference-empty", SettingsTab::Reference);
    empty.laps = Laps::None;
    empty.file_hover = true;
    empty.connection = ConnectionState::Waiting;
    let mut many = Variant::new("reference-many", SettingsTab::Reference);
    many.laps = Laps::Many;
    let mut short = Variant::new("reference-short-monitor", SettingsTab::Reference);
    short.laps = Laps::Many;
    short.error = error.error;
    short.max_height = 520.0;
    vec![
        Variant::new("display", SettingsTab::Display),
        // Over the Background slider's thumb.
        Variant::new("display-hover", SettingsTab::Display).hovering(230.0, 141.0),
        // Close, four tabs, then the first slider.
        Variant::new("display-focus", SettingsTab::Display).tabbing(6),
        Variant::new("labels", SettingsTab::Labels),
        labels_off,
        Variant::new("timing", SettingsTab::Timing),
        time_axis,
        Variant::new("reference", SettingsTab::Reference),
        error,
        bundled,
        empty,
        many,
        short,
        // The second saved lap's ×. Last: its pending "Remove?" would carry over.
        Variant::new("reference-confirm-remove", SettingsTab::Reference).clicking(272.0, 322.0),
    ]
}

/// A track as Garage 61 names it, its lap length and where the lap starts.
struct Track {
    name: &'static str,
    length_m: f64,
    start: (f64, f64),
}

const SILVERSTONE_GP: Track =
    Track { name: "Silverstone Circuit (Grand Prix)", length_m: 5786.4, start: (52.0683531, -1.0235284) };
const SILVERSTONE_NATIONAL: Track =
    Track { name: "Silverstone Circuit (National)", length_m: 2638.0, start: SILVERSTONE_GP.start };
const SPA: Track =
    Track { name: "Circuit de Spa-Francorchamps (Grand Prix Pits)", length_m: 6990.0, start: (50.4372, 5.9714) };
const MONZA: Track =
    Track { name: "Autodromo Nazionale Monza (Grand Prix)", length_m: 5793.0, start: (45.6156, 9.2811) };

fn entry(id: &str, driver: &str, car: &str, track: &Track, lap_time: f64, last_used: u64) -> LibraryEntry {
    LibraryEntry {
        id: id.into(),
        file: format!("{id}.csv"),
        original_name: format!("Garage 61 - {driver} - {car} - {}.csv", track.name),
        driver: Some(driver.into()),
        car: Some(car.into()),
        track: Some(track.name.into()),
        lap_time,
        samples: (lap_time * 60.0) as usize,
        length_m: Some(track.length_m),
        start_latlon: Some(track.start),
        added: last_used,
        last_used,
    }
}

fn saved_laps() -> Vec<LibraryEntry> {
    vec![
        entry("ferrari", "Jonas Lindqvist", "Ferrari 296 GT3", &SILVERSTONE_GP, 115.992, 30),
        entry("porsche", "Sam Okafor", "Porsche 911 GT3 R (992)", &SILVERSTONE_GP, 116.874, 20),
        entry("spa", "Priya Nair", "Ferrari 296 GT3", &SPA, 137.412, 10),
    ]
}

fn many_laps() -> Vec<LibraryEntry> {
    let mut laps = saved_laps();
    laps.extend([
        entry("national", "Jonas Lindqvist", "Ferrari 296 GT3", &SILVERSTONE_NATIONAL, 58.310, 9),
        entry("bmw", "Theodora Castellanos-Whitfield", "BMW M4 GT3", &SILVERSTONE_GP, 117.201, 8),
        entry("monza", "Sam Okafor", "Ferrari 296 GT3", &MONZA, 107.655, 7),
    ]);
    laps
}

fn spa_session(car: Option<String>) -> SessionInfo {
    SessionInfo {
        track_display_name: Some("Circuit de Spa-Francorchamps".into()),
        track_config_name: Some("Grand Prix Pits".into()),
        track_length_m: Some(6995.0),
        track_latlon: Some(SPA.start),
        car_short_name: car.clone(),
        car_name: car,
        ..Default::default()
    }
}

struct Preview {
    variants: Vec<Variant>,
    index: usize,
    /// Frames drawn at the variant's final size.
    settled: u32,
    clicked: bool,
    shot_requested: bool,
    out_dir: PathBuf,
    lap: Lap,
    silverstone: SessionInfo,
    spa: SessionInfo,
    saved: Library,
    many: Library,
    empty: Library,
}

impl Preview {
    fn new(out_dir: PathBuf) -> Self {
        let lap = demo::sample_lap();
        let silverstone = demo::demo_session(&lap);
        let spa = spa_session(lap.meta.car.clone());
        Self {
            variants: variants(),
            index: 0,
            settled: 0,
            clicked: false,
            shot_requested: false,
            out_dir,
            lap,
            silverstone,
            spa,
            saved: Library { laps: saved_laps(), active: Some("ferrari".into()) },
            many: Library { laps: many_laps(), active: Some("ferrari".into()) },
            empty: Library::default(),
        }
    }

    /// Saves a delivered screenshot and moves to the next variant.
    fn take_screenshots(&mut self, ctx: &egui::Context) {
        let shots: Vec<_> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    Event::Screenshot { image, user_data, .. } => {
                        let index = user_data.data.as_ref()?.downcast_ref::<usize>().copied()?;
                        Some((index, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (index, image) in shots {
            let Some(variant) = self.variants.get(index) else { continue };
            let path = self.out_dir.join(format!("settings-{}.png", variant.name));
            match save_png(&path, &image) {
                Ok(()) => println!("{}", path.display()),
                Err(e) => eprintln!("{}: {e}", path.display()),
            }
            self.index = index + 1;
            self.settled = 0;
            self.clicked = false;
            self.shot_requested = false;
        }
    }
}

fn press(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

impl eframe::App for Preview {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.take_screenshots(&ctx);
        let Some(v) = self.variants.get_mut(self.index) else {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        };
        let session = if v.at_spa { &self.spa } else { &self.silverstone };
        let (library, active_id) = match v.laps {
            Laps::None | Laps::Bundled => (&self.empty, None),
            Laps::Saved => (&self.saved, Some("ferrari")),
            Laps::Many => (&self.many, Some("ferrari")),
        };
        let status = matching::status(&RefInfo::from_lap(&self.lap), Some(session));
        let card = match v.laps {
            Laps::None => None,
            Laps::Bundled => Some(RefCard::Bundled(&self.lap, status)),
            Laps::Saved | Laps::Many => Some(RefCard::Saved(&self.lap, status)),
        };
        let cx = PanelContext {
            library,
            session: Some(session),
            card,
            active_id,
            error: v.error,
            hotkey_error: None,
            connection: v.connection,
            browsing: false,
            file_hover: v.file_hover,
            max_height: v.max_height,
        };
        let out = settings_panel::show(ui, &mut v.settings, &cx);

        let want = vec2(settings_panel::WIDTH, out.desired_height.min(v.max_height));
        let have = ctx.content_rect().size();
        if (want - have).length() > 0.5 {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(want));
            self.settled = 0;
        } else {
            self.settled += 1;
            if self.settled >= 3 + v.tabs && !self.shot_requested {
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(self.index)));
                self.shot_requested = true;
            }
        }
        ctx.request_repaint();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BACKDROP
    }

    /// Replaces the real mouse with the variant's simulated pointer.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        raw_input
            .events
            .retain(|e| !matches!(e, Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone));
        let Some(variant) = self.variants.get(self.index) else { return };
        if (1..=variant.tabs).contains(&self.settled) {
            raw_input.events.push(Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            });
        }
        let Some(pos) = variant.pointer else {
            raw_input.events.push(Event::PointerGone);
            return;
        };
        raw_input.events.push(Event::PointerMoved(pos));
        if variant.click && self.settled == 1 && !self.clicked {
            raw_input.events.extend([press(pos, true), press(pos, false)]);
            self.clicked = true;
        }
    }
}

fn save_png(path: &Path, image: &ColorImage) -> Result<(), Box<dyn std::error::Error>> {
    let [w, h] = image.size;
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), u32::try_from(w)?, u32::try_from(h)?);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(image.as_raw())?;
    Ok(())
}

fn main() -> eframe::Result {
    let mut out_dir = std::env::temp_dir().join("ito-settings-preview");
    let mut zoom = 1.0;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--zoom" => zoom = args.next().and_then(|z| z.parse().ok()).unwrap_or(zoom),
            _ => out_dir = PathBuf::from(arg),
        }
    }
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("{}: {e}", out_dir.display());
    }

    let viewport = egui::ViewportBuilder::default()
        .with_title("Settings preview")
        .with_inner_size([settings_panel::WIDTH, 400.0])
        .with_position([60.0, 60.0])
        .with_decorations(false)
        .with_resizable(false);
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    eframe::run_native(
        "settings_preview",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            cc.egui_ctx.set_zoom_factor(zoom);
            Ok(Box::new(Preview::new(out_dir)))
        }),
    )
}
