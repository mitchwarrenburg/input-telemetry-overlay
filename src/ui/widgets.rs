//! Custom settings widgets styled after the prototype: slider, switch, segmented
//! control, tabs, buttons, drop zone.
//!
//! Sizes and colours follow `prototype/src/styles.css`. Interactive widgets implement
//! [`Widget`], so `ui.add` / `ui.add_enabled` work and each returns a [`Response`]
//! (`changed()` when a value was edited). Line boxes use the prototype's
//! `line-height: 1.4`, rounded to whole pixels so hairlines stay crisp.

use std::ops::RangeInclusive;
use std::sync::Arc;

use eframe::egui::epaint::tessellator::path::rounded_rectangle;
use eframe::egui::epaint::{CornerRadiusF32, Shadow};
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{
    Align2, Color32, Context, CornerRadius, CursorIcon, Event, EventFilter, FontId, Galley, Id, InputState, Key,
    Modifiers, Painter, PointerButton, Pos2, Rangef, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextFormat, Ui,
    Vec2, Widget, WidgetInfo, WidgetType, lerp, pos2, vec2,
};

use super::theme::{self, Weight};

/// Line box of 13 px body text.
pub const LINE: f32 = 18.0;
/// Line box of 12 px secondary text.
pub const LINE_SMALL: f32 = 17.0;

/// Slider thumb and switch knob.
const KNOB: Color32 = Color32::from_rgb(0xf2, 0xf7, 0xf5);
/// Text of the selected segment.
const SEGMENT_TEXT_ON: Color32 = Color32::from_rgb(0xc9, 0xf7, 0xe4);

/// Setting names. The prototype sets them at weight 500, between our Regular and
/// SemiBold faces; SemiBold keeps them clearly above their hints.
fn label_font() -> FontId {
    theme::font(Weight::SemiBold, 13.0)
}

/// White at opacity `a`: the prototype's `rgba(255, 255, 255, a)` tints.
pub fn white(a: f32) -> Color32 {
    Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Pointer is over an enabled widget.
fn hot(response: &Response) -> bool {
    response.enabled() && (response.hovered() || response.is_pointer_button_down_on())
}

/// Keyboard focus outline: 2 px accent, `offset` outside `rect` (negative: inside).
fn focus_ring(painter: &Painter, rect: Rect, radius: f32, offset: f32) {
    let outer = rect.expand(offset + 2.0);
    painter.rect_stroke(outer, radius + offset + 2.0, Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
}

// ---- Text ----

/// Text wrapped with a CSS-style line height: rows are `line_height` apart and each
/// row's glyphs sit in the middle of their line box, as in a browser.
pub struct TextBlock {
    galley: Arc<Galley>,
    lead: f32,
}

impl TextBlock {
    /// Lays out `job` with every section's rows `line_height` apart.
    pub fn new(ui: &Ui, mut job: LayoutJob, line_height: f32) -> Self {
        for section in &mut job.sections {
            section.format.line_height = Some(line_height);
        }
        let font_height =
            job.sections.first().map_or(line_height, |s| ui.ctx().fonts_mut(|f| f.row_height(&s.format.font_id)));
        Self { galley: ui.painter().layout_job(job), lead: ((line_height - font_height) / 2.0).max(0.0) }
    }

    /// Plain text wrapped at `width`.
    pub fn wrapped(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32, line_height: f32) -> Self {
        let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
        job.wrap = TextWrapping::wrap_at_width(width);
        Self::new(ui, job, line_height)
    }

    pub fn size(&self) -> Vec2 {
        self.galley.size()
    }

    pub fn paint(&self, painter: &Painter, top_left: Pos2) {
        painter.galley(top_left + vec2(0.0, self.lead), Arc::clone(&self.galley), Color32::PLACEHOLDER);
    }
}

/// One line of text, cut with "…" past `max_width`.
pub fn single_line(ui: &Ui, text: impl Into<String>, font: FontId, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.into(), TextFormat::simple(font, color));
    job.wrap = TextWrapping::truncate_at_width(max_width.max(0.0));
    ui.painter().layout_job(job)
}

/// Uppercase, letter-spaced text (`text-transform: uppercase; letter-spacing: <em>em`).
pub fn caps(ui: &Ui, text: &str, font: FontId, color: Color32, spacing_em: f32) -> Arc<Galley> {
    let format = TextFormat { extra_letter_spacing: font.size * spacing_em, ..TextFormat::simple(font, color) };
    ui.painter().layout_job(LayoutJob::single_section(text.to_uppercase(), format))
}

/// Paints `galley` anchored in `rect` by `align` (e.g. left edge, vertically centred).
/// Uncoloured text takes `color`.
pub fn paint_galley(painter: &Painter, rect: Rect, align: Align2, galley: Arc<Galley>, color: Color32) {
    let pos = align.align_size_within_rect(galley.size(), rect).min;
    painter.galley(pos, galley, color);
}

// ---- Static elements ----

/// Small uppercase heading over a group of settings (`.set-section`).
pub fn section_label(ui: &mut Ui, text: &str) {
    let galley = caps(ui, text, theme::font(Weight::SemiBold, 10.5), theme::UI_MUTED, 0.1);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 11.0), Sense::hover());
    paint_galley(ui.painter(), rect, Align2::LEFT_CENTER, galley, theme::UI_MUTED);
}

/// Hairline across the row, reaching `bleed` points past both sides (`.set-divider`).
pub fn divider(ui: &mut Ui, bleed: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range().expand(bleed), rect.center().y, Stroke::new(1.0, theme::UI_LINE));
}

