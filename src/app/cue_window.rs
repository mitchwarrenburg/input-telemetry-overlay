//! The brake point window: the countdown model fed from the live trace, the pulse and
//! beeps at each brake point, and the second overlay window it's drawn in (an egui
//! immediate viewport, found by its title for the things egui does only with focus).

use std::time::{Duration, Instant};

use eframe::egui::{self, Pos2, Rect, Vec2, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2};

use super::geometry::{self, Monitor};
use crate::cue::{BrakeCue, CueMode, CuePosition, CueState};
use crate::lap::Lap;
use crate::platform::{self, NativeWindow};
use crate::settings::{Settings, WindowRect};
use crate::trace::LiveTrace;
use crate::ui::cue_view::{self, CueFrame, CueIntent};
use crate::ui::graph::CarNow;
use crate::ui::overlay::{Chrome, MARGIN};

/// Unique among this program's windows: the native window is found by it.
const TITLE: &str = "Input Telemetry Overlay: brake point";
/// Between the graph's panel and the brake point window's at its default spot, points.
const GAP: f32 = 12.0;
/// The size readout stays up this long after the window stops changing size.
const READOUT_HOLD: Duration = Duration::from_millis(800);
/// A size asked for is given this long to arrive before the window's own is saved again.
const SETTLE: Duration = Duration::from_millis(600);

pub struct CueWindow {
    model: BrakeCue,
    state: CueState,
    /// The native window, once it exists.
    native: Option<NativeWindow>,
    /// The viewport was shown last frame (else showing it creates the window).
    shown: bool,
    /// The layout the window is sized for.
    compact: bool,
    /// Where to put the window's top-left corner once it exists, physical pixels.
    place: Option<Pos2>,
    /// A size asked for (points) and when: the window's own isn't saved meanwhile.
    sizing: Option<(Vec2, Instant)>,
    /// The current count and when it started (it pops in).
    count: (u8, Instant),
    pulse_at: Option<Instant>,
    last_size: Option<Vec2>,
    readout_until: Option<Instant>,
}

impl CueWindow {
    pub fn new(settings: &Settings, now: Instant) -> Self {
        Self {
            model: BrakeCue::new(),
            state: BrakeCue::new().update(None, &LiveTrace::new(), None, 0.0, &settings.cue_config()),
            native: None,
            shown: false,
            compact: settings.cue_compact,
            place: None,
            sizing: None,
            count: (0, now),
            pulse_at: None,
            last_size: None,
            readout_until: None,
        }
    }

    pub fn state(&self) -> &CueState {
        &self.state
    }

    /// Runs the countdown for this frame: grades, the pulse and the beeps.
    pub fn update(
        &mut self,
        car: Option<CarNow>,
        live: &LiveTrace,
        lap: Option<&Lap>,
        track_length: f64,
        settings: &Settings,
        now: Instant,
    ) {
        let position = car.map(|c| CuePosition { t: c.t, lap_pos: c.lap_pos, brake: c.brake });
        let prev = std::mem::replace(
            &mut self.state,
            self.model.update(position, live, lap, track_length, &settings.cue_config()),
        );
        let s = &self.state;
        let counting = |st: &CueState| if st.mode == CueMode::Countdown { st.beat } else { 0 };
        if counting(s) != self.count.0 {
            self.count = (counting(s), now);
        }
        // Both at the brake point itself, whether or not you've braked already.
        let at_brake_point = prev.zone_no == s.zone_no && prev.until_brake > 0.0 && s.until_brake <= 0.0;
        if !settings.cue_on {
            return;
        }
        if settings.cue_pulse && at_brake_point {
            self.pulse_at = Some(now);
        }
        if settings.cue_beep {
            if s.mode == CueMode::Countdown && counting(s) != counting(&prev) {
                platform::beep(880, 80);
            } else if s.mode == CueMode::Brake && prev.mode == CueMode::Countdown {
                platform::beep(1320, 300);
            }
        }
    }

