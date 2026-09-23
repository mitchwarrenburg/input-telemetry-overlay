//! The overlay application (eframe): the transparent overlay window, live and demo
//! input, reference selection, the settings window, tray icon and hotkey.
#![cfg(windows)]

mod capture;
mod geometry;
mod live;
mod reference;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Rect, Vec2, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::monitor::MonitorHandle;
use winit::window::Window;

use crate::demo;
use crate::lap::Lap;
use crate::library::Library;
use crate::platform::{self, Desktop, DesktopEvent};
use crate::settings::{self, Settings, SettingsTab, WindowRect};
use crate::telemetry::iracing::IracingReader;
use crate::telemetry::{SessionInfo, TelemetryEvent};
use crate::ui::graph::{self, GraphScene, LabelOptions};
use crate::ui::overlay::{self, Chrome, HEADER_HEIGHT, Intent};
use crate::ui::settings_panel::{self, ConnectionState, PanelAction, PanelContext, PanelOutput};
use crate::ui::theme;

use capture::Screenshots;
use live::{Demo, LiveFeed};
use reference::Reference;

const TITLE: &str = "Input Telemetry Overlay";
const APP_ID: &str = "input-telemetry-overlay";
/// Settings are written this long after the last change.
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// The size readout stays up this long after the window stops changing size.
const READOUT_HOLD: Duration = Duration::from_millis(800);
/// egui takes one predicted frame off `request_repaint_after` delays; add it back.
const FRAME: Duration = Duration::from_millis(17);
const WAITING: (&str, &str) = ("WAITING FOR IRACING", "Start a session, or turn on demo mode in settings");
const NO_REFERENCE: (&str, &str) = ("NO REFERENCE LAP", "Drop a Garage 61 CSV here, or load one in ⚙ settings");

/// Command-line options.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// Run the simulated driver even if iRacing is running.
    pub demo: bool,
    /// Open the settings window on this tab at start.
    pub open_settings: Option<SettingsTab>,
    /// Save a PNG of the overlay window here after it has rendered, then…
    pub screenshot: Option<PathBuf>,
    /// …a PNG of the settings window here (requires `open_settings`), then exit.
    pub settings_screenshot: Option<PathBuf>,
    /// Use this folder instead of %APPDATA%\input-telemetry-overlay.
    pub data_dir: Option<PathBuf>,
    /// Garage 61 CSVs to add to the library at start (a CSV dropped on the exe, or
    /// "Open with"); the last one becomes the reference.
    pub import: Vec<PathBuf>,
}

impl LaunchOptions {
    /// Where settings, the lap library and the log live.
    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.clone().unwrap_or_else(settings::app_dir)
    }
}

/// Opens the overlay and runs until it's closed. With `instance` (this process holds the
/// data folder), files other launches hand over are imported. Fails when no window can
/// be created (e.g. no OpenGL 2 driver).
pub fn run(opts: LaunchOptions, instance: Option<platform::Instance>) -> eframe::Result {
    let settings = Settings::load(&Paths::new(&opts.data_dir()).settings);
    let options = eframe::NativeOptions {
        viewport: overlay_viewport(&settings),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let result = eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(OverlayApp::new(cc, opts, settings, instance)))),
    );
    if let Err(e) = &result {
        log::error!("The overlay couldn't start: {e}");
    }
    result
}

/// Files in the data folder.
struct Paths {
    settings: PathBuf,
    library: PathBuf,
    laps: PathBuf,
}

impl Paths {
    fn new(dir: &Path) -> Self {
        Self { settings: dir.join("settings.json"), library: Library::index_path(dir), laps: Library::laps_dir(dir) }
    }
}

