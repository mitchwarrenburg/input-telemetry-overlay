//! The settings window's content: Display, Labels, Timing and Reference tabs.

use std::sync::Arc;

use eframe::egui::epaint::RectShape;
use eframe::egui::scroll_area::ScrollBarVisibility;
use eframe::egui::style::ScrollStyle;
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{
    self, Align, Align2, Color32, Frame, Galley, Id, Key, Layout, Margin, Modifiers, Painter, Pos2, Rect, ScrollArea,
    Sense, Shape, Stroke, StrokeKind, TextFormat, Ui, UiBuilder, Vec2, pos2, vec2,
};

use crate::lap::{Lap, format_lap_time};
use crate::library::{Library, LibraryEntry};
use crate::matching::MatchStatus;
use crate::settings::{Axis, LabelMode, Settings, SettingsTab};
use crate::telemetry::SessionInfo;
use crate::ui::theme::{self, Weight};
use crate::ui::widgets::{
    self, Button, DropZone, Icon, IconButton, LINE, LINE_SMALL, Link, Segmented, Slider, Switch, TabBar, TextBlock,
};

/// Settings window width, points.
pub const WIDTH: f32 = 304.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// iRacing isn't running and demo mode is off.
    Waiting,
    /// iRacing isn't running; the simulated driver is on.
    Demo,
    /// Connected to iRacing.
    Live,
}

pub struct PanelContext<'a> {
    pub library: &'a Library,
    pub session: Option<&'a SessionInfo>,
    /// The Reference tab's card.
    pub card: Option<RefCard<'a>>,
    /// Library id of the chosen lap (highlighted in the list), whether it's drawn or not.
    pub active_id: Option<&'a str>,
    /// Last import/load error to show in the Reference tab.
    pub error: Option<&'a str>,
    /// Why the unlock shortcut couldn't be registered, shown under it.
    pub hotkey_error: Option<&'a str>,
    pub connection: ConnectionState,
    /// A file dialog is open (disable Browse).
    pub browsing: bool,
    /// Files are being dragged over the settings window (highlight the drop zone).
    pub file_hover: bool,
    /// Tallest the window can be, points; a taller tab scrolls.
    pub max_height: f32,
}

/// What the Reference tab's card shows.
#[derive(Debug, Clone, Copy)]
pub enum RefCard<'a> {
    /// The chosen saved lap, drawn, and how it matches the session.
    Saved(&'a Lap, MatchStatus),
    /// The chosen saved lap while the demo draws the bundled lap in its place (the
    /// saved one is for another track).
    Waiting(&'a Lap),
    /// The bundled lap the demo draws while no saved lap is chosen.
    Bundled(&'a Lap, MatchStatus),
}

impl<'a> RefCard<'a> {
    fn lap(self) -> &'a Lap {
        match self {
            RefCard::Saved(lap, _) | RefCard::Waiting(lap) | RefCard::Bundled(lap, _) => lap,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelAction {
    Close,
    /// Open the file picker for a Garage 61 CSV.
    Browse,
    /// Make a saved lap the reference.
    SelectLap(String),
    /// Forget a saved lap (and delete its copy).
    RemoveLap(String),
    /// Show no reference.
    ClearReference,
    /// A lock/unlock shortcut was picked: register it, even when it's the current one
    /// (registering it may have failed before).
    SetHotkey(String),
    ResetLayout,
    /// Settings back to defaults (keeps window position, library and hotkey).
    ResetAll,
    DismissError,
}

pub struct PanelOutput {
    pub actions: Vec<PanelAction>,
    /// Height the content needs without scrolling, points; the window is resized to it
    /// (up to [`PanelContext::max_height`]).
    pub desired_height: f32,
}

/// Panel edge to content: 1 px border + 14 px padding.
const PAD_X: f32 = 15.0;
/// Space between rows in a tab.
const GAP: f32 = 13.0;
/// Space between a row's label and its control.
const ROW_GAP: f32 = 6.0;
const RADIUS: f32 = 12.0;
const LAP_ROW_HEIGHT: f32 = 46.0;
/// The saved-laps list scrolls past this height (three and a half rows).
const LAPS_MAX_HEIGHT: f32 = 3.5 * LAP_ROW_HEIGHT + 3.0 * ROW_GAP;
/// "Reset to defaults" and the connection status: 1 px line, padding 9 / 11 around a
/// 17 px line, 1 px border.
const FOOT_HEIGHT: f32 = 1.0 + 9.0 + LINE_SMALL + 11.0 + 1.0;
/// Where a pending "Remove?" (a lap id) is kept.
const PENDING_REMOVE: &str = "ito_settings_pending_remove";

const TABS: [(SettingsTab, &str); 4] = [
    (SettingsTab::Display, "Display"),
    (SettingsTab::Labels, "Labels"),
    (SettingsTab::Timing, "Timing"),
    (SettingsTab::Reference, "Reference"),
];

/// Draws the settings panel into `ui` (the whole settings window, which is
/// transparent: the panel paints its own rounded background). Edits `settings` in
/// place; the caller saves and applies changes.
pub fn show(ui: &mut egui::Ui, settings: &mut Settings, cx: &PanelContext) -> PanelOutput {
    let mut actions = Vec::new();
    let origin = ui.available_rect_before_wrap().min;
    let background = ui.painter().add(Shape::Noop);
    let mut panel = ui.new_child(
        UiBuilder::new()
            .id_salt("settings_panel")
            .max_rect(Rect::from_min_size(origin, vec2(WIDTH, f32::INFINITY)))
            .layout(Layout::top_down(Align::Min)),
    );
    panel.spacing_mut().item_spacing = Vec2::ZERO;
    // Dimmed controls (`.is-disabled { opacity: 0.4 }`).
    panel.visuals_mut().disabled_alpha = 0.4;

    head(&mut panel, &mut actions);
    // Inside the 1 px border.
    Frame::new().inner_margin(Margin::symmetric(1, 0)).show(&mut panel, |ui| {
        ui.add(TabBar::new(&mut settings.tab, &TABS));
    });
    let body_max = cx.max_height - panel.min_rect().height() - FOOT_HEIGHT;
    let scrolled_off = tab_body(&mut panel, settings, cx, &mut actions, body_max);
    foot(&mut panel, cx.connection, &mut actions);

    let rect = Rect::from_min_size(origin, vec2(WIDTH, panel.min_rect().height()));
    let fill = RectShape::new(rect, RADIUS, theme::UI_SURFACE, Stroke::new(1.0, theme::UI_LINE), StrokeKind::Inside);
    ui.painter().set(background, fill);
    ui.advance_cursor_after_rect(rect);
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        actions.push(PanelAction::Close);
    }
    PanelOutput { actions, desired_height: rect.height() + scrolled_off }
}

/// Forgets what the panel was in the middle of: a shortcut being picked, a pending
/// "Remove?". Call when its window closes or opens, so neither survives unseen.
pub fn reset(ctx: &egui::Context) {
    widgets::cancel_shortcut_capture(ctx);
    ctx.data_mut(|d| d.remove::<String>(Id::new(PENDING_REMOVE)));
}

/// "OVERLAY SETTINGS" and the close button.
fn head(ui: &mut Ui, actions: &mut Vec<PanelAction>) {
    // 1 px border, then padding 10 / 8 / 4 around a 26 px button.
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, 41.0), Sense::hover());
    let close = Rect::from_min_size(pos2(rect.right() - 9.0 - 26.0, rect.top() + 11.0), Vec2::splat(26.0));
    let title = widgets::caps(ui, "Overlay settings", theme::font(Weight::SemiBold, 10.5), theme::UI_MUTED, 0.1);
    let title_rect = Rect::from_x_y_ranges(rect.left() + PAD_X..=close.left(), close.y_range());
    widgets::paint_galley(ui.painter(), title_rect, Align2::LEFT_CENTER, title, theme::UI_MUTED);
    if ui.place(close, IconButton::new(Icon::Close, "Close settings")).clicked() {
        actions.push(PanelAction::Close);
    }
}

/// The open tab, scrolling when it's taller than `max_height`. Returns how much of it
/// is scrolled out of view.
fn tab_body(
    ui: &mut Ui,
    settings: &mut Settings,
    cx: &PanelContext,
    actions: &mut Vec<PanelAction>,
    max_height: f32,
) -> f32 {
    ui.scope(|ui| {
        // A thin bar in the right-hand padding; it takes no width from the content.
        ui.spacing_mut().scroll = ScrollStyle {
            bar_width: 4.0,
            floating_width: 4.0,
            bar_outer_margin: 3.0,
            dormant_handle_opacity: 0.6,
            ..ScrollStyle::floating()
        };
        let out = ScrollArea::vertical()
            .id_salt(("settings_body", settings.tab))
            .max_height(max_height.max(0.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                Frame::new().inner_margin(Margin { left: 15, right: 15, top: 13, bottom: 14 }).show(ui, |ui| {
                    ui.set_width(WIDTH - 2.0 * PAD_X);
                    ui.spacing_mut().item_spacing.y = GAP;
                    match settings.tab {
                        SettingsTab::Display => display_tab(ui, settings, cx, actions),
                        SettingsTab::Labels => labels_tab(ui, settings),
                        SettingsTab::Timing => timing_tab(ui, settings),
                        SettingsTab::Reference => reference_tab(ui, settings, cx, actions),
                    }
                });
            });
        (out.content_size.y - out.inner_rect.height()).max(0.0)
    })
    .inner
}

/// Section heading; it sits 7 px above its first row (CSS `margin-bottom: -6px`).
fn section(ui: &mut Ui, title: &str) {
    widgets::section_label(ui, title);
    ui.add_space(-6.0);
}

/// Hairline across the whole panel.
fn divider(ui: &mut Ui) {
    widgets::divider(ui, PAD_X - 1.0);
}

/// A label with its control under it.
fn row<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = ROW_GAP;
        add_contents(ui)
    })
    .inner
}