/// Muted explanatory text (`.set-hint`).
pub fn hint(ui: &mut Ui, text: &str) {
    let width = ui.available_width();
    let block = TextBlock::wrapped(ui, text, theme::font(Weight::Regular, 12.0), theme::UI_MUTED, width, LINE_SMALL);
    let (rect, _) = ui.allocate_exact_size(vec2(width, block.size().y), Sense::hover());
    block.paint(ui.painter(), rect.min);
}

/// Wrapped 12 px problem text under a control.
pub fn error_text(ui: &mut Ui, text: &str) {
    let width = ui.available_width();
    let block = TextBlock::wrapped(ui, text, theme::font(Weight::Regular, 12.0), theme::DANGER, width, LINE_SMALL);
    let (rect, _) = ui.allocate_exact_size(vec2(width, block.size().y), Sense::hover());
    block.paint(ui.painter(), rect.min);
}

/// A setting's name on its own line above its control (`.row-head`).
pub fn row_label(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), LINE), Sense::hover());
    paint_row_head(ui, rect, text, None, None);
}

/// Label (plus optional muted detail) on the left, value on the right.
fn paint_row_head(ui: &Ui, rect: Rect, label: &str, detail: Option<&str>, value: Option<&str>) {
    let painter = ui.painter();
    let value =
        value.map(|v| painter.layout_no_wrap(v.to_owned(), theme::font(Weight::SemiBold, 13.0), theme::UI_TEXT));
    let value_width = value.as_ref().map_or(0.0, |g| g.size().x + 8.0);

    let mut job = LayoutJob::default();
    job.append(label, 0.0, TextFormat::simple(label_font(), theme::UI_TEXT));
    if let Some(detail) = detail {
        job.append(detail, 3.5, TextFormat::simple(theme::font(Weight::Regular, 12.0), theme::UI_MUTED));
    }
    job.wrap = TextWrapping::truncate_at_width(rect.width() - value_width);
    paint_galley(painter, rect, Align2::LEFT_CENTER, painter.layout_job(job), theme::UI_TEXT);
    if let Some(value) = value {
        paint_galley(painter, rect, Align2::RIGHT_CENTER, value, theme::UI_TEXT);
    }
}

// ---- Icons ----

/// Line icons from the prototype's 24×24 SVGs (Barlow has no glyphs for them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Close,
    Check,
    Warning,
    /// Hollow circle: nothing known yet.
    Ring,
    /// Tray with an up arrow: add a file.
    Upload,
}

/// Paints `icon` scaled into the square `rect`. `stroke` is in the icon's 24-unit grid,
/// like SVG `stroke-width`.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32, stroke: f32) {
    let s = rect.width() / 24.0;
    let p = |x: f32, y: f32| rect.min + vec2(x * s, y * s);
    let stroke = Stroke::new(stroke * s, color);
    match icon {
        Icon::Close => {
            painter.line_segment([p(6.0, 6.0), p(18.0, 18.0)], stroke);
            painter.line_segment([p(18.0, 6.0), p(6.0, 18.0)], stroke);
        }
        Icon::Check => {
            painter.line(vec![p(5.0, 12.5), p(9.5, 17.0), p(19.0, 7.5)], stroke);
        }
        Icon::Warning => {
            painter.add(Shape::closed_line(vec![p(12.0, 4.0), p(2.8, 19.5), p(21.2, 19.5)], stroke));
            painter.line_segment([p(12.0, 10.0), p(12.0, 14.2)], stroke);
            painter.circle_filled(p(12.0, 17.1), stroke.width * 0.6, color);
        }
        Icon::Ring => {
            painter.circle_stroke(p(12.0, 12.0), 7.0 * s, stroke);
        }
        Icon::Upload => {
            painter.line_segment([p(12.0, 15.0), p(12.0, 4.0)], stroke);
            painter.line(vec![p(7.5, 8.5), p(12.0, 4.0), p(16.5, 8.5)], stroke);
            painter.line(
                vec![
                    p(4.0, 15.0),
                    p(4.0, 18.0),
                    p(4.6, 19.4),
                    p(6.0, 20.0),
                    p(18.0, 20.0),
                    p(19.4, 19.4),
                    p(20.0, 18.0),
                    p(20.0, 15.0),
                ],
                stroke,
            );
        }
    }
}

// ---- Slider ----

/// `value` moved to the nearest multiple of `step` from the start of `range`, inside it.
pub fn snap(value: f32, range: &RangeInclusive<f32>, step: f32) -> f32 {
    let (lo, hi) = (*range.start(), *range.end());
    let snapped = if step > 0.0 { lo + ((value - lo) / step).round() * step } else { value };
    snapped.clamp(lo, hi)
}

/// A slider's readout: whole numbers without decimals, others with one; "Off" at zero
/// when the setting reads that way.
pub fn format_value(value: f32, unit: &str, off_at_zero: bool) -> String {
    if off_at_zero && value == 0.0 {
        "Off".to_owned()
    } else if value.fract() == 0.0 {
        format!("{value:.0}{unit}")
    } else {
        format!("{value:.1}{unit}")
    }
}

/// Labelled range slider (`.set-row` with `input[type=range]`): a head with the label
/// and value, then a full-width 4 px track with a 14 px thumb. Arrow keys, Home and End
/// work while it has keyboard focus.
pub struct Slider<'a> {
    label: &'a str,
    detail: Option<&'a str>,
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    step: f32,
    unit: &'a str,
    off_at_zero: bool,
}

impl<'a> Slider<'a> {
    pub fn new(label: &'a str, value: &'a mut f32, range: RangeInclusive<f32>) -> Self {
        Self { label, detail: None, value, range, step: 1.0, unit: "", off_at_zero: false }
    }

    /// Values snap to multiples of `step` from the start of the range (default 1).
    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// Appended to the readout, e.g. `"%"` or `" m"`.
    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    /// Muted words after the label.
    pub fn detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Read "Off" at zero.
    pub fn off_at_zero(mut self) -> Self {
        self.off_at_zero = true;
        self
    }