pub struct OverlayApp {
    paths: Paths,
    /// `--demo`: drive the simulated car and don't read iRacing.
    force_demo: bool,
    settings: Settings,
    /// The settings as last applied; a difference is applied and saved.
    applied: Settings,
    save_due: Option<Instant>,
    library: Library,
    reference: Reference,
    /// The bundled lap the demo drives.
    sample: Arc<Lap>,
    /// The session the graph and reference follow: iRacing's while live, the demo's
    /// while the demo runs.
    session: Option<SessionInfo>,
    /// Last session info from iRacing, which can arrive before `Connected`.
    sim_session: Option<SessionInfo>,
    feed: LiveFeed,
    reader: Option<IracingReader>,
    live: bool,
    demo: Option<Demo>,
    desktop: Desktop,
    /// Last import or save error, shown in the settings window.
    error: Option<String>,
    /// Why the chosen saved lap couldn't be loaded; cleared once a lap loads.
    load_error: Option<String>,
    /// Why the unlock shortcut couldn't be registered.
    hotkey_error: Option<String>,
    settings_open: bool,
    /// `settings_open` as of the last frame: the panel forgets half-done things
    /// (a shortcut being picked, a pending "Remove?") when the window opens or closes.
    settings_was_open: bool,
    /// Height the settings panel asked for last frame.
    settings_height: f32,
    /// A file dialog is open.
    browsing: bool,
    last_window_size: Option<Vec2>,
    readout_until: Option<Instant>,
    screenshots: Option<Screenshots>,
}