    /// The pulse over both windows' backgrounds, 0..1.
    pub fn pulse(&self, now: Instant) -> f32 {
        self.pulse_at.map_or(0.0, |at| cue_view::pulse_alpha(now.duration_since(at).as_secs_f32()))
    }

    /// Something is animating: keep frames coming.
    fn animating(&self, now: Instant) -> bool {
        let since = |at: Instant| now.duration_since(at).as_secs_f32();
        (self.count.0 > 0 && since(self.count.1) < cue_view::POP_S)
            || self.pulse_at.is_some_and(|at| since(at) < cue_view::PULSE_S)
    }

    /// Back to the default size and spot for the current layout.
    pub fn reset_layout(&mut self, ctx: &egui::Context, settings: &mut Settings, screen: &Screen) {
        *slot(settings) = None;
        if self.shown {
            let rect = default_window(settings.cue_compact, screen.home);
            self.place = Some(rect.min);
            send(ctx, self.resize(rect_size(settings.cue_compact), Instant::now()));
        }
    }

    /// Asks for a new size (the window's own isn't saved until it arrives): the commands
    /// that set it.
    fn resize(&mut self, size: Vec2, now: Instant) -> [ViewportCommand; 2] {
        self.sizing = Some((size, now));
        // The minimum first, or a compact size would be held at the full one's.
        [ViewportCommand::MinInnerSize(min_window(self.compact)), ViewportCommand::InnerSize(size)]
    }