// ---- Display ----

fn display_tab(ui: &mut Ui, s: &mut Settings, cx: &PanelContext, actions: &mut Vec<PanelAction>) {
    section(ui, "Opacity");
    ui.add(Slider::new("Background", &mut s.bg_opacity, 0.0..=100.0).unit("%"));
    ui.add(Slider::new("Reference fill", &mut s.ref_opacity, 0.0..=100.0).unit("%"));
    divider(ui);
    section(ui, "Layout");
    let lock_hint = format!("Click-through. Unlock with {} or the tray icon", s.unlock_hotkey);
    ui.add(Switch::new(&mut s.locked, "Lock size & position").hint(&lock_hint));
    row(ui, |ui| {
        widgets::row_label(ui, "Lock / unlock shortcut");
        if let Some(spec) = widgets::shortcut_field(ui, &s.unlock_hotkey) {
            actions.push(PanelAction::SetHotkey(spec));
        }
        if let Some(e) = cx.hotkey_error {
            widgets::error_text(ui, e);
        }
    });
    ui.add(Switch::new(&mut s.demo_when_idle, "Demo when iRacing isn't running"));
    if ui.add(Button::new("Reset size & position").fill_width()).clicked() {
        actions.push(PanelAction::ResetLayout);
    }
}

// ---- Labels ----

fn labels_tab(ui: &mut Ui, s: &mut Settings) {
    const MODES: [(LabelMode, &str); 3] =
        [(LabelMode::Live, "Live"), (LabelMode::Reference, "Reference"), (LabelMode::Both, "Both")];
    ui.add(Switch::new(&mut s.labels, "Brake peak labels").hint("Max % reached in each braking zone"));
    ui.add_enabled_ui(s.labels, |ui| {
        row(ui, |ui| {
            widgets::row_label(ui, "Label");
            ui.add(Segmented::new(&mut s.label_mode, &MODES));
        });
        ui.add(Slider::new("Ignore peaks below", &mut s.label_min, 0.0..=50.0).unit("%"));
        label_key(ui);
    });
}

/// Explains the two label pills: solid is yours, outlined is the reference's.
fn label_key(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0 + LINE_SMALL + 10.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, theme::UI_RAISED);
    let mut x = rect.left() + 12.0;
    for (reference, pill, text) in [(false, "82%", "Your peak"), (true, "76%", "Reference peak")] {
        let pill_rect = peak_pill(ui.painter(), pos2(x, rect.center().y), pill, reference);
        let text = ui.painter().layout_no_wrap(text.to_owned(), theme::font(Weight::Regular, 12.0), theme::UI_MUTED);
        let text_rect = Rect::from_x_y_ranges(pill_rect.right() + 7.0..=rect.right(), rect.y_range());
        x = text_rect.left() + text.size().x + 16.0;
        widgets::paint_galley(ui.painter(), text_rect, Align2::LEFT_CENTER, text, theme::UI_MUTED);
    }
}