impl OverlayApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        mut opts: LaunchOptions,
        settings: Settings,
        instance: Option<platform::Instance>,
    ) -> Self {
        let import = std::mem::take(&mut opts.import);
        let ctx = &cc.egui_ctx;
        theme::install_fonts(ctx);
        ctx.set_theme(egui::Theme::Dark);
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        if let Some(window) = cc.winit_window() {
            prepare_window(window, settings.window);
        }

        let tray_icon =
            decode_icon(include_bytes!("../assets/icon/icon-32.png")).map(|i| (i.pixels, i.width, i.height));
        let mut desktop = Desktop::new(ctx, tray_icon, settings.locked, instance);
        let hotkey_error = desktop.set_hotkey(&settings.unlock_hotkey).err();
        let reader = (!opts.demo).then(|| spawn_reader(ctx, settings.update_hz));
        let paths = Paths::new(&opts.data_dir());
        let mut app = Self {
            library: Library::load(&paths.library),
            paths,
            force_demo: opts.demo,
            applied: settings.clone(),
            settings,
            save_due: None,
            reference: Reference::default(),
            sample: Arc::new(demo::sample_lap()),
            session: None,
            sim_session: None,
            feed: LiveFeed::default(),
            reader,
            live: false,
            demo: None,
            desktop,
            error: None,
            load_error: None,
            hotkey_error,
            settings_open: opts.open_settings.is_some() || opts.settings_screenshot.is_some(),
            settings_was_open: false,
            settings_height: 400.0,
            browsing: false,
            last_window_size: None,
            readout_until: None,
            screenshots: Screenshots::new(opts.screenshot, opts.settings_screenshot),
        };
        if let Some(tab) = opts.open_settings {
            app.settings.tab = tab;
        }
        if app.settings.locked && !app.desktop.can_unlock() {
            log::warn!("No tray icon or hotkey to unlock with: starting unlocked");
            app.settings.locked = false;
        }
        app.enter_idle(Instant::now());
        for path in &import {
            app.import(path);
        }
        app
    }

    fn connection(&self) -> ConnectionState {
        match (self.live, &self.demo) {
            (true, _) => ConnectionState::Live,
            (false, Some(_)) => ConnectionState::Demo,
            (false, None) => ConnectionState::Waiting,
        }
    }

    fn demo_enabled(&self) -> bool {
        self.force_demo || self.settings.demo_when_idle
    }

    /// Not connected to iRacing: run the demo when it's enabled, else show nothing.
    fn enter_idle(&mut self, now: Instant) {
        self.live = false;
        let demo = self.demo_enabled();
        self.session = demo.then(|| demo::demo_session(&self.sample));
        self.on_session_changed();
        self.feed.clear();
        self.demo = demo.then(|| Demo::start(Arc::clone(&self.sample), &mut self.feed, now));
    }

    fn drain_telemetry(&mut self, now: Instant) {
        let Some(reader) = self.reader.take() else { return };
        for event in reader.drain() {
            self.on_telemetry(event, now);
        }
        self.reader = Some(reader);
    }

    fn on_telemetry(&mut self, event: TelemetryEvent, now: Instant) {
        match event {
            TelemetryEvent::Connected => {
                log::info!("Connected to iRacing");
                self.live = true;
                self.demo = None;
                self.session = self.sim_session.clone();
                self.feed.clear();
                self.on_session_changed();
            }
            TelemetryEvent::Disconnected => {
                log::info!("iRacing disconnected");
                self.sim_session = None;
                self.enter_idle(now);
            }
            TelemetryEvent::Session(info) if self.sim_session.as_ref() != Some(&info) => {
                self.sim_session = Some(info);
                if self.live {
                    self.session = self.sim_session.clone();
                    self.on_session_changed();
                }
            }
            TelemetryEvent::Frame(frame) if self.live => self.feed.push(&frame),
            TelemetryEvent::Session(_) | TelemetryEvent::Frame(_) => {}
        }
    }

    /// A new track or car, or the demo's pretend session. Live, with auto reference on,
    /// the matching saved lap is picked (passing over one whose file won't load). The
    /// demo never changes the saved choice: it only decides what's drawn meanwhile.
    fn on_session_changed(&mut self) {
        self.refresh_reference();
        if self.live
            && self.settings.auto_reference
            && let Some(session) = &self.session
            && reference::auto_pick(&mut self.library, session, self.reference.broken(), unix_now())
        {
            self.library_changed();
        }
    }

    fn refresh_reference(&mut self) {
        let demo_lap = (!self.live && self.demo_enabled()).then_some(&self.sample);
        let loaded = self.reference.refresh(&self.library, &self.paths.laps, self.session.as_ref(), demo_lap);
        self.load_error = loaded.err().map(|e| format!("Couldn't load the saved lap: {e}"));
        let lap = self.reference.lap().map(Arc::as_ref);
        self.feed.set_track_length(reference::track_length(self.session.as_ref(), lap));
    }

    fn library_changed(&mut self) {
        self.save_library();
        self.refresh_reference();
    }

    fn save_library(&mut self) {
        if let Err(e) = self.library.save(&self.paths.library) {
            log::error!("Couldn't save {}: {e}", self.paths.library.display());
            self.error = Some(format!("Couldn't save the lap library: {e}"));
        }
    }

    /// Adds a Garage 61 CSV to the library and makes it the reference; either way the
    /// settings window opens on the Reference tab to show the result.
    fn import(&mut self, path: &Path) {
        match self.library.import(&self.paths.laps, path, unix_now()) {
            Ok((entry, lap)) => {
                log::info!("New reference: {}", entry.original_name);
                self.reference.set_saved(entry.id, lap);
                self.settings.show_ref = true;
                self.error = None;
                self.library_changed();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        self.settings.tab = SettingsTab::Reference;
        self.settings_open = true;
    }

    fn handle_desktop_events(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        for event in self.desktop.poll() {
            log::debug!("{event:?}");
            match event {
                DesktopEvent::OpenSettings if self.settings_open => {
                    ctx.send_viewport_cmd_to(settings_viewport(), ViewportCommand::Focus);
                }
                DesktopEvent::OpenSettings => self.settings_open = true,
                DesktopEvent::SetLocked(locked) => self.settings.locked = locked,
                DesktopEvent::ToggleLock => self.settings.locked = !self.settings.locked,
                DesktopEvent::ResetPosition => reset_layout(ctx, frame),
                DesktopEvent::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
                DesktopEvent::Picked(path) => {
                    self.browsing = false;
                    if let Some(path) = path {
                        self.import(&path);
                    }
                }
                DesktopEvent::Import(paths) => {
                    if self.settings_open {
                        ctx.send_viewport_cmd_to(settings_viewport(), ViewportCommand::Focus);
                    }
                    for path in &paths {
                        self.import(path);
                    }
                }
            }
        }
    }

    fn on_panel_action(&mut self, action: PanelAction, ctx: &egui::Context, frame: &eframe::Frame) {
        match action {
            PanelAction::Close => self.settings_open = false,
            PanelAction::Browse => {
                if !self.browsing {
                    self.browsing = true;
                    self.desktop.pick_csv(frame);
                }
            }
            PanelAction::SelectLap(id) => {
                self.library.set_active(Some(&id), unix_now());
                self.library_changed();
            }
            PanelAction::RemoveLap(id) => {
                if let Err(e) = self.library.remove(&self.paths.laps, &id) {
                    self.error = Some(format!("Couldn't delete the lap's file: {e}"));
                }
                self.library_changed();
            }
            PanelAction::ClearReference => {
                self.library.set_active(None, unix_now());
                self.library_changed();
            }
            PanelAction::SetHotkey(spec) => {
                self.hotkey_error = self.desktop.set_hotkey(&spec).err();
                self.settings.unlock_hotkey = spec;
            }
            PanelAction::ResetLayout => reset_layout(ctx, frame),
            PanelAction::ResetAll => self.settings = reset_all(&self.settings),
            PanelAction::DismissError if self.error.is_some() => self.error = None,
            PanelAction::DismissError => self.load_error = None,
        }
    }

    /// Applies what changed in the settings since last time (from the panel, tray or
    /// hotkey) and schedules a save.
    fn apply_settings(&mut self, ctx: &egui::Context, now: Instant) {
        if self.settings == self.applied {
            return;
        }
        let old = std::mem::replace(&mut self.applied, self.settings.clone());
        let new = &self.applied;
        if old.locked != new.locked {
            ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(new.locked));
            self.desktop.set_locked(new.locked);
        }
        if old.update_hz != new.update_hz
            && let Some(reader) = &self.reader
        {
            reader.set_wake_divisor(wake_divisor(new.update_hz));
        }
        // The unlock shortcut is registered by `PanelAction::SetHotkey`, its only source.
        let restart_idle = old.demo_when_idle != new.demo_when_idle && !self.live && !self.force_demo;
        let auto_on = new.auto_reference && !old.auto_reference;
        if restart_idle {
            self.enter_idle(now);
        } else if auto_on {
            self.on_session_changed();
        }
        self.save_due = Some(now + SAVE_DELAY);
        ctx.request_repaint_after(SAVE_DELAY + FRAME);
    }

    fn save_when_due(&mut self, ctx: &egui::Context, now: Instant) {
        match self.save_due {
            Some(due) if now >= due => self.save_settings(),
            Some(due) => ctx.request_repaint_after(due - now + FRAME),
            None => {}
        }
    }

    fn save_settings(&mut self) {
        self.save_due = None;
        if let Err(e) = self.settings.save(&self.paths.settings) {
            log::error!("Couldn't save {}: {e}", self.paths.settings.display());
        }
    }

    /// Remembers the window's position (physical pixels: points differ per monitor)
    /// and size, and shows the size readout while it changes.
    fn track_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame, now: Instant) {
        let Some(outer) = ctx.input(|i| i.viewport().outer_rect) else { return };
        if let Some(pos) = frame.winit_window().and_then(|w| w.outer_position().ok()) {
            let rect = WindowRect { x: pos.x as f32, y: pos.y as f32, w: outer.width(), h: outer.height(), px: true };
            if self.settings.window.is_none_or(|w| geometry::moved(w, rect)) {
                self.settings.window = Some(rect);
            }
        }
        if self.last_window_size.is_some_and(|size| size != outer.size()) {
            self.readout_until = Some(now + READOUT_HOLD);
        }
        self.last_window_size = Some(outer.size());
        match self.readout_until {
            Some(until) if now < until => ctx.request_repaint_after(until - now + FRAME),
            Some(_) => self.readout_until = None,
            None => {}
        }
    }

    fn paint_overlay(&self, ui: &egui::Ui, window: Rect, now: Instant) -> Option<Intent> {
        let (drawn, ref_badge) = match self.reference.status(self.session.as_ref()) {
            Some(status) if self.settings.show_ref => reference::visibility(status),
            _ => (false, None),
        };
        let reference = self.reference.lap().filter(|_| drawn).map(Arc::as_ref);
        let lap_time = reference.map(Lap::lap_time_text);
        let badges: Vec<&str> = [ref_badge, self.demo.as_ref().map(|_| "DEMO")].into_iter().flatten().collect();
        let chrome = Chrome {
            opacity: self.settings.bg_opacity / 100.0,
            locked: self.settings.locked,
            settings_open: self.settings_open,
            reference_time: lap_time.as_deref(),
            badges: &badges,
            file_hover: ui.input(|i| !i.raw.hovered_files.is_empty()),
            resizing: self.readout_until.is_some_and(|until| now < until),
        };
        let header = overlay::panel_and_header(ui, window, &chrome);

        let content = overlay::content_rect(window);
        let (behind, ahead) = self.settings.window_span();
        let scene = GraphScene {
            axis: self.settings.axis,
            behind,
            ahead,
            track_length: self.feed.track_length(),
            now: self.feed.now(),
            live: self.feed.trace(),
            reference,
            ref_opacity: self.settings.ref_opacity / 100.0,
            labels: LabelOptions {
                show: self.settings.labels,
                mode: self.settings.label_mode,
                min: self.settings.label_min / 100.0,
            },
            header_height: HEADER_HEIGHT,
            obstacles: &header.obstacles,
            message: self.graph_message(),
        };
        graph::paint(&ui.painter().with_clip_rect(content), content, &scene);

        let anchors = overlay::frame_controls(ui, window, &chrome);
        header.intent.or(anchors)
    }

    fn graph_message(&self) -> Option<(&'static str, &'static str)> {
        if !self.live && self.demo.is_none() {
            Some(WAITING)
        } else if self.reference.lap().is_none() {
            Some(NO_REFERENCE)
        } else {
            None
        }
    }

    fn on_intent(&mut self, intent: Intent, frame: &eframe::Frame) {
        // egui's StartDrag/BeginResize need focus, which this window never takes.
        let window = frame.winit_window();
        let started = match intent {
            Intent::ToggleSettings => {
                self.settings_open = !self.settings_open;
                return;
            }
            Intent::Move => window.map(|w| w.drag_window()),
            Intent::Resize(dir) => window.map(|w| w.drag_resize_window(winit_direction(dir))),
        };
        if let Some(Err(e)) = started {
            log::warn!("Couldn't start moving or resizing the overlay: {e}");
        }
    }

    fn show_settings_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let monitor = frame.winit_window().and_then(|w| w.current_monitor()).map(|m| monitor_rect(&m));
        let max_height = monitor.map_or(f32::INFINITY, |m| m.height() - 16.0);
        let size = vec2(settings_panel::WIDTH, self.settings_height.min(max_height));
        let mut builder = ViewportBuilder::default()
            .with_title("Overlay settings")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_drag_and_drop(true)
            .with_inner_size(size);
        let overlay_panel = ctx.input(|i| i.viewport().outer_rect).map(overlay::panel_rect);
        if let (Some(panel), Some(monitor)) = (overlay_panel, monitor) {
            builder = builder.with_position(geometry::settings_position(panel, size, monitor));
        }

        let connection = self.connection();
        let out = ctx.show_viewport_immediate(settings_viewport(), builder, |ui, _| {
            let cx = PanelContext {
                library: &self.library,
                session: self.session.as_ref(),
                card: self.reference.card(self.session.as_ref()),
                active_id: self.library.active.as_deref(),
                error: self.error.as_deref().or(self.load_error.as_deref()),
                hotkey_error: self.hotkey_error.as_deref(),
                connection,
                browsing: self.browsing,
                file_hover: false, // `settings_ui` reads it from the window's input.
                max_height,
            };
            settings_ui(ui, &mut self.settings, cx)
        });

        self.settings_height = out.panel.desired_height.max(1.0);
        for action in out.panel.actions {
            self.on_panel_action(action, ctx, frame);
        }
        if out.close {
            self.settings_open = false;
        }
        if let Some(path) = out.dropped {
            self.import(&path);
        }
    }

    /// `--screenshot` / `--settings-screenshot`: quits once they're saved.
    fn take_screenshots(&mut self, ctx: &egui::Context) {
        let Some(mut shots) = self.screenshots.take() else { return };
        if shots.update(ctx, self.settings_rect_px(ctx)) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        } else {
            self.screenshots = Some(shots);
        }
    }

    /// The settings window on screen, physical pixels.
    fn settings_rect_px(&self, ctx: &egui::Context) -> Option<Rect> {
        if !self.settings_open {
            return None;
        }
        ctx.input_for(settings_viewport(), |i| {
            let info = i.viewport();
            info.outer_rect.map(|r| r * info.native_pixels_per_point.unwrap_or(1.0))
        })
    }
}

