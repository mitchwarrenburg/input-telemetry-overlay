//! The brake point window: a bar that counts 3-2-1-BRAKE into each reference brake
//! point, the zone's target peak, and how your timing compared. Compact, it's just the
//! bar with a readout inside. A port of the prototype's `BrakeCueView`
//! (`prototype/src/brake-cue.js`; sizes and colours from `prototype/src/styles.css`).

use std::sync::Arc;

use eframe::egui::{
    Color32, CornerRadius, CursorIcon, Galley, Painter, PointerButton, Pos2, Rect, ResizeDirection, Sense, Shadow,
    Shape, Stroke, StrokeKind, Ui, Vec2, pos2, text::LayoutJob, vec2,
};

use super::overlay::{self, BUTTON_GAP, Chrome, GEAR_SIZE, Intent, PAD_LEFT};
use super::theme::{self, Weight};
use super::widgets;
use crate::cue::{CueMode, CueState, Grade, Pip};

/// Panel size on first launch and after a layout reset.
pub const DEFAULT_PANEL: Vec2 = vec2(360.0, 96.0);
/// Compact panel height on first collapse.
pub const COMPACT_HEIGHT: f32 = 40.0;
/// Smallest panels: the full one keeps its bar at [`BAR_MIN`] tall with the header and
/// the timing row; the compact one is just the bar.
pub const MIN_PANEL: Vec2 = vec2(230.0, 96.0);
pub const MIN_COMPACT: Vec2 = vec2(180.0, 30.0);
/// The pulse's length, seconds.
pub const PULSE_S: f32 = 0.55;
/// Each count pops in over this, seconds.
pub const POP_S: f32 = 0.22;

/// The bar never gets shorter than this in the full window, so the count, BRAKE and the
/// gauge always have room.
const BAR_MIN: f32 = 34.0;
/// The body inside the panel's border: below the header, and its side and bottom insets.
const BODY_TOP: f32 = 27.0;
const BODY_SIDE: f32 = 10.0;
const BODY_BOTTOM: f32 = 8.0;
const COMPACT_INSET: f32 = 4.0;
const VERDICT_H: f32 = 18.0;
const ROW_GAP: f32 = 7.0;
/// Between the bar and its cap.
const CAP_GAP: f32 = 3.0;
/// Before the gauge, and between it and the target text.
const TARGET_GAP: f32 = 7.0;
const GAUGE_W: f32 = 6.0;
const TRACK_RADIUS: CornerRadius = CornerRadius { nw: 5, ne: 1, sw: 5, se: 1 };
const CAP_RADIUS: CornerRadius = CornerRadius { nw: 1, ne: 5, sw: 1, se: 5 };
/// Barlow's capitals are 0.7 em tall: text is centred on them, not on its line box.
const CAP_HEIGHT: f32 = 0.7;
/// Header items are hidden as the window narrows: pips, then the distance, then the
/// timing row's distance, then its peak.
const PIPS_MIN_W: f32 = 330.0;
const NEXT_MIN_W: f32 = 290.0;
const PEAK_MIN_W: f32 = 250.0;
/// The compact readout drops its labels at this width.
const LABELS_MIN_W: f32 = 250.0;

/// Solid red behind white text (4.5:1).
const BRAKE_INK: Color32 = theme::BRAKE_PILL;
/// Your peak so far, on the gauge.
const PEAK_MARK: Color32 = Color32::from_rgb(0xff, 0x9b, 0x91);
/// The cap's outline and text before the count.
const CAP_LINE: Color32 = Color32::from_rgba_premultiplied(36, 36, 36, 36); // white @ 14%
const CAP_TEXT: Color32 = Color32::from_rgba_premultiplied(93, 96, 95, 102); // #e8efee @ 40%

/// The prototype's collapse and expand icons (24 × 24 view box), as polylines.
const COLLAPSE_ICON: [&[(f32, f32)]; 4] = [
    &[(4.0, 14.0), (10.0, 14.0), (10.0, 20.0)],
    &[(20.0, 10.0), (14.0, 10.0), (14.0, 4.0)],
    &[(14.0, 10.0), (21.0, 3.0)],
    &[(3.0, 21.0), (10.0, 14.0)],
];
const EXPAND_ICON: [&[(f32, f32)]; 4] = [
    &[(15.0, 3.0), (21.0, 3.0), (21.0, 9.0)],
    &[(9.0, 21.0), (3.0, 21.0), (3.0, 15.0)],
    &[(21.0, 3.0), (14.0, 10.0)],
    &[(3.0, 21.0), (10.0, 14.0)],
];

/// What the window shows this frame.
pub struct CueFrame<'a> {
    pub state: &'a CueState,
    /// Lock, settings, pulse and the resize readout; `opacity` is the background's.
    pub chrome: Chrome<'a>,
    pub compact: bool,
    /// Everything drawn on the panel (bar, text, buttons), 0..1.
    pub contents: f32,
    /// How far the current count's pop-in has run, 0..1.
    pub pop: f32,
}

/// Something the user started this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CueIntent {
    Move,
    Resize(ResizeDirection),
    ToggleSettings,
    /// Hide the window.
    Close,
    /// Collapse to the bar (true) or expand again.
    Compact(bool),
}

/// Colours that follow the background's opacity (see [`theme::scrim`]).
#[derive(Clone, Copy)]
struct Look {
    fade: f32,
    scrim: Color32,
    muted: Color32,
}