/// A brake-peak label as the overlay draws it; returns its rect.
fn peak_pill(painter: &Painter, left_center: Pos2, text: &str, reference: bool) -> Rect {
    let (weight, color) = if reference { (Weight::SemiBold, theme::HUD_TEXT) } else { (Weight::Bold, Color32::WHITE) };
    let galley = painter.layout_no_wrap(text.to_owned(), theme::font(weight, 11.0), color);
    let rect = Rect::from_min_size(left_center - vec2(0.0, 8.5), vec2(galley.size().x + 10.0, 17.0));
    if reference {
        painter.rect(rect, 4.0, theme::SURFACE, Stroke::new(1.0, theme::alpha(theme::BRAKE, 0.9)), StrokeKind::Inside);
    } else {
        painter.rect_filled(rect, 4.0, theme::BRAKE_PILL);
    }
    widgets::paint_galley(painter, rect, Align2::CENTER_CENTER, galley, color);
    rect
}

// ---- Timing ----

fn timing_tab(ui: &mut Ui, s: &mut Settings) {
    const AXES: [(Axis, &str); 2] = [(Axis::Distance, "Distance"), (Axis::Time, "Time")];
    const RATES: [(u32, &str); 2] = [(30, "30 Hz"), (60, "60 Hz")];
    row(ui, |ui| {
        widgets::row_label(ui, "X-axis");
        ui.add(Segmented::new(&mut s.axis, &AXES));
    });
    let (history, ahead) = match s.axis {
        Axis::Distance => (
            Slider::new("History", &mut s.history_m, 100.0..=1500.0).step(50.0).unit(" m"),
            Slider::new("Look-ahead", &mut s.ahead_m, 0.0..=1500.0).step(50.0).unit(" m"),
        ),
        Axis::Time => (
            Slider::new("History", &mut s.history_s, 1.0..=20.0).step(0.5).unit(" s"),
            Slider::new("Look-ahead", &mut s.ahead_s, 0.0..=15.0).step(0.5).unit(" s"),
        ),
    };
    ui.add(history.detail("behind the car"));
    ui.add(ahead.detail("reference preview").off_at_zero());
    divider(ui);
    row(ui, |ui| {
        widgets::row_label(ui, "Update rate");
        ui.add(Segmented::new(&mut s.update_hz, &RATES));
        widgets::hint(ui, "iRacing sends telemetry at 60 Hz. 30 Hz halves the redraw cost.");
    });
}

// ---- Reference ----

fn reference_tab(ui: &mut Ui, s: &mut Settings, cx: &PanelContext, actions: &mut Vec<PanelAction>) {
    if let Some(card) = cx.card {
        reference_card(ui, card, cx, actions);
    }
    saved_laps(ui, cx, actions);
    let title = if cx.card.is_some() { "Drop another lap here to replace it" } else { "Drop a Garage 61 lap CSV" };
    let drop_zone = DropZone::new(title)
        .hint("Needs LapDistPct, Brake and Throttle columns")
        .compact(cx.card.is_some())
        .highlight(cx.file_hover);
    if ui.add_enabled(!cx.browsing, drop_zone).clicked() {
        actions.push(PanelAction::Browse);
    }
    if let Some(message) = cx.error {
        ui.add_space(-6.0);
        error_box(ui, message, actions);
    }
    ui.add(Switch::new(&mut s.show_ref, "Show reference lap"));
    ui.add(
        Switch::new(&mut s.auto_reference, "Pick a matching lap automatically")
            .hint("Uses the saved lap for this track and car when a session starts"),
    );
    widgets::hint(ui, "Lined up by lap distance, so a lap of any pace matches corner for corner.");
}

/// The chosen lap (or the bundled one): who, what, how it matches, and Replace /
/// Remove. The bundled lap isn't a choice, so it has no Remove.
fn reference_card(ui: &mut Ui, card: RefCard, cx: &PanelContext, actions: &mut Vec<PanelAction>) {
    let lap = card.lap();
    card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = vec2(6.0, 5.0);
        let name = match card {
            RefCard::Bundled(..) => "Sample lap (bundled)".to_owned(),
            RefCard::Saved(..) | RefCard::Waiting(_) => {
                lap.meta.driver.clone().unwrap_or_else(|| lap.meta.file_name.clone())
            }
        };
        card_title(ui, lap, &name);
        let sub = car_and_track(lap.meta.car.as_deref(), lap.meta.track.as_deref())
            .unwrap_or_else(|| "Car and track not in file name".to_owned());
        widgets::hint(ui, &sub);
        let chip = match card {
            RefCard::Saved(_, status) | RefCard::Bundled(_, status) => StatusChip::long(ui, status),
            RefCard::Waiting(_) => StatusChip::new(ui, Icon::Ring, theme::UI_MUTED, "Not in the demo", theme::UI_MUTED),
        };
        card_meta(ui, &lap_stats(lap), chip);
        if let RefCard::Waiting(lap) = card {
            widgets::hint(ui, &waiting_note(lap, cx.session));
        }
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!cx.browsing, Button::new("Replace…")).clicked() {
                actions.push(PanelAction::Browse);
            }
            if !matches!(card, RefCard::Bundled(..)) && ui.add(Button::new("Remove").quiet()).clicked() {
                actions.push(PanelAction::ClearReference);
            }
        });
    });
}

/// When a lap the demo can't draw will be drawn, and what the demo shows meanwhile.
fn waiting_note(lap: &Lap, demo: Option<&SessionInfo>) -> String {
    let when = match lap.meta.track.as_deref() {
        Some(track) => format!("Drawn when you drive {track}."),
        None => "Drawn in your iRacing sessions.".to_owned(),
    };
    match demo.and_then(|s| s.track_display_name.as_deref()) {
        Some(demo_track) => format!("{when} The demo runs {demo_track} with the bundled lap."),
        None => format!("{when} The demo runs with the bundled lap."),
    }
}

fn card_frame() -> Frame {
    Frame::new()
        .fill(theme::UI_RAISED)
        .stroke(Stroke::new(1.0, theme::UI_LINE))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(12, 11))
}