impl eframe::App for OverlayApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let now = Instant::now();
        platform::keep_no_activate(frame);
        self.handle_desktop_events(ctx, frame);
        self.drain_telemetry(now);
        if let Some(demo) = &mut self.demo {
            demo.advance(now, self.settings.update_hz, &mut self.feed);
            ctx.request_repaint_after(Demo::repaint_delay(self.settings.update_hz));
        }
        self.apply_settings(ctx, now);
        self.save_when_due(ctx, now);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        self.track_window(&ctx, frame, now);
        if let Some(intent) = self.paint_overlay(ui, ui.max_rect(), now) {
            self.on_intent(intent, frame);
        }
        if let Some(path) = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf())) {
            self.import(&path);
        }
        if self.settings_open != self.settings_was_open {
            settings_panel::reset(&ctx);
            self.settings_was_open = self.settings_open;
        }
        if self.settings_open {
            self.show_settings_window(&ctx, frame);
        }
        self.apply_settings(&ctx, now);
        self.take_screenshots(&ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.save_due.is_some() {
            self.save_settings();
        }
        if let Some(reader) = &mut self.reader {
            reader.stop();
        }
    }
}

struct SettingsFrame {
    panel: PanelOutput,
    dropped: Option<PathBuf>,
    /// Closed from outside the panel (Alt+F4, the taskbar).
    close: bool,
}