    /// Shows the window for this frame (while `settings.cue_on`). `screen` places a new
    /// window: where it was, else centred just above the graph. Returns true when its
    /// gear was clicked.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        settings_open: bool,
        screen: impl FnOnce() -> Screen,
        now: Instant,
    ) -> bool {
        let compact = settings.cue_compact;
        let mut builder = ViewportBuilder::default()
            .with_title(TITLE)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(false)
            .with_resizable(true)
            .with_mouse_passthrough(settings.locked)
            .with_min_inner_size(min_window(compact));
        if !self.shown {
            // A new window: where it was, else the default spot. The builder's position
            // is in points at a scale egui guesses, so it's placed again in pixels once
            // it exists.
            let screen = screen();
            let saved =
                (*slot(settings)).filter(|w| w.px && geometry::reachable(pos2(w.x, w.y), w.w, &screen.monitors));
            let (min, size) = match saved {
                Some(w) => (pos2(w.x, w.y), vec2(w.w, w.h)),
                None => (default_window(compact, screen.home).min, rect_size(compact)),
            };
            let scale = screen.home.map_or(1.0, |(m, _)| m.scale);
            builder = builder.with_position(min / scale).with_inner_size(size);
            self.place = Some(min);
            self.native = None;
            self.compact = compact;
            self.sizing = None;
        } else if compact != self.compact {
            send(ctx, self.switch_layout(settings, now));
        }

        let chrome = Chrome {
            opacity: settings.cue_bg_opacity / 100.0,
            locked: settings.locked,
            settings_open,
            reference_time: None,
            badges: &[],
            file_hover: false,
            resizing: self.readout_until.is_some_and(|until| now < until),
            pulse: self.pulse(now),
        };
        let pop = (now.duration_since(self.count.1).as_secs_f32() / cue_view::POP_S).min(1.0);
        let frame = CueFrame { state: &self.state, chrome, compact, contents: settings.cue_fg_opacity / 100.0, pop };
        let intent =
            ctx.show_viewport_immediate(viewport(), builder, |ui, _| cue_view::show(ui, ui.max_rect(), &frame));
        self.shown = true;
        self.attach();
        self.track(ctx, settings, now);
        if self.animating(now) {
            ctx.request_repaint();
        }

        match (intent, self.native) {
            (Some(CueIntent::ToggleSettings), _) => return true,
            (Some(CueIntent::Close), _) => settings.cue_on = false,
            (Some(CueIntent::Compact(on)), _) => settings.cue_compact = on,
            (Some(CueIntent::Move), Some(native)) => native.start_move(),
            (Some(CueIntent::Resize(dir)), Some(native)) => native.start_resize(dir),
            _ => {}
        }
        false
    }

    /// The window isn't shown this frame: egui closes it.
    pub fn hide(&mut self) {
        self.shown = false;
        self.native = None;
        self.last_size = None;
    }

    /// Finds the new window's handle, fixes its styles and puts it where it goes; then
    /// keeps it from taking focus.
    fn attach(&mut self) {
        if self.native.is_none_or(|n| !n.alive()) {
            self.native = NativeWindow::find(TITLE);
            if let Some(native) = self.native {
                native.fix_transparency();
            }
        }
        let Some(native) = self.native else { return };
        native.keep_no_activate();
        if let Some(p) = self.place.take() {
            native.move_to_px(p.x.round() as i32, p.y.round() as i32);
        }
    }

    /// Collapsing or expanding keeps the window's top-left corner and width; each layout
    /// has its own height. Returns the commands that resize the window.
    fn switch_layout(&mut self, settings: &mut Settings, now: Instant) -> [ViewportCommand; 2] {
        let current = if self.compact { settings.cue_compact_window } else { settings.cue_window };
        self.compact = settings.cue_compact;
        let Some(cur) = current else { return self.resize(rect_size(self.compact), now) };
        let kept = *slot(settings);
        let h = kept.map_or_else(|| rect_size(self.compact).y, |w| w.h);
        let size = vec2(cur.w, h.max(min_window(self.compact).y));
        *slot(settings) = Some(WindowRect { h: size.y, ..cur });
        self.resize(size, now)
    }

    /// Remembers where the window is (position in physical pixels, size in points) for
    /// its layout, and shows the size readout while it changes.
    fn track(&mut self, ctx: &egui::Context, settings: &mut Settings, now: Instant) {
        let Some(outer) = ctx.input_for(viewport(), |i| i.viewport().outer_rect) else { return };
        if let Some((size, asked)) = self.sizing {
            if (outer.size() - size).length() > 1.0 && now.duration_since(asked) < SETTLE {
                return;
            }
            self.sizing = None;
        }
        if let Some(px) = self.native.and_then(NativeWindow::outer_px).filter(|_| self.place.is_none()) {
            let rect = WindowRect { x: px.min.x, y: px.min.y, w: outer.width(), h: outer.height(), px: true };
            let saved = slot(settings);
            if saved.is_none_or(|w| geometry::moved(w, rect)) {
                *saved = Some(rect);
            }
        }
        if self.last_size.is_some_and(|size| size != outer.size()) {
            self.readout_until = Some(now + READOUT_HOLD);
        }
        self.last_size = Some(outer.size());
        match self.readout_until {
            Some(until) if now < until => ctx.request_repaint_after(until - now),
            Some(_) => self.readout_until = None,
            None => {}
        }
    }

    /// The window on screen, physical pixels (for `--cue-screenshot`).
    pub fn rect_px(&self) -> Option<Rect> {
        self.native.and_then(NativeWindow::outer_px)
    }
}

pub fn viewport() -> ViewportId {
    ViewportId::from_hash_of("ito-brake-point")
}

fn send(ctx: &egui::Context, commands: impl IntoIterator<Item = ViewportCommand>) {
    for command in commands {
        ctx.send_viewport_cmd_to(viewport(), command);
    }
}

/// The saved spot for the current layout.
fn slot(settings: &mut Settings) -> &mut Option<WindowRect> {
    if settings.cue_compact { &mut settings.cue_compact_window } else { &mut settings.cue_window }
}

/// Default window size (points) for a layout: its panel plus the anchor margin.
fn rect_size(compact: bool) -> Vec2 {
    let panel =
        if compact { vec2(cue_view::DEFAULT_PANEL.x, cue_view::COMPACT_HEIGHT) } else { cue_view::DEFAULT_PANEL };
    geometry::window_size(panel)
}

fn min_window(compact: bool) -> Vec2 {
    geometry::window_size(if compact { cue_view::MIN_COMPACT } else { cue_view::MIN_PANEL })
}