/// Source badge, driver (or file) name, lap time.
fn card_title(ui: &mut Ui, lap: &Lap, name: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), LINE), Sense::hover());
    let painter = ui.painter();
    let badge = source_badge(ui, painter, rect.left_center(), if lap.meta.source.is_some() { "G61" } else { "CSV" });
    let time = painter.layout_no_wrap(lap.lap_time_text(), theme::font(Weight::Bold, 14.0), theme::UI_TEXT);
    let name_width = rect.right() - badge.right() - 8.0 - time.size().x - 8.0;
    let name = widgets::single_line(ui, name, theme::font(Weight::SemiBold, 13.0), theme::UI_TEXT, name_width);
    let name_rect = Rect::from_x_y_ranges(badge.right() + 8.0..=rect.right(), rect.y_range());
    widgets::paint_galley(painter, name_rect, Align2::LEFT_CENTER, name, theme::UI_TEXT);
    widgets::paint_galley(painter, rect, Align2::RIGHT_CENTER, time, theme::UI_TEXT);
}

/// "G61" / "CSV" tag; returns its rect.
fn source_badge(ui: &Ui, painter: &Painter, left_center: Pos2, text: &str) -> Rect {
    let galley = widgets::caps(ui, text, theme::font(Weight::Bold, 9.5), theme::UI_MUTED, 0.08);
    let rect = Rect::from_min_size(left_center - vec2(0.0, 7.5), vec2(galley.size().x + 10.0, 15.0));
    painter.rect_filled(rect, 4.0, widgets::white(0.08));
    widgets::paint_galley(painter, rect, Align2::CENTER_CENTER, galley, theme::UI_MUTED);
    rect
}

/// Stats on the left, match status on the right (or on its own line when both don't fit).
fn card_meta(ui: &mut Ui, stats: &str, chip: StatusChip) {
    let width = ui.available_width();
    let stats = ui.painter().layout_no_wrap(stats.to_owned(), theme::font(Weight::Regular, 11.5), theme::UI_MUTED);
    let one_line = stats.size().x + 10.0 + chip.width() <= width;
    let height = if one_line { 16.0 } else { 16.0 + 4.0 + 16.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let top = Rect::from_min_size(rect.min, vec2(width, 16.0));
    widgets::paint_galley(ui.painter(), top, Align2::LEFT_CENTER, stats, theme::UI_MUTED);
    if one_line {
        chip.paint(ui.painter(), pos2(rect.right() - chip.width(), top.center().y));
    } else {
        chip.paint(ui.painter(), pos2(rect.left(), rect.bottom() - 8.0));
    }
}

/// `6,959 samples · 60 Hz · 5.79 km`
fn lap_stats(lap: &Lap) -> String {
    let mut s = format!("{} samples · {:.0} Hz", thousands(lap.n()), lap.hz);
    if let Some(m) = lap.track_length_est {
        s.push_str(&format!(" · {:.2} km", m / 1000.0));
    }
    s
}

/// `6959` → `6,959`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn car_and_track(car: Option<&str>, track: Option<&str>) -> Option<String> {
    match (car, track) {
        (Some(c), Some(t)) => Some(format!("{c} · {t}")),
        (Some(one), None) | (None, Some(one)) => Some(one.to_owned()),
        (None, None) => None,
    }
}

/// Icon plus words for how a lap relates to the session.
struct StatusChip {
    icon: Icon,
    icon_color: Color32,
    text: Arc<Galley>,
}

impl StatusChip {
    const ICON: f32 = 13.0;

    /// The reference card's wording.
    fn long(ui: &Ui, status: MatchStatus) -> Self {
        let (icon, icon_color, text) = match status {
            MatchStatus::Match => (Icon::Check, theme::OK, "Matches session"),
            MatchStatus::DifferentCar => (Icon::Warning, theme::WARN, "Different car"),
            MatchStatus::DifferentLayout => (Icon::Warning, theme::WARN, "Different layout"),
            MatchStatus::DifferentTrack => (Icon::Warning, theme::WARN, "Different track — hidden"),
            MatchStatus::Unknown => (Icon::Ring, theme::UI_MUTED, "No session yet"),
        };
        let color = if status == MatchStatus::Unknown { theme::UI_MUTED } else { theme::UI_TEXT };
        Self::new(ui, icon, icon_color, text, color)
    }

    /// A saved-lap row's wording; nothing to say without a session.
    fn short(ui: &Ui, status: MatchStatus) -> Option<Self> {
        let (icon, icon_color, text) = match status {
            MatchStatus::Match => (Icon::Check, theme::OK, "Match"),
            MatchStatus::DifferentCar => (Icon::Warning, theme::WARN, "Other car"),
            MatchStatus::DifferentLayout => (Icon::Warning, theme::WARN, "Other layout"),
            MatchStatus::DifferentTrack => (Icon::Warning, theme::WARN, "Other track"),
            MatchStatus::Unknown => return None,
        };
        Some(Self::new(ui, icon, icon_color, text, theme::UI_MUTED))
    }

    fn new(ui: &Ui, icon: Icon, icon_color: Color32, text: &str, color: Color32) -> Self {
        let text = ui.painter().layout_no_wrap(text.to_owned(), theme::font(Weight::Regular, 11.5), color);
        Self { icon, icon_color, text }
    }

    fn width(&self) -> f32 {
        Self::ICON + 4.0 + self.text.size().x
    }

    fn paint(&self, painter: &Painter, left_center: Pos2) {
        let icon = Rect::from_min_size(left_center - vec2(0.0, Self::ICON / 2.0), Vec2::splat(Self::ICON));
        widgets::paint_icon(painter, icon, self.icon, self.icon_color, 2.2);
        let text_pos = pos2(icon.right() + 4.0, left_center.y - self.text.size().y / 2.0);
        painter.galley(text_pos, Arc::clone(&self.text), theme::UI_TEXT);
    }
}

/// What happened on a saved-lap row this frame.
enum RowEvent {
    Select,
    AskRemove,
    Remove,
}