/// One frame of the settings window. Escape belongs to the panel: something in it may
/// use the key (cancelling a shortcut being picked or a "Remove?"); only when nothing
/// does is it a `PanelAction::Close`.
fn settings_ui(ui: &mut egui::Ui, settings: &mut Settings, cx: PanelContext) -> SettingsFrame {
    let (file_hover, dropped, close) = ui.input(|i| {
        (
            !i.raw.hovered_files.is_empty(),
            i.raw.dropped_files.first().map(|f| f.path().to_path_buf()),
            i.viewport().close_requested(),
        )
    });
    let cx = PanelContext { file_hover, ..cx };
    let panel = egui::CentralPanel::no_frame().show(ui, |ui| settings_panel::show(ui, settings, &cx)).inner;
    SettingsFrame { panel, dropped, close }
}

fn settings_viewport() -> ViewportId {
    ViewportId::from_hash_of("ito-settings")
}

/// The overlay window: undecorated, transparent, always on top, never focused, out of
/// the taskbar, restored to where it was.
fn overlay_viewport(settings: &Settings) -> ViewportBuilder {
    let mut builder = ViewportBuilder::default()
        .with_title(TITLE)
        .with_app_id(APP_ID)
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_active(false)
        .with_resizable(true)
        .with_drag_and_drop(true)
        .with_inner_size(geometry::window_size(geometry::DEFAULT_PANEL))
        .with_mouse_passthrough(settings.locked);
    // The position and the minimum size are set in `prepare_window`: here they'd be
    // converted at the scale of whichever monitor the window starts on.
    if let Some(w) = settings.window {
        builder = builder.with_inner_size(vec2(w.w, w.h));
    }
    if let Some(icon) = decode_icon(include_bytes!("../assets/icon/icon-256.png")) {
        builder = builder.with_icon(egui::IconData { rgba: icon.pixels, width: icon.width, height: icon.height });
    }
    builder
}