    /// The value the pointer or keyboard asks for this frame, if any.
    fn requested(&self, ui: &Ui, response: &Response, x_range: Rangef) -> Option<f32> {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        if let Some(pos) = response.interact_pointer_pos() {
            let t = ((pos.x - x_range.min) / x_range.span().max(1.0)).clamp(0.0, 1.0);
            return Some(lerp(lo..=hi, t));
        }
        if !response.has_focus() {
            return None;
        }
        ui.memory_mut(|m| {
            let filter = EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() };
            m.set_focus_lock_filter(response.id, filter);
        });
        ui.input(|i| {
            let up = i.num_presses(Key::ArrowRight) + i.num_presses(Key::ArrowUp);
            let down = i.num_presses(Key::ArrowLeft) + i.num_presses(Key::ArrowDown);
            if i.key_pressed(Key::Home) {
                Some(lo)
            } else if i.key_pressed(Key::End) {
                Some(hi)
            } else if up != down {
                Some(*self.value + (up as f32 - down as f32) * self.step)
            } else {
                None
            }
        })
    }
}

impl Widget for Slider<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        const TRACK_HEIGHT: f32 = 16.0;
        const THUMB_RADIUS: f32 = 7.0;
        let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), LINE + 6.0 + TRACK_HEIGHT), Sense::hover());
        let track = Rect::from_min_max(pos2(rect.left(), rect.bottom() - TRACK_HEIGHT), rect.max);
        let mut response = ui.interact(track, row.id.with("track"), Sense::click_and_drag());
        let x_range = track.x_range().shrink(THUMB_RADIUS);

        if let Some(v) = self.requested(ui, &response, x_range).map(|v| snap(v, &self.range, self.step))
            && v != *self.value
        {
            *self.value = v;
            response.mark_changed();
        }
        let value = *self.value;
        let text = format_value(value, self.unit, self.off_at_zero);
        response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), f64::from(value), self.label));

        paint_row_head(
            ui,
            Rect::from_min_size(rect.min, vec2(rect.width(), LINE)),
            self.label,
            self.detail,
            Some(&text),
        );
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let t = if hi > lo { ((value - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
        let thumb = pos2(lerp(x_range, t), track.center().y);
        paint_slider(ui.painter(), track, thumb, &response);
        response.on_hover_cursor(CursorIcon::PointingHand)
    }
}

fn paint_slider(painter: &Painter, track: Rect, thumb: Pos2, response: &Response) {
    let bar = Rect::from_x_y_ranges(track.x_range(), thumb.y - 2.0..=thumb.y + 2.0);
    painter.rect_filled(bar, 2.0, white(0.12));
    painter.rect_filled(Rect::from_x_y_ranges(bar.left()..=thumb.x, bar.y_range()), 2.0, theme::ACCENT);

    let halo = if response.has_focus() {
        Some(0.4)
    } else if hot(response) || response.dragged() {
        Some(0.18)
    } else {
        None
    };
    if let Some(a) = halo {
        painter.circle_filled(thumb, 11.0, theme::alpha(theme::ACCENT, a));
    }
    let knob = Rect::from_center_size(thumb, Vec2::splat(14.0));
    let shadow = Shadow { offset: [0, 1], blur: 3, spread: 0, color: Color32::from_black_alpha(128) };
    painter.add(shadow.as_shape(knob, 7));
    painter.circle_filled(thumb, 7.0, KNOB);
}

// ---- Switch ----

/// Switch row (`.set-toggle`): label and optional hint on the left, a 32×18 switch on
/// the right. Clicking anywhere on the row toggles it.
pub struct Switch<'a> {
    on: &'a mut bool,
    label: &'a str,
    hint: Option<&'a str>,
}

impl<'a> Switch<'a> {
    pub fn new(on: &'a mut bool, label: &'a str) -> Self {
        Self { on, label, hint: None }
    }

    /// Smaller muted line under the label.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }
}

impl Widget for Switch<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        const SIZE: Vec2 = vec2(32.0, 18.0);
        let width = ui.available_width();
        let text_width = width - SIZE.x - 12.0;
        let label = single_line(ui, self.label, label_font(), theme::UI_TEXT, text_width);
        let hint = self.hint.map(|h| {
            TextBlock::wrapped(ui, h, theme::font(Weight::Regular, 12.0), theme::UI_MUTED, text_width, LINE_SMALL)
        });
        let height = LINE + hint.as_ref().map_or(0.0, |h| 1.0 + h.size().y);

        let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        let on = *self.on;
        response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, self.label));

        let painter = ui.painter();
        let label_rect = Rect::from_min_size(rect.min, vec2(text_width, LINE));
        paint_galley(painter, label_rect, Align2::LEFT_CENTER, label, theme::UI_TEXT);
        if let Some(hint) = hint {
            hint.paint(painter, rect.min + vec2(0.0, LINE + 1.0));
        }
        let track = Align2::RIGHT_CENTER.align_size_within_rect(SIZE, rect);
        paint_switch(ui, response.id, track, on, response.has_focus());
        response.on_hover_cursor(CursorIcon::PointingHand)
    }
}

fn paint_switch(ui: &Ui, id: Id, track: Rect, on: bool, focused: bool) {
    let t = ui.ctx().animate_bool_with_time(id, on, 0.15);
    let painter = ui.painter();
    painter.rect_filled(track, 9.0, white(0.14).lerp_to_gamma(theme::ACCENT, t));
    let x = lerp(track.left() + 9.0..=track.right() - 9.0, t);
    painter.circle_filled(pos2(x, track.center().y), 7.0, KNOB);
    if focused {
        focus_ring(painter, track, 9.0, 2.0);
    }
}