/// Library laps, matches first; click one to use it, × (then "Remove?") to forget it.
fn saved_laps(ui: &mut Ui, cx: &PanelContext, actions: &mut Vec<PanelAction>) {
    let laps = cx.library.sorted_for(cx.session);
    if laps.is_empty() {
        return;
    }
    section(ui, "Saved laps");
    let pending_key = Id::new(PENDING_REMOVE);
    let mut pending: Option<String> = ui.data(|d| d.get_temp(pending_key));
    let mut handled = false;
    ui.scope(|ui| {
        ui.spacing_mut().scroll = ScrollStyle { floating: false, bar_width: 4.0, ..ScrollStyle::solid() };
        ScrollArea::vertical()
            .max_height(LAPS_MAX_HEIGHT)
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::VisibleWhenNeeded)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = ROW_GAP;
                for (entry, status) in laps {
                    let active = cx.active_id == Some(entry.id.as_str());
                    let confirming = pending.as_deref() == Some(entry.id.as_str());
                    let Some(event) = lap_row(ui, entry, status, active, confirming) else { continue };
                    handled = true;
                    match event {
                        RowEvent::Select => {
                            actions.push(PanelAction::SelectLap(entry.id.clone()));
                            pending = None;
                        }
                        RowEvent::AskRemove => pending = Some(entry.id.clone()),
                        RowEvent::Remove => {
                            actions.push(PanelAction::RemoveLap(entry.id.clone()));
                            pending = None;
                        }
                    }
                }
            });
    });
    // Escape (instead of closing the window) or a click anywhere else (another row
    // included) cancels a pending "Remove?".
    let escaped = pending.is_some() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    if escaped || (!handled && ui.input(|i| i.pointer.any_click())) {
        pending = None;
    }
    ui.data_mut(|d| match pending {
        Some(id) => {
            d.insert_temp(pending_key, id);
        }
        None => d.remove::<String>(pending_key),
    });
}

fn lap_row(ui: &mut Ui, entry: &LibraryEntry, status: MatchStatus, active: bool, confirming: bool) -> Option<RowEvent> {
    let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), LAP_ROW_HEIGHT), Sense::click());
    let driver = entry.driver.clone().unwrap_or_else(|| entry.original_name.clone());
    row.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), active, &driver));
    paint_row_background(ui.painter(), &row, active);
    let remove = remove_button(ui, rect, confirming);
    let content = Rect::from_min_max(
        pos2(rect.left() + 10.0, rect.top() + 7.0),
        pos2(remove.rect.left() - 8.0, rect.bottom() - 7.0),
    );
    paint_row_text(ui, content, entry, &driver, status);

    match (remove.clicked(), confirming, row.clicked()) {
        (true, true, _) => Some(RowEvent::Remove),
        (true, false, _) => Some(RowEvent::AskRemove),
        (false, _, true) => Some(RowEvent::Select),
        _ => None,
    }
}

/// The row's × button, or "Remove?" while confirming, at its right end.
fn remove_button(ui: &mut Ui, row: Rect, confirming: bool) -> egui::Response {
    let area = row.shrink2(vec2(6.0, 0.0));
    let mut ui = ui.new_child(UiBuilder::new().max_rect(area).layout(Layout::right_to_left(Align::Center)));
    if confirming {
        ui.add(Button::new("Remove?").danger().small())
    } else {
        ui.add(IconButton::new(Icon::Close, "Remove lap").size(22.0, 12.0))
    }
}

fn paint_row_background(painter: &Painter, row: &egui::Response, active: bool) {
    let hovered = row.enabled() && row.contains_pointer();
    let (fill, border) = match (active, hovered) {
        (true, _) => (theme::alpha(theme::ACCENT, 0.07), theme::alpha(theme::ACCENT, 0.4)),
        (false, true) => (theme::UI_RAISED, theme::UI_LINE_STRONG),
        (false, false) => (Color32::TRANSPARENT, theme::UI_LINE),
    };
    painter.rect(row.rect, 8.0, fill, Stroke::new(1.0, border), StrokeKind::Inside);
    if row.has_focus() {
        painter.rect_stroke(row.rect.shrink(1.0), 7.0, Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
    }
}

/// Driver and lap time on top; car · track and the match chip below.
fn paint_row_text(ui: &Ui, content: Rect, entry: &LibraryEntry, driver: &str, status: MatchStatus) {
    let painter = ui.painter();
    let top = Rect::from_min_size(content.min, vec2(content.width(), 17.0));
    let bottom = Rect::from_x_y_ranges(content.x_range(), top.bottom()..=content.bottom());

    let time =
        painter.layout_no_wrap(format_lap_time(entry.lap_time), theme::font(Weight::SemiBold, 13.0), theme::UI_TEXT);
    let name_width = top.width() - time.size().x - 8.0;
    let name = widgets::single_line(ui, driver, theme::font(Weight::SemiBold, 13.0), theme::UI_TEXT, name_width);
    widgets::paint_galley(painter, top, Align2::LEFT_CENTER, name, theme::UI_TEXT);
    widgets::paint_galley(painter, top, Align2::RIGHT_CENTER, time, theme::UI_TEXT);

    let chip = StatusChip::short(ui, status);
    let chip_width = chip.as_ref().map_or(0.0, |c| c.width() + 8.0);
    let place = car_and_track(entry.car.as_deref(), entry.track.as_deref()).unwrap_or_default();
    let mut job =
        LayoutJob::single_section(place, TextFormat::simple(theme::font(Weight::Regular, 11.5), theme::UI_MUTED));
    job.wrap = TextWrapping::truncate_at_width(bottom.width() - chip_width);
    widgets::paint_galley(painter, bottom, Align2::LEFT_CENTER, painter.layout_job(job), theme::UI_MUTED);
    if let Some(chip) = chip {
        chip.paint(painter, pos2(bottom.right() - chip.width(), bottom.center().y));
    }
}

/// The last import error, with a dismiss button.
fn error_box(ui: &mut Ui, message: &str, actions: &mut Vec<PanelAction>) {
    const PAD: Vec2 = vec2(10.0, 8.0);
    const ICON: f32 = 13.0;
    const CLOSE: f32 = 20.0;
    let width = ui.available_width();
    let mut job = LayoutJob::default();
    job.append(message, ICON + 5.0, TextFormat::simple(theme::font(Weight::Regular, 12.0), theme::UI_TEXT));
    job.wrap = TextWrapping::wrap_at_width(width - 2.0 * PAD.x - CLOSE);
    let text = TextBlock::new(ui, job, LINE_SMALL);
    let (rect, _) = ui.allocate_exact_size(vec2(width, text.size().y + 2.0 * PAD.y), Sense::hover());

    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, theme::alpha(theme::DANGER, 0.1));
    let icon = Rect::from_min_size(rect.min + PAD + vec2(0.0, (LINE_SMALL - ICON) / 2.0), Vec2::splat(ICON));
    widgets::paint_icon(painter, icon, Icon::Warning, theme::DANGER, 2.0);
    text.paint(painter, rect.min + PAD);
    let close = Rect::from_min_size(pos2(rect.right() - 4.0 - CLOSE, rect.top() + 4.5), Vec2::splat(CLOSE));
    if ui.place(close, IconButton::new(Icon::Close, "Dismiss error").size(CLOSE, 12.0)).clicked() {
        actions.push(PanelAction::DismissError);
    }
}