/// Paints the window and senses its controls.
pub fn show(ui: &Ui, window: Rect, f: &CueFrame) -> Option<CueIntent> {
    let chrome = &f.chrome;
    overlay::paint_panel(ui.painter(), window, chrome.opacity, chrome.pulse);
    let fade = 1.0 - chrome.opacity.clamp(0.0, 1.0);
    let look = Look { fade, scrim: theme::scrim(fade), muted: theme::muted(fade) };
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(f.contents.clamp(0.0, 1.0));
    let content = overlay::content_rect(window);
    let target_w = if f.compact { 0.0 } else { target_width(&painter, content.height()) };
    let layout = Layout::new(window, f.compact, target_w);
    let hover = overlay::hover_fade(ui, window, chrome);

    let mut intent = None;
    if f.compact {
        // No header: the whole bar moves the window, except from the expand button, which
        // a move would take the click from.
        if !chrome.locked {
            let drag = ui.interact(layout.panel, ui.id().with("cue-move"), Sense::drag());
            let on_expand = ui.input(|i| i.pointer.press_origin()).is_some_and(|p| expand_rect(content).contains(p));
            if drag.on_hover_cursor(CursorIcon::Grab).drag_started_by(PointerButton::Primary) && !on_expand {
                intent = Some(CueIntent::Move);
            }
        }
    } else {
        intent = header(ui, &painter, window, f, look, hover);
    }

    paint_bar(&painter, &layout, f.state, look, f.pop);
    if f.compact {
        paint_readout(&painter, layout.track, content, f.state, look);
        if !chrome.locked && hover > 0.0 && expand_button(ui, &painter, content, hover) {
            intent = Some(CueIntent::Compact(false));
        }
    } else {
        if let (Some(gauge), Some(text)) = (layout.gauge, layout.target) {
            paint_target(&painter, gauge, text, content.height(), f.state, look);
        }
        if let Some(row) = layout.verdict {
            paint_verdict(&painter, row, content.width(), f.state, look);
        }
    }

    let frame = overlay::frame_controls(ui, window, chrome).map(|i| match i {
        Intent::Move => CueIntent::Move,
        Intent::Resize(dir) => CueIntent::Resize(dir),
        Intent::ToggleSettings => CueIntent::ToggleSettings,
        Intent::Close => CueIntent::Close,
    });
    frame.or(intent)
}