fn decode_icon(bytes: &[u8]) -> Option<capture::Rgba> {
    capture::decode_png(bytes).map_err(|e| log::warn!("Couldn't decode an icon: {e}")).ok()
}

/// Before the first frame: no Windows 11 border shadow, the transparency fix, no
/// focus, the minimum size, and the saved position, or the default one when the saved
/// one isn't on any monitor now.
fn prepare_window(window: &Window, saved: Option<WindowRect>) {
    use winit::platform::windows::WindowExtWindows;
    window.set_undecorated_shadow(false);
    platform::fix_transparency(window);
    platform::keep_no_activate(window);
    // In points, so winit scales it on every monitor the window goes to.
    let min = geometry::min_window_size();
    window.set_min_inner_size(Some(LogicalSize::new(min.x, min.y)));

    let primary_scale = window.primary_monitor().map_or(1.0, |m| m.scale_factor() as f32);
    let monitors: Vec<geometry::Monitor> = window.available_monitors().map(|m| physical_monitor(&m)).collect();
    if let Some(w) = saved {
        let pos = geometry::physical_position(w, primary_scale);
        if geometry::reachable(pos, w.w, &monitors) {
            let pos = PhysicalPosition::new(pos.x.round() as i32, pos.y.round() as i32);
            // Landing on a monitor with other scaling rescales the window, which can
            // nudge it: place it, size it, place it again.
            window.set_outer_position(pos);
            let _ = window.request_inner_size(LogicalSize::new(w.w, w.h));
            window.set_outer_position(pos);
            log::debug!("Overlay placed at {:?}, scale {}", window.outer_position(), window.scale_factor());
            return;
        }
    }
    let Some(primary) = window.primary_monitor() else { return };
    let rect = geometry::default_window(monitor_rect(&primary));
    let scale = primary.scale_factor();
    let px = |v: f32| (f64::from(v) * scale).round();
    window.set_outer_position(PhysicalPosition::new(px(rect.min.x), px(rect.min.y)));
    let _ = window.request_inner_size(PhysicalSize::new(px(rect.width()), px(rect.height())));
}