// ---- Segmented control ----

/// Segmented control (`.seg`): equal-width options in a bordered strip; the selected one
/// is accent-tinted.
pub struct Segmented<'a, T> {
    value: &'a mut T,
    options: &'a [(T, &'a str)],
}

impl<'a, T> Segmented<'a, T> {
    pub fn new(value: &'a mut T, options: &'a [(T, &'a str)]) -> Self {
        Self { value, options }
    }
}

impl<T: PartialEq + Copy> Widget for Segmented<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, mut response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
        ui.painter().rect(rect, 8.0, white(0.03), Stroke::new(1.0, theme::UI_LINE), StrokeKind::Inside);
        let inner = rect.shrink(3.0); // 1 px border + 2 px padding
        let width = inner.width() / self.options.len().max(1) as f32;
        for (i, &(option, label)) in self.options.iter().enumerate() {
            let segment = Rect::from_min_size(inner.min + vec2(width * i as f32, 0.0), vec2(width, inner.height()));
            let r = ui.interact(segment, response.id.with(i), Sense::click());
            if r.clicked() && *self.value != option {
                *self.value = option;
                response.mark_changed();
            }
            let selected = *self.value == option;
            r.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, ui.is_enabled(), selected, label));
            paint_segment(ui, &r, label, selected);
            r.on_hover_cursor(CursorIcon::PointingHand);
        }
        response
    }
}

fn paint_segment(ui: &Ui, response: &Response, label: &str, selected: bool) {
    let painter = ui.painter();
    let rect = response.rect;
    if selected {
        painter.rect_filled(rect, 6.0, theme::alpha(theme::ACCENT, 0.14));
    }
    let color = match (selected, hot(response)) {
        (true, _) => SEGMENT_TEXT_ON,
        (false, true) => theme::UI_TEXT,
        (false, false) => theme::UI_MUTED,
    };
    let galley = single_line(ui, label, theme::font(Weight::SemiBold, 12.5), color, rect.width() - 8.0);
    paint_galley(painter, rect, Align2::CENTER_CENTER, galley, color);
    if response.has_focus() {
        focus_ring(painter, rect, 6.0, -2.0);
    }
}

// ---- Tabs ----

/// Tab strip (`.set-tabs`): text tabs over a hairline spanning the full width, the
/// selected one underlined in the accent colour.
pub struct TabBar<'a, T> {
    value: &'a mut T,
    tabs: &'a [(T, &'a str)],
}

impl<'a, T> TabBar<'a, T> {
    pub fn new(value: &'a mut T, tabs: &'a [(T, &'a str)]) -> Self {
        Self { value, tabs }
    }
}

impl<T: PartialEq + Copy> Widget for TabBar<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        const HEIGHT: f32 = 36.0; // 8 + line + 10
        const PAD: f32 = 8.0;
        let (rect, mut response) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT + 1.0), Sense::hover());
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme::UI_LINE));
        let font = theme::font(Weight::SemiBold, 13.0);
        let mut x = rect.left() + PAD;
        for (i, &(tab, label)) in self.tabs.iter().enumerate() {
            let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER);
            let tab_rect = Rect::from_min_size(pos2(x, rect.top()), vec2(galley.size().x + 2.0 * PAD, HEIGHT));
            x = tab_rect.right() + 2.0;
            let r = ui.interact(tab_rect, response.id.with(i), Sense::click());
            if r.clicked() && *self.value != tab {
                *self.value = tab;
                response.mark_changed();
            }
            let selected = *self.value == tab;
            r.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, ui.is_enabled(), selected, label));
            paint_tab(ui.painter(), &r, galley, selected);
            r.on_hover_cursor(CursorIcon::PointingHand);
        }
        response
    }
}

fn paint_tab(painter: &Painter, response: &Response, galley: Arc<Galley>, selected: bool) {
    let rect = response.rect;
    let color = if selected || hot(response) { theme::UI_TEXT } else { theme::UI_MUTED };
    let text_box = Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(rect.width(), LINE));
    paint_galley(painter, text_box, Align2::CENTER_CENTER, galley, color);
    if selected {
        let underline = Rect::from_min_max(
            pos2(rect.left() + 8.0, rect.bottom() - 1.0),
            pos2(rect.right() - 8.0, rect.bottom() + 1.0),
        );
        painter.rect_filled(underline, CornerRadius { nw: 2, ne: 2, sw: 0, se: 0 }, theme::ACCENT);
    }
    if response.has_focus() {
        focus_ring(painter, rect, 6.0, -2.0);
    }
}

// ---- Buttons ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonKind {
    Standard,
    Quiet,
    Danger,
}

/// Text button (`.btn`): 30 px tall, 1 px strong border. See [`Button::quiet`] and
/// [`Button::danger`] for the other looks.
pub struct Button<'a> {
    text: &'a str,
    kind: ButtonKind,
    fill_width: bool,
    small: bool,
}