/// The pulse's strength `age` seconds after the brake point: up fast, then fading, like
/// the prototype's keyframes (opacity 1 at 12 %, eased out).
pub fn pulse_alpha(age: f32) -> f32 {
    let t = age / PULSE_S;
    if !(0.0..1.0).contains(&t) {
        return 0.0;
    }
    const PEAK: f32 = 0.12;
    if t < PEAK { ease_out(t / PEAK) } else { 1.0 - ease_out((t - PEAK) / (1.0 - PEAK)) }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Where everything goes.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Layout {
    panel: Rect,
    track: Rect,
    cap: Rect,
    /// The cap's base font size.
    em: f32,
    /// Full window only: the target gauge, the target text beside it, and the timing row.
    gauge: Option<Rect>,
    target: Option<Rect>,
    verdict: Option<Rect>,
}

impl Layout {
    /// `target_w`: the target text's width (see [`target_width`]).
    fn new(window: Rect, compact: bool, target_w: f32) -> Self {
        let panel = overlay::panel_rect(window);
        let content = overlay::content_rect(window);
        let (w, h) = (content.width(), content.height());
        if compact {
            let body = content.shrink(COMPACT_INSET);
            let cap_w = (0.13 * w).clamp(40.0, 72.0);
            let cap = Rect::from_min_max(pos2(body.right() - cap_w, body.top()), body.max);
            let track = Rect::from_min_max(body.min, pos2(cap.left() - CAP_GAP, body.bottom()));
            let em = (0.34 * h).clamp(10.0, 16.0);
            return Self { panel, track, cap, em, gauge: None, target: None, verdict: None };
        }
        let body =
            Rect::from_min_max(content.min + vec2(BODY_SIDE, BODY_TOP), content.max - vec2(BODY_SIDE, BODY_BOTTOM));
        let verdict = Rect::from_min_max(pos2(body.left(), body.bottom() - VERDICT_H), body.max);
        let main_bottom = (verdict.top() - ROW_GAP).max(body.top() + BAR_MIN);
        let main = Rect::from_min_max(body.min, pos2(body.right(), main_bottom));
        let target = Rect::from_min_max(pos2(main.right() - target_w, main.top()), main.max);
        let gauge_right = target.left() - TARGET_GAP;
        let gauge = Rect::from_min_max(pos2(gauge_right - GAUGE_W, main.top()), pos2(gauge_right, main.bottom()));
        let cap_w = (0.17 * w).clamp(58.0, 110.0);
        let cap_right = gauge.left() - TARGET_GAP;
        let cap = Rect::from_min_max(pos2(cap_right - cap_w, main.top()), pos2(cap_right, main.bottom()));
        let track = Rect::from_min_max(main.min, pos2(cap.left() - CAP_GAP, main.bottom()));
        let em = (0.15 * h).clamp(12.0, 20.0);
        Self { panel, track, cap, em, gauge: Some(gauge), target: Some(target), verdict: Some(verdict) }
    }
}

// ---- Text ----

fn galley(painter: &Painter, text: &str, weight: Weight, size: f32, tracking: f32, color: Color32) -> Arc<Galley> {
    overlay::text_galley(painter, text, weight, size, tracking, color)
}

/// Where a one-line galley's baseline is, from its top.
fn baseline(galley: &Galley) -> f32 {
    galley.rows.first().and_then(|r| r.row.glyphs.first().map(|g| r.pos.y + g.pos.y)).unwrap_or(0.8 * galley.size().y)
}

/// The top-left that puts a galley's capitals (at font `size`) centred on `y`.
fn caps_top(galley: &Galley, size: f32, y: f32) -> f32 {
    y + CAP_HEIGHT * size / 2.0 - baseline(galley)
}

/// Paints one line of text with its capitals centred on `center`.
fn paint_centred(painter: &Painter, center: Pos2, galley: Arc<Galley>, size: f32, color: Color32, halo: f32) {
    let pos = pos2(center.x - galley.size().x / 2.0, caps_top(&galley, size, center.y));
    overlay::halo_galley(painter, pos, galley, color, halo);
}

/// Paints one line of text from `left`, its capitals centred on `y`. Returns its right edge.
fn paint_left(painter: &Painter, left: f32, y: f32, galley: Arc<Galley>, size: f32, color: Color32, halo: f32) -> f32 {
    let right = left + galley.size().x;
    overlay::halo_galley(painter, pos2(left, caps_top(&galley, size, y)), galley, color, halo);
    right
}

fn pct(v: f32) -> String {
    format!("{}%", (v * 100.0).round() as i32)
}

/// "+0.04 s", "−3 m"; no sign when it rounds to zero.
fn signed(v: f64, digits: usize, unit: &str) -> String {
    let text = format!("{:.digits$}", v.abs());
    let zero = text.parse::<f64>().is_ok_and(|r| r == 0.0);
    let sign = if zero {
        ""
    } else if v < 0.0 {
        "\u{2212}"
    } else {
        "+"
    };
    format!("{sign}{text}{unit}")
}

/// "240 m", "1.2 km".
fn dist_text(m: f64) -> String {
    if m >= 1000.0 { format!("{:.1} km", m / 1000.0) } else { format!("{} m", m.max(0.0).round() as i64) }
}

/// Your peak against the target, in whole percent: "+4", "−3", "±0".
fn peak_diff(peak: f32, target: f32) -> String {
    let diff = (peak * 100.0).round() as i32 - (target * 100.0).round() as i32;
    if diff == 0 { "±0".to_owned() } else { signed(f64::from(diff), 0, "") }
}

// ---- Header ----

/// Title, the zone ahead, the zone pips and the collapse, gear and close buttons.
fn header(ui: &Ui, painter: &Painter, window: Rect, f: &CueFrame, look: Look, hover: f32) -> Option<CueIntent> {
    let (state, chrome) = (f.state, &f.chrome);
    let header = overlay::header_rect(window);
    let content = overlay::content_rect(window);
    let width = content.width();
    let cy = header.center().y;
    let close = overlay::close_rect(header);
    let step = vec2(-(GEAR_SIZE + BUTTON_GAP), 0.0);
    let gear = close.translate(step);
    let collapse = gear.translate(step);

    if hover > 0.0 {
        overlay::paint_grip(painter, content, hover);
    }
    let title = galley(painter, "BRAKE POINT", Weight::Bold, 11.0, 0.07, theme::HUD_TEXT);
    let mut right = paint_left(painter, header.left() + PAD_LEFT, cy, title, 11.0, theme::HUD_TEXT, look.fade);
    if width > NEXT_MIN_W && state.zone_no > 0 {
        let mut text = format!("Z{}", state.zone_no);
        if state.mode == CueMode::Countdown {
            text += &format!(" · {}", dist_text(state.dist));
        }
        let next = galley(painter, &text, Weight::SemiBold, 10.5, 0.04, look.muted);
        right = paint_left(painter, right + 10.0, cy, next, 10.5, look.muted, look.fade);
    }
    if width > PIPS_MIN_W {
        paint_pips(painter, right + 10.0, collapse.left() - 10.0, cy, &state.pips, look);
    }

    let mut intent = None;
    if !chrome.locked {
        let drag_area = header.with_max_x(collapse.left() - 4.0);
        let drag = ui.interact(drag_area, ui.id().with("cue-header"), Sense::drag());
        if drag.on_hover_cursor(CursorIcon::Grab).drag_started_by(PointerButton::Primary) {
            intent = Some(CueIntent::Move);
        }
    }
    let button = |rect: Rect, name: &str, tip: &str| {
        ui.interact(rect, ui.id().with(name), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(tip)
    };
    let collapsing = button(collapse, "cue-collapse", "Collapse to the bar");
    if collapsing.clicked() {
        intent = Some(CueIntent::Compact(true));
    }
    paint_icon_button(painter, collapse, collapsing.hovered(), &COLLAPSE_ICON);

    let settings = button(gear, "cue-gear", "Brake point settings");
    if settings.clicked() {
        intent = Some(CueIntent::ToggleSettings);
    }
    let turn = ui.ctx().animate_bool_with_time_and_easing(
        ui.id().with("cue-gear-turn"),
        chrome.settings_open,
        0.3,
        eframe::egui::emath::easing::cubic_out,
    );
    overlay::paint_gear(painter, gear, settings.hovered(), chrome.settings_open, turn);

    let closing = button(close, "cue-close", "Hide the brake point window");
    if closing.clicked() {
        intent = Some(CueIntent::Close);
    }
    overlay::paint_close(painter, close, closing.hovered());
    intent
}

/// One pip per counted zone, right-aligned against `right`: your grade there (last lap's
/// dimmed), a ring on the zone ahead. Skipped when they don't fit after `left`.
fn paint_pips(painter: &Painter, left: f32, right: f32, cy: f32, pips: &[Pip], look: Look) {
    const W: f32 = 9.0;
    const H: f32 = 5.0;
    const GAP: f32 = 3.0;
    let n = pips.len() as f32;
    let start = right - (n * W + (n - 1.0).max(0.0) * GAP);
    if pips.is_empty() || start < left {
        return;
    }
    for (i, pip) in pips.iter().enumerate() {
        let rect = Rect::from_min_size(pos2(start + i as f32 * (W + GAP), cy - H / 2.0), vec2(W, H));
        let a = if pip.stale && !pip.current { 0.35 } else { 1.0 };
        if pip.current {
            painter.rect_filled(rect.expand(2.5), 4.0, theme::HUD_TEXT);
            painter.rect_filled(rect.expand(1.5), 3.0, theme::SURFACE);
        }
        match pip.grade {
            Some(g) => {
                painter.rect_filled(rect, 1.5, theme::alpha(theme::grade_color(g), a));
            }
            None => {
                painter.rect_filled(rect, 1.5, theme::alpha(look.scrim, a));
                painter.rect_filled(rect, 1.5, theme::alpha(Color32::WHITE, 0.14 * a));
            }
        }
    }
}

/// A header button with one of the prototype's line icons, styled like the gear.
fn paint_icon_button(painter: &Painter, rect: Rect, hovered: bool, icon: &[&[(f32, f32)]]) {
    let (color, background) =
        if hovered { (theme::HUD_TEXT, theme::UI_LINE) } else { (theme::HUD_MUTED, Color32::TRANSPARENT) };
    painter.rect_filled(rect, 6.0, background);
    paint_icon(painter, Rect::from_center_size(rect.center(), Vec2::splat(14.0)), icon, 1.8, color);
}

fn paint_icon(painter: &Painter, rect: Rect, icon: &[&[(f32, f32)]], width: f32, color: Color32) {
    let s = rect.width() / 24.0;
    let stroke = Stroke::new(width * s, color);
    for line in icon {
        painter.add(Shape::line(line.iter().map(|&(x, y)| rect.min + vec2(x, y) * s).collect(), stroke));
    }
}

/// Where compact mode's expand button goes: 7 pt in from the bar's right end.
fn expand_rect(content: Rect) -> Rect {
    Rect::from_center_size(pos2(content.right() - 7.0 - 11.0, content.center().y), Vec2::splat(22.0))
}

/// Compact mode's expand button, at the bar's right end while hovered. True when clicked.
fn expand_button(ui: &Ui, painter: &Painter, content: Rect, hover: f32) -> bool {
    let rect = expand_rect(content);
    let resp = ui
        .interact(rect, ui.id().with("cue-expand"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("Expand");
    painter.rect(
        rect,
        6.0,
        theme::alpha(theme::SURFACE, 0.92 * hover),
        Stroke::new(1.0, theme::alpha(theme::ACCENT, 0.6 * hover)),
        StrokeKind::Inside,
    );
    let color = if resp.hovered() { theme::ACCENT } else { theme::HUD_TEXT };
    paint_icon(
        painter,
        Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
        &EXPAND_ICON,
        1.9,
        theme::alpha(color, hover),
    );
    resp.clicked()
}

// ---- The bar ----

/// The track with its fill, the join hatching and the count ticks, then the cap.
fn paint_bar(painter: &Painter, l: &Layout, s: &CueState, look: Look, pop: f32) {
    let track = l.track;
    let flash = s.flash.filter(|f| f.alpha > 0.0);
    let perfect = flash.filter(|f| f.grade == Grade::Perfect);

    // Glows first, around (not under) the bar and its cap.
    let glow = |blur: u8, color: Color32| {
        let shadow = Shadow { offset: [0, 0], blur, spread: 0, color };
        overlay::outer_shadow(painter, track, TRACK_RADIUS, shadow);
        overlay::outer_shadow(painter, l.cap, CAP_RADIUS, shadow);
    };
    if let Some(f) = perfect {
        glow(14, theme::alpha(theme::grade_color(Grade::Perfect), 0.8 * f.alpha));
    } else if s.mode == CueMode::Brake {
        glow(10, theme::alpha(theme::BRAKE, 0.5));
    }

    painter.rect_filled(track, TRACK_RADIUS, look.scrim);
    painter.rect_filled(track, TRACK_RADIUS, theme::alpha(Color32::WHITE, 0.07));
    painter.rect_stroke(track, TRACK_RADIUS, Stroke::new(1.0, theme::alpha(Color32::WHITE, 0.08)), StrokeKind::Inside);

    let (join, fill) = (s.join.clamp(0.0, 1.0), s.fill.clamp(0.0, 1.0));
    match flash {
        Some(f) if f.grade == Grade::Perfect => {
            let c = theme::alpha(theme::grade_color(Grade::Perfect), f.alpha);
            fill_span(painter, track, 0.0, 1.0, |_| c);
        }
        Some(f) => {
            let c = theme::alpha(theme::grade_color(f.grade), f.alpha);
            fill_span(painter, track, join, fill, |_| c);
        }
        None if s.mode == CueMode::Brake => fill_span(painter, track, join, fill, |_| theme::BRAKE),
        // After the flash, the bar stays empty while you brake; the gauge has your pressure.
        None if s.mode == CueMode::Braking => {}
        None => fill_span(painter, track, join, fill, count_color),
    }
    if join > 0.001 && perfect.is_none() {
        paint_hatch(painter, track.with_max_x(track.left() + join * track.width()));
    }
    for k in [1.0, 2.0] {
        let x = track.left() + k * track.width() / 3.0;
        let tick = Rect::from_min_max(pos2(x - 1.0, track.top()), pos2(x + 1.0, track.bottom()));
        painter.rect_filled(tick, 0.0, theme::alpha(theme::SURFACE, 0.75));
    }
    if l.verdict.is_some()
        && let Some(message) = track_message(s)
    {
        let size = (0.11 * (l.panel.height() - 2.0)).clamp(10.0, 13.0);
        let text = galley(painter, &message, Weight::SemiBold, size, 0.08, look.muted);
        paint_centred(painter, track.center(), text, size, look.muted, look.fade);
    }
    paint_cap(painter, l, s, look, pop);
}

/// The count's red, lighter at the start of the bar and darker at the brake point.
fn count_color(x: f32) -> Color32 {
    const MID: f32 = 0.45;
    if x < MID {
        theme::COUNT_LIGHT.lerp_to_gamma(theme::BRAKE, x / MID)
    } else {
        theme::BRAKE.lerp_to_gamma(theme::COUNT_DEEP, (x - MID) / (1.0 - MID))
    }
}

/// Fills the track from fraction `a` to `b` of its width, following its rounded left
/// corners, coloured by position along the whole track (so a gradient doesn't squash as
/// the fill grows).
fn fill_span(painter: &Painter, track: Rect, a: f32, b: f32, color_at: impl Fn(f32) -> Color32) {
    let (w, r) = (track.width(), f32::from(TRACK_RADIUS.nw));
    if b - a <= 0.0 || w <= 0.0 {
        return;
    }
    let mut xs = vec![a, b, 0.45];
    xs.extend((0..=10).map(|i| i as f32 * r / 10.0 / w));
    xs.retain(|&x| (a..=b).contains(&x));
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    let mut mesh = eframe::egui::Mesh::default();
    for (i, &x) in xs.iter().enumerate() {
        let dx = x * w;
        let inset = if dx < r { r - (r * r - (r - dx) * (r - dx)).max(0.0).sqrt() } else { 0.0 };
        let px = track.left() + dx;
        let c = color_at(x);
        mesh.colored_vertex(pos2(px, track.top() + inset), c);
        mesh.colored_vertex(pos2(px, track.bottom() - inset), c);
        if i > 0 {
            let n = 2 * i as u32;
            mesh.add_triangle(n - 2, n - 1, n);
            mesh.add_triangle(n - 1, n + 1, n);
        }
    }
    painter.add(mesh);
}

/// The part a short straight skipped: faint diagonal stripes.
fn paint_hatch(painter: &Painter, rect: Rect) {
    if rect.width() <= 0.0 {
        return;
    }
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let stroke = Stroke::new(2.0, theme::alpha(Color32::WHITE, 0.16));
    let period = 6.0 * std::f32::consts::SQRT_2;
    let (top, bottom) = (rect.top(), rect.bottom());
    let mut c = rect.left() + top;
    while c < rect.right() + bottom {
        painter.line_segment([pos2(c - top, top), pos2(c - bottom, bottom)], stroke);
        c += period;
    }
}

/// The full window's message in the track: the distance to the next brake point between
/// zones, or why there's nothing to count.
fn track_message(s: &CueState) -> Option<String> {
    match s.mode {
        CueMode::Idle => Some(format!("NEXT  {}", dist_text(s.dist))),
        CueMode::NoReference => Some("NO REFERENCE LAP".into()),
        CueMode::NoZones => Some("NO BRAKE ZONES".into()),
        CueMode::NoCar => Some("WAITING FOR THE CAR".into()),
        CueMode::Countdown | CueMode::Brake | CueMode::Braking => None,
    }
}

/// The cap: 3, 2, 1 in a red outline deepening with each count, then BRAKE on solid
/// red; your grade's moment lights it too (purple for perfect).
fn paint_cap(painter: &Painter, l: &Layout, s: &CueState, look: Look, pop: f32) {
    let cap = l.cap;
    let flash = s.flash.filter(|f| f.alpha > 0.0);
    painter.rect_filled(cap, CAP_RADIUS, look.scrim);
    let mut outline = Some(Stroke::new(1.5, CAP_LINE));
    let mut text_color = CAP_TEXT;
    match (s.mode, flash) {
        (_, Some(f)) => {
            let tint = if f.grade == Grade::Perfect { theme::grade_color(Grade::Perfect) } else { BRAKE_INK };
            painter.rect_filled(cap, CAP_RADIUS, theme::alpha(tint, f.alpha));
            text_color = theme::alpha(Color32::WHITE, 0.4 + 0.6 * f.alpha);
        }
        (CueMode::Countdown, None) => {
            let tint = match s.beat {
                3 => Color32::from_rgba_unmultiplied(255, 61, 46, 26),
                2 => Color32::from_rgba_unmultiplied(200, 40, 28, 56),
                _ => Color32::from_rgba_unmultiplied(138, 26, 17, 115),
            };
            painter.rect_filled(cap, CAP_RADIUS, tint);
            outline = Some(Stroke::new(1.5, theme::BRAKE));
            text_color = Color32::WHITE;
        }
        (CueMode::Brake, None) => {
            painter.rect_filled(cap, CAP_RADIUS, BRAKE_INK);
            outline = None;
            text_color = Color32::WHITE;
        }
        (CueMode::NoReference | CueMode::NoZones | CueMode::NoCar, None) => text_color = look.muted,
        _ => {}
    }
    if let Some(stroke) = outline {
        painter.rect_stroke(cap, CAP_RADIUS, stroke, StrokeKind::Inside);
    }

    let halo = look.fade;
    if s.mode == CueMode::Countdown && flash.is_none() {
        // The digit, 82 % of the box's height, popping in on each count.
        let p = ease_out(pop);
        let size = (0.82 * cap.height()).clamp(14.0, 44.0) * (1.35 - 0.35 * p);
        let text = galley(painter, &s.beat.to_string(), Weight::Bold, size, 0.0, text_color);
        paint_centred(painter, cap.center(), text, size, theme::alpha(text_color, 0.4 + 0.6 * p), halo);
    } else if s.mode == CueMode::Brake || flash.is_some() {
        // BRAKE is about 3.6 em wide: sized to stay inside the box at any window size.
        let size = l.em.min(0.27 * (cap.width() - 8.0)).min(0.58 * cap.height());
        let text = galley(painter, "BRAKE", Weight::Bold, size, 0.08, text_color);
        paint_centred(painter, cap.center(), text, size, text_color, halo);
    } else if matches!(s.mode, CueMode::NoReference | CueMode::NoZones | CueMode::NoCar) {
        let text = galley(painter, "\u{2013}", Weight::Bold, l.em, 0.0, text_color);
        paint_centred(painter, cap.center(), text, l.em, text_color, halo);
    }
}

// ---- Target and timing (full window) ----

/// The target pill's font size for a content height.
fn target_size(content_h: f32) -> f32 {
    (0.15 * content_h).clamp(12.0, 18.0)
}

/// Width of the target text column: the wider of its label and a "100%" pill, so the
/// bar doesn't shift as the target changes.
fn target_width(painter: &Painter, content_h: f32) -> f32 {
    let label = galley(painter, "TARGET", Weight::SemiBold, 9.0, 0.1, theme::HUD_MUTED);
    let pill = galley(painter, "100%", Weight::Bold, target_size(content_h), 0.0, theme::TARGET);
    label.size().x.max(pill.size().x + 12.0).ceil()
}

/// The gauge (your pedal filling toward the gold target line, your peak so far) and the
/// zone's target in a gold pill.
fn paint_target(painter: &Painter, gauge: Rect, text: Rect, content_h: f32, s: &CueState, look: Look) {
    painter.rect_filled(gauge, 2.0, look.scrim);
    painter.rect_filled(gauge, 2.0, theme::alpha(Color32::WHITE, 0.08));
    let y = |v: f32| gauge.bottom() - v.clamp(0.0, 1.0) * gauge.height();
    if s.live > 0.0 {
        painter.rect_filled(Rect::from_min_max(pos2(gauge.left(), y(s.live)), gauge.max), 2.0, theme::BRAKE);
    }
    if s.mode == CueMode::Braking
        && let Some(peak) = s.peak
    {
        painter.rect_filled(
            Rect::from_center_size(pos2(gauge.center().x, y(peak)), vec2(GAUGE_W, 2.0)),
            0.0,
            PEAK_MARK,
        );
    }
    if let Some(target) = s.target {
        let line = Rect::from_center_size(pos2(gauge.center().x, y(target)), vec2(GAUGE_W + 6.0, 2.0));
        painter.rect_filled(line, 1.0, theme::TARGET);
    }

    // "TARGET" over the pill, the pair centred on the bar.
    let size = target_size(content_h);
    let idle = s.mode == CueMode::Idle;
    let value_color = if idle { theme::alpha(theme::TARGET, 0.75) } else { theme::TARGET };
    let value = s.target.map_or_else(|| "\u{2013}".to_owned(), pct);
    let value = galley(painter, &value, Weight::Bold, size, 0.0, value_color);
    let pill_h = size + 8.0;
    let top = text.center().y - (9.0 + 4.0 + pill_h) / 2.0;
    let label = galley(painter, "TARGET", Weight::SemiBold, 9.0, 0.1, look.muted);
    paint_left(painter, text.left(), top + 4.5, label, 9.0, look.muted, look.fade);
    let pill = Rect::from_min_size(pos2(text.left(), top + 13.0), vec2(value.size().x + 12.0, pill_h));
    let border = theme::alpha(theme::TARGET, if idle { 0.5 } else { 0.85 });
    painter.rect(pill, 4.0, theme::alpha(theme::SURFACE, 0.9), Stroke::new(1.0, border), StrokeKind::Inside);
    paint_centred(painter, pill.center(), value, size, value_color, 0.0);
}

/// The timing row: the zone, its grade, your brake-on against the reference's in
/// seconds and metres, and your peak against the target.
fn paint_verdict(painter: &Painter, row: Rect, width: f32, s: &CueState, look: Look) {
    let painter = painter.with_clip_rect(row.expand2(vec2(2.0, 3.0)).intersect(painter.clip_rect()));
    let cy = row.center().y;
    let halo = look.fade;
    let Some(v) = s.verdict else {
        let text = if s.mode == CueMode::NoReference {
            "Load a Garage 61 lap to get brake points"
        } else {
            "Timing shows after the first zone"
        };
        let text = galley(&painter, text, Weight::Regular, 11.0, 0.0, look.muted);
        paint_left(&painter, row.left(), cy, text, 11.0, look.muted, halo);
        return;
    };
    let zone = if v.zone_no > 0 { format!("Z{}", v.zone_no) } else { "\u{2013}".to_owned() };
    let zone_color = if v.current { theme::HUD_TEXT } else { look.muted };
    let zone = galley(&painter, &zone, Weight::SemiBold, 11.0, 0.04, zone_color);
    let zone_w = zone.size().x.max(16.0);
    paint_left(&painter, row.left(), cy, zone, 11.0, zone_color, halo);
    let mut x = row.left() + zone_w + ROW_GAP;

    let chip = widgets::grade_chip_styled(&painter, pos2(x, cy), v.grade, 10.5, v.pending);
    x = chip.right() + ROW_GAP;
    if let Some(dt) = v.dt {
        let text = galley(&painter, &signed(dt, 2, " s"), Weight::Bold, 12.0, 0.0, theme::HUD_TEXT);
        x = paint_left(&painter, x, cy, text, 12.0, theme::HUD_TEXT, halo) + ROW_GAP;
    }
    if width > NEXT_MIN_W
        && let Some(dm) = v.dm
    {
        let text = galley(&painter, &signed(dm, 0, " m"), Weight::SemiBold, 11.0, 0.0, look.muted);
        x = paint_left(&painter, x, cy, text, 11.0, look.muted, halo) + ROW_GAP;
    }
    // Your peak against the target; the difference once you're off the brake.
    if width > PEAK_MIN_W
        && let Some(peak) = v.peak
    {
        let mut job = LayoutJob::default();
        overlay::spaced(&mut job, "PEAK ", Weight::SemiBold, 11.0, 0.04, look.muted, 0.0);
        overlay::spaced(&mut job, &pct(peak), Weight::Bold, 11.0, 0.04, theme::HUD_TEXT, 0.0);
        if !(v.current && s.mode == CueMode::Braking) {
            overlay::spaced(&mut job, &peak_diff(peak, v.target), Weight::SemiBold, 10.5, 0.04, look.muted, 4.0);
        }
        let text = painter.layout_job(job);
        let left = row.right() - text.size().x;
        if left >= x {
            paint_left(&painter, left, cy, text, 11.0, theme::HUD_TEXT, halo);
        }
    }
}

// ---- Compact readout ----

/// One number centred in each third of the bar, so none crosses the lines between the
/// counts: the target for the zone ahead (gold); you (light blue): your pressure while
/// braking, then your final pressure against the target for a few seconds; and the
/// distance to the next brake point between zones. Without a reference the message
/// spans all three.
fn paint_readout(painter: &Painter, track: Rect, content: Rect, s: &CueState, look: Look) {
    let size = (0.36 * content.height()).clamp(11.0, 14.0);
    let labels = content.width() > LABELS_MIN_W;
    let third = track.width() / 3.0;
    let slot = |i: f32| Rect::from_min_size(pos2(track.left() + i * third, track.top()), vec2(third, track.height()));
    let item = |slot: Rect, label: &str, value: &str, color: Color32, extra: Option<String>| {
        let label = labels.then_some(label).filter(|l| !l.is_empty());
        readout_item(painter, slot, size, label, value, color, extra.as_deref(), look);
    };
    let message = match s.mode {
        CueMode::NoReference => Some("No reference lap"),
        CueMode::NoZones => Some("No brake zones"),
        CueMode::NoCar => Some("Waiting for the car"),
        _ => None,
    };
    if let Some(message) = message {
        item(track, "", message, look.muted, None);
        return;
    }
    if let Some(target) = s.target {
        item(slot(0.0), "TGT", &pct(target), theme::TARGET, None);
    }
    if s.live > 0.02 {
        item(slot(1.0), "NOW", &pct(s.live), theme::YOU, None);
    } else if let Some(f) = s.final_peak {
        item(slot(1.0), "FINAL", &pct(f.peak), theme::YOU, Some(peak_diff(f.peak, f.target)));
    }
    if s.mode == CueMode::Idle {
        item(slot(2.0), "NEXT", &dist_text(s.dist), look.muted, None);
    }
}

/// A small label, a value and an optional note on a shared baseline, on a faint dark
/// backing so it reads over any colour the bar takes. Drops the label, then clips,
/// when the slot is too narrow.
#[allow(clippy::too_many_arguments)]
fn readout_item(
    painter: &Painter,
    slot: Rect,
    size: f32,
    label: Option<&str>,
    value: &str,
    color: Color32,
    extra: Option<&str>,
    look: Look,
) {
    const GAP: f32 = 4.0;
    const PAD: Vec2 = vec2(5.0, 2.0);
    let soft = |a: f32| theme::alpha(theme::HUD_TEXT, a);
    let value_weight = if color == look.muted { Weight::SemiBold } else { Weight::Bold };
    let value = galley(painter, value, value_weight, size, if color == look.muted { 0.04 } else { 0.0 }, color);
    let extra = extra.map(|e| (galley(painter, e, Weight::SemiBold, 0.8 * size, 0.0, soft(0.8)), 0.8 * size));
    let mut label = label.map(|l| (galley(painter, l, Weight::SemiBold, 0.68 * size, 0.08, soft(0.75)), 0.68 * size));
    let width = |label: &Option<(Arc<Galley>, f32)>| {
        let mut w = value.size().x;
        for (g, _) in label.iter().chain(extra.iter()) {
            w += GAP + g.size().x;
        }
        w
    };
    let max_w = slot.width() - 6.0 - 2.0 * PAD.x;
    if width(&label) > max_w {
        label = None;
    }
    let w = width(&label).min(max_w);
    let cy = slot.center().y;
    let backing =
        Rect::from_center_size(pos2(slot.center().x, cy), vec2(w + 2.0 * PAD.x, CAP_HEIGHT * size + 2.0 * PAD.y + 4.0));
    painter.rect_filled(backing, 3.0, theme::alpha(theme::SURFACE, 0.5));

    // Everything sits on the value's baseline, its capitals centred in the bar.
    let painter = painter.with_clip_rect(backing.intersect(painter.clip_rect()));
    let base = cy + CAP_HEIGHT * size / 2.0;
    let mut x = backing.left() + PAD.x;
    let mut put = |g: Arc<Galley>, c: Color32| {
        let right = x + g.size().x;
        overlay::halo_galley(&painter, pos2(x, base - baseline(&g)), g, c, 1.0);
        x = right + GAP;
    };
    if let Some((g, _)) = label {
        put(g, soft(0.75));
    }
    put(value, color);
    if let Some((g, _)) = extra {
        put(g, soft(0.8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::{BrakeCue, CueConfig};
    use crate::trace::LiveTrace;
    use eframe::egui::{self, Event};

    fn window(w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(w, h) + Vec2::splat(2.0 * overlay::MARGIN))
    }

    /// The window in a headless egui, fed pointer events.
    struct Harness {
        ctx: egui::Context,
        window: Rect,
        compact: bool,
        locked: bool,
    }

    impl Harness {
        fn new(panel: Vec2, compact: bool) -> Self {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx);
            let window = window(panel.x, panel.y);
            let h = Self { ctx, window, compact, locked: false };
            h.frame(Vec::new());
            h.frame(Vec::new());
            h
        }

        fn frame(&self, events: Vec<Event>) -> Option<CueIntent> {
            let state = BrakeCue::new().update(None, &LiveTrace::new(), None, 0.0, &CueConfig::default());
            let chrome = Chrome {
                opacity: 0.8,
                locked: self.locked,
                settings_open: false,
                reference_time: None,
                badges: &[],
                file_hover: false,
                resizing: false,
                pulse: 0.0,
            };
            let f = CueFrame { state: &state, chrome, compact: self.compact, contents: 1.0, pop: 1.0 };
            let input = egui::RawInput { screen_rect: Some(self.window), events, ..Default::default() };
            let mut intent = None;
            let mut out = self.ctx.run_ui(input, |ui| intent = show(ui, self.window, &f));
            out.textures_delta.clear();
            intent
        }

        /// Moves there (a frame to hover), then presses and releases: what each frame asked for.
        fn click(&self, at: Pos2) -> Vec<CueIntent> {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)], Vec::new()]
                .into_iter()
                .filter_map(|events| self.frame(events))
                .collect()
        }

        /// Moves there, presses and drags 20 pt right.
        fn drag(&self, at: Pos2) -> Vec<CueIntent> {
            let button = |pos, pressed| Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            let to = at + vec2(20.0, 0.0);
            [
                vec![Event::PointerMoved(at)],
                vec![button(at, true)],
                vec![Event::PointerMoved(to)],
                vec![button(to, false)],
            ]
            .into_iter()
            .filter_map(|events| self.frame(events))
            .collect()
        }
    }

    #[test]
    fn the_header_moves_the_window_and_its_buttons_do_what_they_say() {
        let h = Harness::new(DEFAULT_PANEL, false);
        let header = overlay::header_rect(h.window);
        let close = overlay::close_rect(header);
        let gear = close.translate(vec2(-(GEAR_SIZE + BUTTON_GAP), 0.0));
        let collapse = gear.translate(vec2(-(GEAR_SIZE + BUTTON_GAP), 0.0));
        assert_eq!(h.drag(header.left_center() + vec2(40.0, 0.0)), [CueIntent::Move]);
        assert_eq!(h.click(collapse.center()), [CueIntent::Compact(true)]);
        assert_eq!(h.click(gear.center()), [CueIntent::ToggleSettings]);
        assert_eq!(h.click(close.center()), [CueIntent::Close]);
        // The bar itself doesn't move the full window.
        assert!(h.drag(h.window.center() + vec2(0.0, 5.0)).is_empty());
        // An edge resizes it.
        assert_eq!(h.drag(h.window.right_center() - vec2(3.0, 0.0)), [CueIntent::Resize(ResizeDirection::East)]);
    }

    #[test]
    fn the_compact_bar_moves_the_window_and_hovering_shows_expand() {
        let h = Harness::new(vec2(DEFAULT_PANEL.x, COMPACT_HEIGHT), true);
        assert_eq!(h.drag(h.window.center() - vec2(60.0, 0.0)), [CueIntent::Move]);
        let content = overlay::content_rect(h.window);
        let expand = pos2(content.right() - 18.0, content.center().y);
        assert_eq!(h.click(expand), [CueIntent::Compact(false)], "a click, not a move");
        assert_eq!(h.drag(h.window.right_center() - vec2(3.0, 0.0)), [CueIntent::Resize(ResizeDirection::East)]);
    }

    #[test]
    fn a_locked_window_ignores_drags() {
        let mut h = Harness::new(DEFAULT_PANEL, false);
        h.locked = true;
        let header = overlay::header_rect(h.window);
        assert!(h.drag(header.left_center() + vec2(40.0, 0.0)).is_empty());
        assert!(h.drag(h.window.right_center() - vec2(3.0, 0.0)).is_empty());
        let mut compact = Harness::new(vec2(DEFAULT_PANEL.x, COMPACT_HEIGHT), true);
        compact.locked = true;
        assert!(compact.drag(compact.window.center()).is_empty());
    }

    #[test]
    fn the_full_bar_keeps_its_minimum_at_the_smallest_window() {
        let l = Layout::new(window(MIN_PANEL.x, MIN_PANEL.y), false, 40.0);
        assert!(l.track.height() >= BAR_MIN - 0.01, "{:?}", l.track);
        let verdict = l.verdict.unwrap();
        assert!(l.track.bottom() + ROW_GAP <= verdict.top() + 0.01, "bar and timing row overlap");
        assert!(verdict.bottom() <= l.panel.bottom());
        assert!(l.track.width() > 60.0, "{:?}", l.track);
        // Left to right: track, cap, gauge, target text, inside the panel.
        let (gauge, target) = (l.gauge.unwrap(), l.target.unwrap());
        assert!(l.track.right() < l.cap.left() && l.cap.right() < gauge.left() && gauge.right() < target.left());
        assert!(target.right() <= l.panel.right() - BODY_SIDE);
    }

    #[test]
    fn the_cap_grows_with_the_window_within_limits() {
        let small = Layout::new(window(230.0, 96.0), false, 40.0);
        let big = Layout::new(window(900.0, 200.0), false, 40.0);
        assert_eq!(small.cap.width(), 58.0);
        assert_eq!(big.cap.width(), 110.0);
        let compact = Layout::new(window(MIN_COMPACT.x, MIN_COMPACT.y), true, 0.0);
        assert_eq!(compact.cap.width(), 40.0);
        assert!(compact.verdict.is_none() && compact.gauge.is_none());
        assert_eq!(compact.track.height(), MIN_COMPACT.y - 2.0 - 2.0 * COMPACT_INSET);
    }

    #[test]
    fn brake_fits_its_box_at_every_size() {
        for (w, h, compact) in [
            (230.0, 96.0, false),
            (360.0, 96.0, false),
            (900.0, 300.0, false),
            (180.0, 30.0, true),
            (600.0, 60.0, true),
        ] {
            let l = Layout::new(window(w, h), compact, 40.0);
            let size = l.em.min(0.27 * (l.cap.width() - 8.0)).min(0.58 * l.cap.height());
            // About 3.6 em wide with its tracking, capitals 0.7 em tall.
            assert!(3.6 * size <= l.cap.width() - 8.0 + 0.01, "{w}×{h}: {size}");
            assert!(CAP_HEIGHT * size <= l.cap.height(), "{w}×{h}: {size}");
        }
    }

    #[test]
    fn numbers_read_like_the_prototype() {
        assert_eq!(signed(0.041, 2, " s"), "+0.04 s");
        assert_eq!(signed(-0.004, 2, " s"), "0.00 s");
        assert_eq!(signed(-3.4, 0, " m"), "\u{2212}3 m");
        assert_eq!(dist_text(239.6), "240 m");
        assert_eq!(dist_text(-5.0), "0 m");
        assert_eq!(dist_text(1234.0), "1.2 km");
        assert_eq!(peak_diff(0.82, 0.85), "\u{2212}3");
        assert_eq!(peak_diff(0.851, 0.849), "±0");
        assert_eq!(pct(0.826), "83%");
    }

    #[test]
    fn the_pulse_rises_fast_and_fades() {
        assert_eq!(pulse_alpha(-0.1), 0.0);
        assert_eq!(pulse_alpha(0.0), 0.0);
        assert!((pulse_alpha(0.12 * PULSE_S) - 1.0).abs() < 1e-4);
        assert!(pulse_alpha(0.3 * PULSE_S) > pulse_alpha(0.6 * PULSE_S));
        assert_eq!(pulse_alpha(PULSE_S), 0.0);
    }

    #[test]
    fn the_count_darkens_toward_the_brake_point() {
        assert_eq!(count_color(0.0), theme::COUNT_LIGHT);
        assert_eq!(count_color(0.45), theme::BRAKE);
        assert_eq!(count_color(1.0), theme::COUNT_DEEP);
    }
}