/// Where things are on screen, for placing the brake point window.
pub struct Screen {
    /// Every monitor, physical pixels.
    pub monitors: Vec<Monitor>,
    /// The graph's monitor, and the graph's panel in points on it.
    pub home: Option<(Monitor, Rect)>,
}

/// The default window, physical pixels: its panel centred just above the graph's panel,
/// or below it when there's no room above.
fn default_window(compact: bool, home: Option<(Monitor, Rect)>) -> Rect {
    let size = rect_size(compact);
    let Some((monitor, panel)) = home else {
        return Rect::from_min_size(pos2(100.0, 100.0), size);
    };
    let top = monitor.rect.top() / monitor.scale;
    // The windows' anchor margins overlap the gap.
    let above = panel.top() - GAP + MARGIN - size.y;
    let y = if above >= top { above } else { panel.bottom() + GAP - MARGIN };
    let rect = Rect::from_min_size(pos2(panel.center().x - size.x / 2.0, y), size);
    Rect::from_min_max(rect.min * monitor.scale, rect.max * monitor.scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Monitor = Monitor { rect: Rect { min: Pos2::ZERO, max: pos2(1920.0, 1080.0) }, scale: 1.0 };

    #[test]
    fn the_default_spot_is_just_above_the_graph() {
        let panel = Rect::from_min_size(pos2(620.0, 846.0), vec2(680.0, 170.0));
        let w = default_window(false, Some((MONITOR, panel)));
        assert_eq!(w.size(), geometry::window_size(cue_view::DEFAULT_PANEL));
        assert_eq!(w.center().x, panel.center().x);
        assert_eq!(w.shrink(MARGIN).bottom(), panel.top() - GAP, "panels 12 apart");

        let high = Rect::from_min_size(pos2(620.0, 20.0), vec2(680.0, 170.0));
        let below = default_window(true, Some((MONITOR, high)));
        assert_eq!(below.shrink(MARGIN).top(), high.bottom() + GAP, "below");
    }

    #[test]
    fn collapsing_keeps_the_corner_and_width_and_each_layout_its_height() {
        let now = Instant::now();
        let full = WindowRect { x: 500.0, y: 300.0, w: 420.0, h: 130.0, px: true };
        let mut settings = Settings { cue_window: Some(full), ..Default::default() };
        let mut window = CueWindow::new(&settings, now);
        window.shown = true;

        settings.cue_compact = true;
        let asked = window.switch_layout(&mut settings, now);
        let compact = vec2(420.0, rect_size(true).y);
        // The smaller minimum first, or the compact height would be held at the full one's.
        assert_eq!(asked, [ViewportCommand::MinInnerSize(min_window(true)), ViewportCommand::InnerSize(compact)]);
        assert_eq!(settings.cue_compact_window, Some(WindowRect { h: compact.y, ..full }));
        assert_eq!(settings.cue_window, Some(full), "the full layout's own is kept");

        // Resized while compact, then expanded: back to the full height, the new width.
        settings.cue_compact_window = Some(WindowRect { x: 640.0, w: 300.0, ..settings.cue_compact_window.unwrap() });
        settings.cue_compact = false;
        let asked = window.switch_layout(&mut settings, now);
        assert_eq!(asked[1], ViewportCommand::InnerSize(vec2(300.0, 130.0)));
        assert_eq!(settings.cue_window, Some(WindowRect { x: 640.0, w: 300.0, ..full }));
    }

    #[test]
    fn the_default_spot_is_in_pixels_on_a_scaled_monitor() {
        let scaled = Monitor { rect: Rect { min: pos2(1920.0, 0.0), max: pos2(5760.0, 2160.0) }, scale: 1.5 };
        let panel = Rect::from_min_size(pos2(1600.0, 1200.0), vec2(680.0, 170.0));
        let w = default_window(false, Some((scaled, panel)));
        assert_eq!(w.size(), geometry::window_size(cue_view::DEFAULT_PANEL) * 1.5);
        assert_eq!(w.center().x, panel.center().x * 1.5);
    }
}