impl<'a> Button<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text, kind: ButtonKind::Standard, fill_width: false, small: false }
    }

    /// No border or fill, muted text (`.btn-quiet`).
    pub fn quiet(mut self) -> Self {
        self.kind = ButtonKind::Quiet;
        self
    }

    /// Red-tinted, for confirming something destructive.
    pub fn danger(mut self) -> Self {
        self.kind = ButtonKind::Danger;
        self
    }

    /// Take the full available width.
    pub fn fill_width(mut self) -> Self {
        self.fill_width = true;
        self
    }

    /// 22 px tall with smaller text, for inside list rows.
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (height, font_size, pad, radius) =
            if self.small { (22.0, 11.5, 8.0, 6.0) } else { (30.0, 12.5, 12.0, 7.0) };
        let font = theme::font(Weight::SemiBold, font_size);
        let galley = ui.painter().layout_no_wrap(self.text.to_owned(), font, Color32::PLACEHOLDER);
        let width = if self.fill_width { ui.available_width() } else { galley.size().x + 2.0 * pad };
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), self.text));

        let painter = ui.painter();
        let hot = hot(&response);
        let (fill, border, text) = match self.kind {
            ButtonKind::Standard => {
                (if hot { white(0.08) } else { theme::UI_RAISED }, theme::UI_LINE_STRONG, theme::UI_TEXT)
            }
            ButtonKind::Quiet => {
                (Color32::TRANSPARENT, Color32::TRANSPARENT, if hot { theme::UI_TEXT } else { theme::UI_MUTED })
            }
            ButtonKind::Danger => (
                theme::alpha(theme::DANGER, if hot { 0.24 } else { 0.14 }),
                theme::alpha(theme::DANGER, 0.5),
                theme::UI_TEXT,
            ),
        };
        painter.rect(rect, radius, fill, Stroke::new(1.0, border), StrokeKind::Inside);
        paint_galley(painter, rect, Align2::CENTER_CENTER, galley, text);
        if response.has_focus() {
            focus_ring(painter, rect, radius, 1.0);
        }
        response.on_hover_cursor(CursorIcon::PointingHand)
    }
}

/// Underlined text button (`.link-btn`).
pub struct Link<'a> {
    text: &'a str,
}

impl<'a> Link<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }
}

impl Widget for Link<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let font = theme::font(Weight::Regular, 12.0);
        let underline = Stroke::new(1.0, Color32::PLACEHOLDER);
        let format = TextFormat { underline, ..TextFormat::simple(font, Color32::PLACEHOLDER) };
        let galley = ui.painter().layout_job(LayoutJob::single_section(self.text.to_owned(), format));
        let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x, LINE_SMALL), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Link, ui.is_enabled(), self.text));
        let color = if hot(&response) { theme::UI_TEXT } else { theme::UI_MUTED };
        paint_galley(ui.painter(), rect, Align2::LEFT_CENTER, galley, color);
        if response.has_focus() {
            focus_ring(ui.painter(), rect, 3.0, 1.0);
        }
        response.on_hover_cursor(CursorIcon::PointingHand)
    }
}

/// Square icon-only button (`.icon-btn`): muted icon, faint fill on hover.
pub struct IconButton<'a> {
    icon: Icon,
    /// Accessible name.
    label: &'a str,
    size: f32,
    icon_size: f32,
}

impl<'a> IconButton<'a> {
    /// A 26 px button with a 14 px icon.
    pub fn new(icon: Icon, label: &'a str) -> Self {
        Self { icon, label, size: 26.0, icon_size: 14.0 }
    }

    pub fn size(mut self, size: f32, icon_size: f32) -> Self {
        self.size = size;
        self.icon_size = icon_size;
        self
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), self.label));
        let painter = ui.painter();
        let hot = hot(&response);
        if hot {
            painter.rect_filled(rect, 6.0, white(0.07));
        }
        let color = if hot { theme::UI_TEXT } else { theme::UI_MUTED };
        paint_icon(painter, Rect::from_center_size(rect.center(), Vec2::splat(self.icon_size)), self.icon, color, 1.8);
        if response.has_focus() {
            focus_ring(painter, rect, 6.0, 1.0);
        }
        response.on_hover_cursor(CursorIcon::PointingHand)
    }
}

// ---- Drop zone ----

/// Drop target for files (`.dropzone`), dashed border; clicking it should open a file
/// picker. The compact variant is a single line, for replacing something already loaded.
pub struct DropZone<'a> {
    title: &'a str,
    hint: Option<&'a str>,
    compact: bool,
    highlight: bool,
}

impl<'a> DropZone<'a> {
    pub fn new(title: &'a str) -> Self {
        Self { title, hint: None, compact: false, highlight: false }
    }

    /// Small print under "or browse files" (full variant only).
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// Show as an active target (files are being dragged over the window).
    pub fn highlight(mut self, highlight: bool) -> Self {
        self.highlight = highlight;
        self
    }

    fn content(&self, ui: &Ui, width: f32) -> DropZoneContent {
        let muted = |text: &str, size: f32| {
            ui.painter().layout_no_wrap(text.to_owned(), theme::font(Weight::Regular, size), theme::UI_MUTED)
        };
        if self.compact {
            let title = single_line(ui, self.title, theme::font(Weight::Regular, 12.5), theme::UI_TEXT, width - 22.0);
            return DropZoneContent { title, browse: None, hint: None };
        }
        let title = single_line(ui, self.title, theme::font(Weight::SemiBold, 13.0), theme::UI_TEXT, width);
        let mut browse = LayoutJob::default();
        browse.append("or ", 0.0, TextFormat::simple(theme::font(Weight::Regular, 12.0), theme::UI_MUTED));
        let link = TextFormat {
            underline: Stroke::new(1.0, theme::UI_TEXT),
            ..TextFormat::simple(theme::font(Weight::Regular, 12.0), theme::UI_TEXT)
        };
        browse.append("browse files", 0.0, link);
        DropZoneContent {
            title,
            browse: Some(ui.painter().layout_job(browse)),
            hint: self.hint.map(|h| muted(h, 11.0)),
        }
    }
}

struct DropZoneContent {
    title: Arc<Galley>,
    browse: Option<Arc<Galley>>,
    hint: Option<Arc<Galley>>,
}