/// Back to the default size, centred low on the monitor the overlay is on.
fn reset_layout(ctx: &egui::Context, frame: &eframe::Frame) {
    let Some(window) = frame.winit_window() else { return };
    let Some(monitor) = window.current_monitor().or_else(|| window.primary_monitor()) else { return };
    let rect = geometry::default_window(monitor_rect(&monitor));
    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(rect.min));
    ctx.send_viewport_cmd(ViewportCommand::InnerSize(rect.size()));
}

/// A monitor's area in physical pixels, and its scale.
fn physical_monitor(monitor: &MonitorHandle) -> geometry::Monitor {
    let (pos, size) = (monitor.position(), monitor.size());
    let rect = Rect::from_min_size(pos2(pos.x as f32, pos.y as f32), vec2(size.width as f32, size.height as f32));
    geometry::Monitor { rect, scale: monitor.scale_factor() as f32 }
}

/// A monitor's area in points (its own).
fn monitor_rect(monitor: &MonitorHandle) -> Rect {
    let scale = monitor.scale_factor() as f32;
    let (pos, size) = (monitor.position(), monitor.size());
    Rect::from_min_size(pos2(pos.x as f32, pos.y as f32) / scale, vec2(size.width as f32, size.height as f32) / scale)
}

/// Defaults, except the window, the hotkey and the open tab.
fn reset_all(settings: &Settings) -> Settings {
    Settings {
        window: settings.window,
        unlock_hotkey: settings.unlock_hotkey.clone(),
        tab: settings.tab,
        ..Settings::default()
    }
}

fn spawn_reader(ctx: &egui::Context, update_hz: u32) -> IracingReader {
    let ctx = ctx.clone();
    // A 1 ms delay (not `request_repaint`) gives exactly one repaint per frame.
    let reader = IracingReader::spawn(Box::new(move || ctx.request_repaint_after(Duration::from_millis(1))));
    reader.set_wake_divisor(wake_divisor(update_hz));
    reader
}

/// iRacing sends 60 frames a second; wake the UI on every `n`th.
fn wake_divisor(update_hz: u32) -> u32 {
    (60 / update_hz.max(1)).max(1)
}