// ---- Foot ----

/// "Reset to defaults" and the connection status.
fn foot(ui: &mut Ui, connection: ConnectionState, actions: &mut Vec<PanelAction>) {
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, FOOT_HEIGHT), Sense::hover());
    ui.painter().hline(rect.x_range().shrink(1.0), rect.top() + 0.5, Stroke::new(1.0, theme::UI_LINE));
    let content =
        Rect::from_min_size(pos2(rect.left() + PAD_X, rect.top() + 10.0), vec2(WIDTH - 2.0 * PAD_X, LINE_SMALL));
    let mut line = ui.new_child(UiBuilder::new().max_rect(content).layout(Layout::left_to_right(Align::Center)));
    if line.add(Link::new("Reset to defaults")).clicked() {
        actions.push(PanelAction::ResetAll);
    }
    connection_status(ui.painter(), content, connection);
}

/// `● Live`, `● Demo` or `○ Waiting for iRacing`, right-aligned in `rect`.
fn connection_status(painter: &Painter, rect: Rect, connection: ConnectionState) {
    let (text, dot) = match connection {
        ConnectionState::Live => ("Live", Some(theme::OK)),
        ConnectionState::Demo => ("Demo", Some(theme::WARN)),
        ConnectionState::Waiting => ("Waiting for iRacing", None),
    };
    let galley = painter.layout_no_wrap(text.to_owned(), theme::font(Weight::Regular, 11.0), theme::UI_MUTED);
    let center = pos2(rect.right() - galley.size().x - 5.0 - 3.5, rect.center().y);
    match dot {
        Some(color) => painter.circle_filled(center, 3.5, color),
        None => painter.circle_stroke(center, 3.0, Stroke::new(1.0, theme::UI_MUTED)),
    };
    widgets::paint_galley(painter, rect, Align2::RIGHT_CENTER, galley, theme::UI_MUTED);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use eframe::egui::output::OutputEvent;
    use eframe::egui::{Event, WidgetInfo};

    use super::*;
    use crate::demo;
    use crate::matching::{self, RefInfo};

    fn entry(id: &str, driver: &str, car: &str, track: &str, length_m: f64, start: (f64, f64)) -> LibraryEntry {
        LibraryEntry {
            id: id.into(),
            file: format!("{id}.csv"),
            original_name: format!("{driver}.csv"),
            driver: Some(driver.into()),
            car: Some(car.into()),
            track: Some(track.into()),
            lap_time: 116.5,
            samples: 7000,
            length_m: Some(length_m),
            start_latlon: Some(start),
            added: 1,
            last_used: 1,
        }
    }

    fn library() -> Library {
        let silverstone = (52.0683531, -1.0235284);
        Library {
            laps: vec![
                entry("a", "Ada", "Ferrari 296 GT3", "Silverstone Circuit (Grand Prix)", 5786.4, silverstone),
                entry("b", "Ben", "Porsche 911 GT3 R (992)", "Silverstone Circuit (Grand Prix)", 5786.4, silverstone),
                entry(
                    "c",
                    "Cy",
                    "Ferrari 296 GT3",
                    "Circuit de Spa-Francorchamps (Grand Prix Pits)",
                    6990.0,
                    (50.437, 5.971),
                ),
            ],
            active: Some("a".into()),
        }
    }

    /// Nothing loaded, no session.
    fn bare(library: &Library) -> PanelContext<'_> {
        PanelContext {
            library,
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

    /// A headless egui context with the app's fonts, `height` points tall.
    struct Harness {
        ctx: egui::Context,
        height: f32,
        /// Name and selected state of the widget that last gained keyboard focus.
        focused: RefCell<Option<(String, Option<bool>)>>,
    }

    impl Harness {
        fn new() -> Self {
            Self::tall(900.0)
        }

        fn tall(height: f32) -> Self {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx);
            Self { ctx, height, focused: RefCell::new(None) }
        }

        /// One frame. Pointer events hit the widgets laid out in the previous frame.
        fn frame(&self, settings: &mut Settings, cx: &PanelContext, events: Vec<Event>) -> PanelOutput {
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, self.height));
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut out = None;
            let mut full = self.ctx.run_ui(input, |ui| out = Some(show(ui, settings, cx)));
            full.textures_delta.clear();
            for event in full.platform_output.events {
                if let OutputEvent::FocusGained(WidgetInfo { label: Some(label), selected, .. }) = event {
                    *self.focused.borrow_mut() = Some((label, selected));
                }
            }
            out.expect("ran a frame")
        }

        /// Two frames to settle, then `events`.
        fn run(settings: &mut Settings, cx: &PanelContext, events: Vec<Event>) -> PanelOutput {
            let h = Self::new();
            h.settle(settings, cx);
            h.frame(settings, cx, events)
        }

        fn settle(&self, settings: &mut Settings, cx: &PanelContext) {
            self.frame(settings, cx, Vec::new());
            self.frame(settings, cx, Vec::new());
        }

        /// Presses Tab until the widget named `label` has keyboard focus; returns
        /// whether it's selected (for list rows).
        fn focus(&self, settings: &mut Settings, cx: &PanelContext, label: &str) -> Option<bool> {
            for _ in 0..40 {
                self.frame(settings, cx, vec![key(Key::Tab, Modifiers::NONE)]);
                if let Some((name, selected)) = self.focused.borrow().clone()
                    && name == label
                {
                    return selected;
                }
            }
            panic!("nothing called {label:?} takes focus");
        }

        /// Names of the widgets Tab reaches, in order.
        fn focusable(&self, settings: &mut Settings, cx: &PanelContext) -> Vec<String> {
            let mut names: Vec<String> = Vec::new();
            for _ in 0..40 {
                self.frame(settings, cx, vec![key(Key::Tab, Modifiers::NONE)]);
                let Some((name, _)) = self.focused.borrow().clone() else { continue };
                if names.first() == Some(&name) {
                    break;
                }
                names.push(name);
            }
            names
        }
    }

    fn click(pos: Pos2) -> Vec<Event> {
        let button = |pressed| Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        vec![Event::PointerMoved(pos), button(true), button(false)]
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
    }

    fn escape() -> Vec<Event> {
        vec![key(Key::Escape, Modifiers::NONE)]
    }

    fn enter() -> Vec<Event> {
        vec![key(Key::Enter, Modifiers::NONE)]
    }

    #[test]
    fn every_tab_and_state_renders() {
        let lap = demo::sample_lap();
        let session = demo::demo_session(&lap);
        let lib = library();
        let empty = Library::default();
        let status = |session| matching::status(&RefInfo::from_lap(&lap), session);
        assert_eq!(status(Some(&session)), MatchStatus::Match);
        // (card, library id, error, session): matching, waiting behind the demo,
        // bundled without a session, nothing.
        let states = [
            (Some(RefCard::Saved(&lap, status(Some(&session)))), Some("a"), None, Some(&session)),
            (Some(RefCard::Waiting(&lap)), Some("c"), None, Some(&session)),
            (Some(RefCard::Bundled(&lap, status(None))), None, Some("Missing column: Brake."), None),
            (None, None, None, Some(&session)),
        ];
        for tab in TABS.map(|(t, _)| t) {
            for (card, active_id, error, session) in states {
                let cx = PanelContext {
                    library: if card.is_some() { &lib } else { &empty },
                    session,
                    card,
                    active_id,
                    error,
                    hotkey_error: error,
                    connection: ConnectionState::Live,
                    browsing: error.is_some(),
                    file_hover: error.is_some(),
                    max_height: f32::INFINITY,
                };
                let axis = if active_id.is_some() { Axis::Distance } else { Axis::Time };
                let mut s = Settings { tab, labels: error.is_none(), axis, ..Default::default() };
                let out = Harness::run(&mut s, &cx, Vec::new());
                assert!(out.desired_height > 150.0 && out.desired_height < 900.0, "{tab:?}: {}", out.desired_height);
                assert!(out.actions.is_empty());
            }
        }
    }

    #[test]
    fn display_tab_is_as_tall_as_the_prototype_plus_its_extra_rows() {
        let lib = Library::default();
        let out = Harness::run(&mut Settings::default(), &bare(&lib), Vec::new());
        // Prototype: 378 px. Here the demo switch adds a row (18 + 13), the lock hint a
        // second line (17) and the shortcut picker a labelled row (13 + 18 + 6 + 30).
        assert!((out.desired_height - (378.0 + 31.0 + 17.0 + 67.0)).abs() <= 3.0, "{}", out.desired_height);
    }

    #[test]
    fn close_button_and_escape_close() {
        let lib = Library::default();
        let out = Harness::run(&mut Settings::default(), &bare(&lib), click(pos2(WIDTH - 22.0, 24.0)));
        assert_eq!(out.actions, vec![PanelAction::Close]);
        let out = Harness::run(&mut Settings::default(), &bare(&lib), escape());
        assert_eq!(out.actions, vec![PanelAction::Close]);
    }

    #[test]
    fn clicking_a_tab_selects_it() {
        let lib = Library::default();
        let mut s = Settings::default();
        // Tabs start 9 px in; "Display" is about 50 px wide, so this lands on "Labels".
        Harness::run(&mut s, &bare(&lib), click(pos2(9.0 + 60.0 + 20.0, 41.0 + 18.0)));
        assert_eq!(s.tab, SettingsTab::Labels);
    }

    #[test]
    fn picking_a_shortcut_reports_it_even_when_unchanged() {
        // Registering it may have failed before; picking it again must retry.
        let lib = Library::default();
        let cx = bare(&lib);
        let mut s = Settings::default();
        let h = Harness::new();
        h.settle(&mut s, &cx);
        h.focus(&mut s, &cx, "Ctrl+Alt+Shift+O");
        assert!(h.frame(&mut s, &cx, enter()).actions.is_empty(), "Enter starts listening");
        let ctrl_alt_shift = Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT;
        let out = h.frame(&mut s, &cx, vec![Event::ModifiersChanged(ctrl_alt_shift), key(Key::O, ctrl_alt_shift)]);
        assert_eq!(out.actions, vec![PanelAction::SetHotkey("Ctrl+Alt+Shift+O".into())]);
        assert_eq!(s.unlock_hotkey, Settings::default().unlock_hotkey, "the app applies it");
    }

    /// Vertical centre of saved-lap row `i` on the Reference tab when no lap is drawn:
    /// head, tabs, padding, section heading, then rows.
    fn lap_row_center(i: usize) -> f32 {
        41.0 + 37.0 + GAP + 11.0 + (GAP - 6.0) + i as f32 * (LAP_ROW_HEIGHT + ROW_GAP) + LAP_ROW_HEIGHT / 2.0
    }

    /// Row `i`'s × (and "Remove?" once asked).
    fn lap_row_remove(i: usize) -> Pos2 {
        pos2(WIDTH - PAD_X - 6.0 - 11.0, lap_row_center(i))
    }

    #[test]
    fn saved_laps_select_and_remove_after_confirming() {
        let lib = library();
        let cx = bare(&lib);
        let mut s = Settings { tab: SettingsTab::Reference, ..Default::default() };
        let h = Harness::new();
        h.settle(&mut s, &cx);

        // Unknown session: library order. Clicking the row selects it.
        let out = h.frame(&mut s, &cx, click(pos2(60.0, lap_row_center(1))));
        assert_eq!(out.actions, vec![PanelAction::SelectLap("b".into())]);

        // × asks first; "Remove?" (in the same spot) removes.
        let x = lap_row_remove(1);
        assert!(h.frame(&mut s, &cx, click(x)).actions.is_empty());
        h.frame(&mut s, &cx, Vec::new());
        assert_eq!(h.frame(&mut s, &cx, click(x)).actions, vec![PanelAction::RemoveLap("b".into())]);

        // Escape cancels a pending "Remove?" without closing the window.
        h.frame(&mut s, &cx, click(x));
        assert!(h.frame(&mut s, &cx, escape()).actions.is_empty());
        h.frame(&mut s, &cx, Vec::new());
        assert!(h.frame(&mut s, &cx, click(x)).actions.is_empty(), "× asks again");
    }

    #[test]
    fn selecting_another_lap_cancels_a_pending_remove() {
        let lib = library();
        let cx = bare(&lib);
        let mut s = Settings { tab: SettingsTab::Reference, ..Default::default() };
        let h = Harness::new();
        h.settle(&mut s, &cx);
        let x = lap_row_remove(1);
        assert!(h.frame(&mut s, &cx, click(x)).actions.is_empty(), "B: Remove?");
        h.frame(&mut s, &cx, Vec::new());
        let out = h.frame(&mut s, &cx, click(pos2(60.0, lap_row_center(0))));
        assert_eq!(out.actions, vec![PanelAction::SelectLap("a".into())]);
        h.frame(&mut s, &cx, Vec::new());
        assert!(h.frame(&mut s, &cx, click(x)).actions.is_empty(), "× asks again");
    }

    #[test]
    fn reset_forgets_a_pending_remove_and_a_shortcut_being_picked() {
        let lib = library();
        let cx = bare(&lib);
        let mut s = Settings { tab: SettingsTab::Reference, ..Default::default() };
        let h = Harness::new();
        h.settle(&mut s, &cx);
        let x = lap_row_remove(1);
        h.frame(&mut s, &cx, click(x));
        reset(&h.ctx);
        h.frame(&mut s, &cx, Vec::new());
        assert!(h.frame(&mut s, &cx, click(x)).actions.is_empty(), "× asks again");

        s.tab = SettingsTab::Display;
        h.settle(&mut s, &cx);
        h.focus(&mut s, &cx, "Ctrl+Alt+Shift+O");
        h.frame(&mut s, &cx, enter());
        reset(&h.ctx);
        let out = h.frame(&mut s, &cx, vec![key(Key::K, Modifiers::CTRL | Modifiers::ALT)]);
        assert!(out.actions.is_empty(), "no longer listening: {:?}", out.actions);
    }

    #[test]
    fn the_card_names_the_chosen_lap_and_only_a_choice_can_be_removed() {
        let lap = demo::sample_lap();
        let session = demo::demo_session(&lap);
        let lib = library();
        let mut s = Settings { tab: SettingsTab::Reference, ..Default::default() };

        // Spa lap chosen, demo running: its card, Remove clears the choice, its row is marked.
        let waiting = PanelContext { card: Some(RefCard::Waiting(&lap)), active_id: Some("c"), ..bare(&lib) };
        let waiting = PanelContext { session: Some(&session), connection: ConnectionState::Demo, ..waiting };
        let h = Harness::new();
        h.settle(&mut s, &waiting);
        assert!(h.focusable(&mut s, &waiting).contains(&"Remove".to_owned()));
        assert_eq!(h.focus(&mut s, &waiting, "Cy"), Some(true), "the chosen row is marked");
        h.focus(&mut s, &waiting, "Remove");
        assert_eq!(h.frame(&mut s, &waiting, enter()).actions, vec![PanelAction::ClearReference]);

        // Nothing chosen: the bundled lap has nothing to remove, and no row is marked.
        let bundled = PanelContext { card: Some(RefCard::Bundled(&lap, MatchStatus::Match)), ..waiting };
        let bundled = PanelContext { active_id: None, ..bundled };
        let h = Harness::new();
        h.settle(&mut s, &bundled);
        let names = h.focusable(&mut s, &bundled);
        assert!(names.contains(&"Replace…".to_owned()) && !names.contains(&"Remove".to_owned()), "{names:?}");
        assert_eq!(h.focus(&mut s, &bundled, "Cy"), Some(false));
    }

    #[test]
    fn waiting_note_says_when_the_lap_is_drawn() {
        let lap = demo::sample_lap();
        let session = demo::demo_session(&lap);
        assert_eq!(
            waiting_note(&lap, Some(&session)),
            "Drawn when you drive Silverstone Circuit (Grand Prix). \
             The demo runs Silverstone Circuit with the bundled lap."
        );
    }

    #[test]
    fn a_tab_taller_than_the_window_scrolls() {
        let lap = demo::sample_lap();
        let mut lib = library();
        for i in 0..4 {
            let id = format!("x{i}");
            lib.laps.push(entry(&id, "Dee", "BMW M4 GT3", "Silverstone Circuit (Grand Prix)", 5786.4, (52.0, -1.0)));
        }
        let error = "That isn't a CSV. In Garage 61, open the lap and export it as CSV.";
        let cx = PanelContext {
            card: Some(RefCard::Saved(&lap, MatchStatus::Unknown)),
            active_id: Some("a"),
            error: Some(error),
            ..bare(&lib)
        };
        let mut s = Settings { tab: SettingsTab::Reference, ..Default::default() };
        let natural = Harness::run(&mut s, &cx, Vec::new()).desired_height;
        assert!(natural > 600.0, "{natural}");

        let capped = PanelContext { max_height: 600.0, ..cx };
        let h = Harness::tall(600.0);
        h.settle(&mut s, &capped);
        // It still asks for its full height (the window may grow back), and the foot
        // stays at the bottom of the window, where it can be clicked.
        let reset_link = pos2(PAD_X + 20.0, 600.0 - FOOT_HEIGHT + 10.0 + LINE_SMALL / 2.0);
        let out = h.frame(&mut s, &capped, click(reset_link));
        assert_eq!(out.desired_height, natural);
        assert_eq!(out.actions, vec![PanelAction::ResetAll]);
    }

    #[test]
    fn browse_is_disabled_while_a_dialog_is_open() {
        let lib = Library::default();
        let mut s = Settings { tab: SettingsTab::Reference, ..Default::default() };
        // The full drop zone starts right under the tabs.
        let drop_zone = pos2(WIDTH / 2.0, 41.0 + 37.0 + GAP + 40.0);
        let out = Harness::run(&mut s, &bare(&lib), click(drop_zone));
        assert_eq!(out.actions, vec![PanelAction::Browse]);
        let browsing = PanelContext { browsing: true, ..bare(&lib) };
        assert!(Harness::run(&mut s, &browsing, click(drop_zone)).actions.is_empty());
    }

    #[test]
    fn formats_card_text() {
        assert_eq!(thousands(6959), "6,959");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(thousands(42), "42");
        assert_eq!(car_and_track(Some("Ferrari 296 GT3"), Some("Spa")).as_deref(), Some("Ferrari 296 GT3 · Spa"));
        assert_eq!(car_and_track(None, Some("Spa")).as_deref(), Some("Spa"));
        assert_eq!(car_and_track(None, None), None);
        let lap = demo::sample_lap();
        assert!(lap_stats(&lap).starts_with(&format!("{} samples · 60 Hz · ", thousands(lap.n()))));
    }
}