impl Widget for DropZone<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        const BORDER: f32 = 1.5;
        let width = ui.available_width();
        let content = self.content(ui, width - 2.0 * (12.0 + BORDER));
        let height = if self.compact {
            2.0 * (BORDER + 9.0) + LINE
        } else {
            let hint = content.hint.as_ref().map_or(0.0, |_| 3.0 + 4.0 + 15.0);
            2.0 * (BORDER + 14.0) + 20.0 + 3.0 + 3.0 + LINE + 3.0 + LINE_SMALL + hint
        };
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), self.title));

        let painter = ui.painter();
        let active = response.enabled() && (self.highlight || response.hovered());
        let (border, icon) =
            if active { (theme::ACCENT, theme::ACCENT) } else { (theme::UI_LINE_STRONG, theme::UI_MUTED) };
        if active {
            painter.rect_filled(rect, 10.0, theme::alpha(theme::ACCENT, 0.06));
        }
        dashed_rect(painter, rect.shrink(BORDER / 2.0), 10.0, Stroke::new(BORDER, border));
        if self.compact {
            paint_compact_drop_zone(painter, rect, content.title, icon);
        } else {
            paint_full_drop_zone(painter, rect.shrink(BORDER + 14.0), content, icon);
        }
        if response.has_focus() {
            focus_ring(painter, rect, 10.0, 2.0);
        }
        response.on_hover_cursor(CursorIcon::PointingHand)
    }
}

fn paint_compact_drop_zone(painter: &Painter, rect: Rect, title: Arc<Galley>, icon_color: Color32) {
    const ICON: f32 = 16.0;
    let total = ICON + 6.0 + title.size().x;
    let left = rect.center().x - total / 2.0;
    paint_icon(
        painter,
        Rect::from_center_size(pos2(left + ICON / 2.0, rect.center().y), Vec2::splat(ICON)),
        Icon::Upload,
        icon_color,
        1.8,
    );
    let text_rect = Rect::from_x_y_ranges(left + ICON + 6.0..=rect.right(), rect.y_range());
    paint_galley(painter, text_rect, Align2::LEFT_CENTER, title, theme::UI_TEXT);
}

fn paint_full_drop_zone(painter: &Painter, inner: Rect, content: DropZoneContent, icon_color: Color32) {
    let line = |top: f32, height: f32| Rect::from_x_y_ranges(inner.x_range(), top..=top + height);
    paint_icon(
        painter,
        Rect::from_center_size(pos2(inner.center().x, inner.top() + 10.0), Vec2::splat(20.0)),
        Icon::Upload,
        icon_color,
        1.8,
    );
    let mut y = inner.top() + 20.0 + 3.0 + 3.0;
    paint_galley(painter, line(y, LINE), Align2::CENTER_CENTER, content.title, theme::UI_TEXT);
    y += LINE + 3.0;
    if let Some(browse) = content.browse {
        paint_galley(painter, line(y, LINE_SMALL), Align2::CENTER_CENTER, browse, theme::UI_MUTED);
    }
    y += LINE_SMALL + 3.0 + 4.0;
    if let Some(hint) = content.hint {
        paint_galley(painter, line(y, 15.0), Align2::CENTER_CENTER, hint, theme::UI_MUTED);
    }
}

/// Dashed outline of a rounded rectangle, centred on `rect`'s edge.
fn dashed_rect(painter: &Painter, rect: Rect, radius: f32, stroke: Stroke) {
    let mut points = Vec::new();
    rounded_rectangle(&mut points, rect, CornerRadiusF32::same(radius));
    if let Some(&first) = points.first() {
        points.push(first);
    }
    painter.extend(Shape::dashed_line(&points, stroke, 4.0, 3.0));
}

// ---- Shortcut picker ----

/// Marks a shortcut field that's waiting for a key, in egui's temp data under its id.
#[derive(Clone, Copy)]
struct Capturing;

/// A global-shortcut picker: shows the current combination; click it (or Enter), then
/// press the new one (Esc cancels). Returns the new shortcut, in `global-hotkey` syntax
/// (`Ctrl+Alt+Shift+O`), in the frame it's chosen.
pub fn shortcut_field(ui: &mut Ui, current: &str) -> Option<String> {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let id = response.id;
    let was_capturing = ui.data(|d| d.get_temp::<Capturing>(id).is_some());
    let mut capturing = was_capturing;
    let mut chosen = None;
    // Keys before clicks: Space or Enter on the focused field also counts as a click.
    if was_capturing {
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            capturing = false;
        } else if let Some(spec) = ui.input(pressed_shortcut) {
            chosen = Some(spec);
            capturing = false;
        }
    }
    if !was_capturing && response.clicked() {
        capturing = true;
    } else if was_capturing && (response.clicked_by(PointerButton::Primary) || response.clicked_elsewhere()) {
        // Only the pointer cancels: a keyboard "click" now is a key being picked.
        capturing = false;
    }
    ui.data_mut(|d| {
        if capturing {
            d.insert_temp(id, Capturing);
        } else {
            d.remove::<Capturing>(id);
        }
    });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), current));

    let painter = ui.painter();
    let (fill, border) = if capturing {
        (theme::alpha(theme::ACCENT, 0.1), theme::ACCENT)
    } else if hot(&response) {
        (white(0.08), theme::UI_LINE_STRONG)
    } else {
        (theme::UI_RAISED, theme::UI_LINE_STRONG)
    };
    painter.rect(rect, 7.0, fill, Stroke::new(1.0, border), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(12.0, 0.0));
    let (text, color) = if capturing { ("Press a shortcut…", theme::UI_MUTED) } else { (current, theme::UI_TEXT) };
    let galley = single_line(ui, text, theme::font(Weight::SemiBold, 12.5), color, inner.width() - 60.0);
    paint_galley(painter, inner, Align2::LEFT_CENTER, galley, color);
    let side = if capturing { "Esc cancels" } else { "Change" };
    let side = single_line(ui, side, theme::font(Weight::Regular, 12.0), theme::UI_MUTED, 60.0);
    paint_galley(painter, inner, Align2::RIGHT_CENTER, side, theme::UI_MUTED);
    if response.has_focus() {
        focus_ring(painter, rect, 7.0, 1.0);
    }
    chosen
}