fn winit_direction(dir: egui::ResizeDirection) -> winit::window::ResizeDirection {
    use egui::ResizeDirection as E;
    use winit::window::ResizeDirection as W;
    match dir {
        E::North => W::North,
        E::South => W::South,
        E::East => W::East,
        E::West => W::West,
        E::NorthEast => W::NorthEast,
        E::SouthEast => W::SouthEast,
        E::NorthWest => W::NorthWest,
        E::SouthWest => W::SouthWest,
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::LibraryEntry;
    use crate::settings::Axis;

    #[test]
    fn reset_all_keeps_window_hotkey_and_tab() {
        let s = Settings {
            bg_opacity: 20.0,
            axis: Axis::Time,
            locked: true,
            unlock_hotkey: "Ctrl+Alt+L".into(),
            tab: SettingsTab::Timing,
            window: Some(WindowRect { x: 1.0, y: 2.0, w: 300.0, h: 120.0, px: true }),
            ..Default::default()
        };
        let r = reset_all(&s);
        assert_eq!((r.window, r.unlock_hotkey.as_str(), r.tab), (s.window, "Ctrl+Alt+L", SettingsTab::Timing));
        assert_eq!((r.bg_opacity, r.axis, r.locked), (80.0, Axis::Distance, false));
    }

    #[test]
    fn wake_divisor_follows_update_rate() {
        assert_eq!(wake_divisor(60), 1);
        assert_eq!(wake_divisor(30), 2);
    }

    /// The settings window's content in a headless egui.
    struct SettingsHarness {
        ctx: egui::Context,
        library: Library,
        /// The widget that last gained keyboard focus.
        focused: std::cell::RefCell<Option<String>>,
    }

    impl SettingsHarness {
        fn new(library: Library) -> Self {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx);
            Self { ctx, library, focused: Default::default() }
        }

        fn frame(&self, settings: &mut Settings, events: Vec<egui::Event>) -> SettingsFrame {
            self.frame_with(settings, egui::RawInput { events, ..Default::default() })
        }

        fn frame_with(&self, settings: &mut Settings, mut input: egui::RawInput) -> SettingsFrame {
            input.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(settings_panel::WIDTH, 900.0)));
            let mut out = None;
            let mut full = self.ctx.run_ui(input, |ui| out = Some(settings_ui(ui, settings, self.context())));
            full.textures_delta.clear();
            for event in full.platform_output.events {
                if let egui::output::OutputEvent::FocusGained(egui::WidgetInfo { label: Some(label), .. }) = event {
                    *self.focused.borrow_mut() = Some(label);
                }
            }
            out.expect("ran a frame")
        }

        fn context(&self) -> PanelContext<'_> {
            PanelContext {
                library: &self.library,
                session: None,
                card: None,
                active_id: None,
                error: None,
                hotkey_error: None,
                connection: ConnectionState::Waiting,
                browsing: false,
                file_hover: false,
                max_height: f32::INFINITY,
            }
        }

        /// Presses Tab until the widget named `label` has keyboard focus, then Enter.
        fn activate(&self, settings: &mut Settings, label: &str) {
            for _ in 0..40 {
                self.frame(settings, vec![key(egui::Key::Tab)]);
                if self.focused.borrow().as_deref() == Some(label) {
                    self.frame(settings, vec![key(egui::Key::Enter)]);
                    return;
                }
            }
            panic!("nothing called {label:?} takes focus");
        }
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Default::default() }
    }

    /// Closed neither by the window nor by the panel.
    fn still_open(out: &SettingsFrame) -> bool {
        !out.close && !out.panel.actions.contains(&PanelAction::Close)
    }

    #[test]
    fn escape_cancelling_a_shortcut_being_picked_keeps_settings_open() {
        let h = SettingsHarness::new(Library::default());
        let mut s = Settings::default();
        h.activate(&mut s, "Ctrl+Alt+Shift+O");
        assert!(still_open(&h.frame(&mut s, vec![key(egui::Key::Escape)])), "Escape cancels picking");
        let out = h.frame(&mut s, vec![key(egui::Key::Escape)]);
        assert_eq!(out.panel.actions, vec![PanelAction::Close], "then Escape closes");
    }

    #[test]
    fn escape_cancelling_a_remove_keeps_settings_open() {
        let lap = LibraryEntry {
            id: "a".into(),
            file: "a.csv".into(),
            original_name: "a.csv".into(),
            driver: Some("Ada".into()),
            car: None,
            track: None,
            lap_time: 116.5,
            samples: 7000,
            length_m: None,
            start_latlon: None,
            added: 1,
            last_used: 1,
        };
        let h = SettingsHarness::new(Library { laps: vec![lap], active: None });
        let mut s = Settings { tab: SettingsTab::Reference, ..Default::default() };
        h.activate(&mut s, "Remove lap");
        assert!(still_open(&h.frame(&mut s, vec![key(egui::Key::Escape)])), "Escape cancels \"Remove?\"");
        let out = h.frame(&mut s, vec![key(egui::Key::Escape)]);
        assert_eq!(out.panel.actions, vec![PanelAction::Close]);
    }

    #[test]
    fn the_settings_window_closes_when_windows_asks() {
        let h = SettingsHarness::new(Library::default());
        let close = egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() };
        let mut viewports = egui::ViewportIdMap::default();
        viewports.insert(ViewportId::ROOT, close);
        let input = egui::RawInput { viewports, ..Default::default() };
        assert!(h.frame_with(&mut Settings::default(), input).close);
    }

    #[test]
    fn data_dir_defaults_to_the_app_folder() {
        assert_eq!(LaunchOptions::default().data_dir(), settings::app_dir());
        let custom = LaunchOptions { data_dir: Some("x".into()), ..Default::default() };
        assert_eq!(custom.data_dir(), PathBuf::from("x"));
    }
}