/// Stops every shortcut field from waiting for a key (e.g. when its window closes).
pub fn cancel_shortcut_capture(ctx: &Context) {
    ctx.data_mut(|d| d.remove_by_type::<Capturing>());
}

/// The first key press this frame that makes a shortcut.
fn pressed_shortcut(i: &InputState) -> Option<String> {
    i.events.iter().find_map(|e| match e {
        // Shift changes the logical key (Ctrl+Shift+1 arrives as "!"): then the key's
        // place on the keyboard names it, as global-hotkey does.
        Event::Key { key, physical_key, pressed: true, modifiers, .. } => {
            shortcut_spec(*key, *modifiers).or_else(|| physical_key.and_then(|k| shortcut_spec(k, *modifiers)))
        }
        // egui-winit turns Ctrl+X/C/V (and Shift+Delete / Shift+Insert) into these,
        // without a key event; Paste only comes when the clipboard holds text.
        Event::Cut => shortcut_spec(if i.modifiers.ctrl { Key::X } else { Key::Delete }, i.modifiers),
        Event::Copy => shortcut_spec(Key::C, i.modifiers),
        Event::Paste(_) => shortcut_spec(if i.modifiers.ctrl { Key::V } else { Key::Insert }, i.modifiers),
        _ => None,
    })
}

/// A key press as a shortcut (`Ctrl+Shift+F10`), or `None` for keys a global shortcut
/// can't use. Letters and other typing keys need Ctrl or Alt, so the shortcut never
/// swallows ordinary typing; Alt+F4 stays Windows' "close window".
pub fn shortcut_spec(key: Key, m: Modifiers) -> Option<String> {
    let name = shortcut_key_name(key)?;
    if key == Key::F4 && m.alt && !m.ctrl && !m.shift {
        return None;
    }
    let function_key = matches!(
        key,
        Key::F1
            | Key::F2
            | Key::F3
            | Key::F4
            | Key::F5
            | Key::F6
            | Key::F7
            | Key::F8
            | Key::F9
            | Key::F10
            | Key::F11
            | Key::F12
    );
    if !(m.ctrl || m.alt || function_key) {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    if m.ctrl {
        parts.push("Ctrl");
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    parts.push(name);
    Some(parts.join("+"))
}

/// Key names `global-hotkey` understands.
fn shortcut_key_name(key: Key) -> Option<&'static str> {
    Some(match key {
        Key::Equals => "Equal",
        Key::OpenBracket => "BracketLeft",
        Key::CloseBracket => "BracketRight",
        Key::Backtick => "Backquote",
        Key::ArrowUp => "ArrowUp",
        Key::ArrowDown => "ArrowDown",
        Key::ArrowLeft => "ArrowLeft",
        Key::ArrowRight => "ArrowRight",
        Key::Minus | Key::Backslash | Key::Semicolon | Key::Quote | Key::Comma | Key::Period | Key::Slash => key.name(),
        Key::Space | Key::Insert | Key::Delete | Key::Home | Key::End | Key::PageUp | Key::PageDown => key.name(),
        _ => {
            let name = key.name();
            let letter_or_digit = name.len() == 1 && name.chars().all(|c| c.is_ascii_alphanumeric());
            let function =
                name.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=12).contains(&n));
            if !(letter_or_digit || function) {
                return None;
            }
            name
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_steps_inside_the_range() {
        let r = 100.0..=1500.0;
        assert_eq!(snap(524.0, &r, 50.0), 500.0);
        assert_eq!(snap(526.0, &r, 50.0), 550.0);
        assert_eq!(snap(20.0, &r, 50.0), 100.0);
        assert_eq!(snap(9000.0, &r, 50.0), 1500.0);
        assert_eq!(snap(6.3, &(1.0..=20.0), 0.5), 6.5);
        assert_eq!(snap(0.2, &(0.0..=15.0), 0.5), 0.0);
    }

    #[test]
    fn formats_readouts_like_the_prototype() {
        assert_eq!(format_value(80.0, "%", false), "80%");
        assert_eq!(format_value(500.0, " m", false), "500 m");
        assert_eq!(format_value(6.5, " s", false), "6.5 s");
        assert_eq!(format_value(8.0, " s", false), "8 s");
        assert_eq!(format_value(0.0, " m", true), "Off");
        assert_eq!(format_value(0.0, "%", false), "0%");
    }

    #[test]
    fn white_tints_are_premultiplied() {
        assert_eq!(white(0.08), Color32::from_rgba_unmultiplied(255, 255, 255, 20));
        assert_eq!(white(2.0), Color32::WHITE);
    }

    #[test]
    fn shortcuts_need_ctrl_or_alt_except_function_keys() {
        let ctrl_alt_shift = Modifiers { ctrl: true, alt: true, shift: true, ..Default::default() };
        assert_eq!(shortcut_spec(Key::O, ctrl_alt_shift).as_deref(), Some("Ctrl+Alt+Shift+O"));
        assert_eq!(shortcut_spec(Key::F10, Modifiers::SHIFT).as_deref(), Some("Shift+F10"));
        assert_eq!(shortcut_spec(Key::F9, Modifiers::NONE).as_deref(), Some("F9"));
        assert_eq!(shortcut_spec(Key::Backtick, Modifiers::CTRL).as_deref(), Some("Ctrl+Backquote"));
        assert_eq!(shortcut_spec(Key::O, Modifiers::SHIFT), None);
        assert_eq!(shortcut_spec(Key::Escape, Modifiers::CTRL), None);
        assert_eq!(shortcut_spec(Key::F20, Modifiers::CTRL), None);
        assert_eq!(shortcut_spec(Key::F4, Modifiers::ALT), None, "closes windows");
        assert_eq!(shortcut_spec(Key::F4, Modifiers::CTRL | Modifiers::ALT).as_deref(), Some("Ctrl+Alt+F4"));
    }

    const CTRL_ALT: Modifiers = Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: false, command: true };
    const CTRL_SHIFT: Modifiers = Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: true };

    /// A shortcut field alone in a headless egui, at the top left.
    struct Picker(Context);

    impl Picker {
        const FIELD: Pos2 = pos2(100.0, 15.0);

        fn new() -> Self {
            let ctx = Context::default();
            theme::install_fonts(&ctx);
            let picker = Self(ctx);
            picker.frame(Vec::new());
            picker
        }

        /// One frame with `events` (and the modifiers of the last key event held).
        fn frame(&self, events: Vec<Event>) -> Frame {
            let modifiers = events
                .iter()
                .rev()
                .find_map(|e| match e {
                    Event::Key { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default();
            self.frame_with(events, modifiers)
        }

        fn frame_with(&self, mut events: Vec<Event>, modifiers: Modifiers) -> Frame {
            if !events.is_empty() {
                events.insert(0, Event::ModifiersChanged(modifiers));
            }
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 100.0));
            let input = eframe::egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut picked = None;
            let mut out = self.0.run_ui(input, |ui| picked = shortcut_field(ui, "Ctrl+Alt+Shift+O"));
            out.textures_delta.clear();
            let repaint = out.viewport_output.values().any(|v| v.repaint_delay < std::time::Duration::MAX);
            let capturing = self.0.data(|d| d.count::<Capturing>() > 0);
            Frame { picked, capturing, repaint }
        }

        /// Clicks the field and checks it's waiting for a key.
        fn start(&self) {
            assert!(self.frame(click(Self::FIELD)).capturing);
        }
    }

    #[derive(Debug)]
    struct Frame {
        picked: Option<String>,
        capturing: bool,
        repaint: bool,
    }

    fn click(pos: Pos2) -> Vec<Event> {
        let button =
            |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        vec![Event::PointerMoved(pos), button(true), button(false)]
    }

    fn key(key: Key, physical_key: Option<Key>, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn picker_records_a_combination_and_escape_cancels() {
        let p = Picker::new();
        p.start();
        assert_eq!(p.frame(vec![key(Key::K, Some(Key::K), Modifiers::SHIFT)]).picked, None, "needs Ctrl or Alt");
        let f = p.frame(vec![key(Key::K, Some(Key::K), CTRL_ALT)]);
        assert_eq!((f.picked.as_deref(), f.capturing), (Some("Ctrl+Alt+K"), false));

        p.start();
        let f = p.frame(vec![key(Key::Escape, None, Modifiers::NONE)]);
        assert_eq!((f.picked, f.capturing), (None, false));
        assert!(p.frame(click(Picker::FIELD)).capturing, "a click starts again");
        assert!(!p.frame(click(Picker::FIELD)).capturing, "and another click stops");
    }

    #[test]
    fn picker_records_ctrl_c_x_and_v() {
        // egui-winit sends these instead of key events.
        for (event, modifiers, spec) in [
            (Event::Copy, CTRL_ALT, "Ctrl+Alt+C"),
            (Event::Cut, CTRL_ALT, "Ctrl+Alt+X"),
            (Event::Paste("clipboard".into()), CTRL_SHIFT, "Ctrl+Shift+V"),
            (Event::Cut, Modifiers::ALT | Modifiers::SHIFT, "Alt+Shift+Delete"),
        ] {
            let p = Picker::new();
            p.start();
            assert_eq!(p.frame_with(vec![event], modifiers).picked.as_deref(), Some(spec));
        }
    }

    #[test]
    fn picker_names_shifted_keys_by_their_place() {
        let p = Picker::new();
        p.start();
        let f = p.frame(vec![key(Key::Exclamationmark, Some(Key::Num1), CTRL_SHIFT)]);
        assert_eq!(f.picked.as_deref(), Some("Ctrl+Shift+1"));
    }

    #[test]
    fn picker_started_from_the_keyboard_takes_space_combinations() {
        let p = Picker::new();
        p.frame(vec![key(Key::Tab, None, Modifiers::NONE)]);
        assert!(p.frame(vec![key(Key::Enter, None, Modifiers::NONE)]).capturing, "Enter starts");
        // Space and Enter "click" the focused field; that mustn't cancel or swallow the key.
        assert!(p.frame(vec![key(Key::Space, None, Modifiers::NONE)]).capturing);
        let f = p.frame(vec![key(Key::Space, Some(Key::Space), CTRL_ALT)]);
        assert_eq!((f.picked.as_deref(), f.capturing), (Some("Ctrl+Alt+Space"), false));
    }

    #[test]
    fn waiting_for_a_key_doesnt_keep_repainting() {
        let p = Picker::new();
        p.start();
        // egui repaints once more after input; then nothing is scheduled.
        let frames: Vec<Frame> = (0..3).map(|_| p.frame(Vec::new())).collect();
        assert!(frames.iter().all(|f| f.capturing), "{frames:?}");
        assert!(!frames[2].repaint, "{frames:?}");
    }

    #[test]
    fn cancelling_capture_from_outside_stops_it() {
        let p = Picker::new();
        p.start();
        cancel_shortcut_capture(&p.0);
        let f = p.frame(vec![key(Key::K, Some(Key::K), CTRL_ALT)]);
        assert_eq!((f.picked, f.capturing), (None, false));
    }
}
