//! Overlay chrome: panel background, header (title, legend, badge, gear, close), resize
//! anchors, hover outline, size readout and the file-drop hint.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, Galley, Id, Painter, PointerButton, Pos2, Rect, ResizeDirection, Sense,
    Shadow, Shape, Stroke, StrokeKind, Ui, Vec2, emath::Rot2, pos2, text::LayoutJob, text::TextFormat, vec2,
};

use super::theme::{self, Weight};

/// Transparent border around the panel that holds the resize anchors, points.
pub const MARGIN: f32 = 8.0;
/// Height of the header strip at the top of the panel, points.
pub const HEADER_HEIGHT: f32 = 26.0;
/// Smallest panel, points: wide enough for the short title, the longest badge
/// ("REF: OTHER LAYOUT"), the gear and the close button.
pub const MIN_PANEL: Vec2 = vec2(272.0, 90.0);

const RADIUS: u8 = 8;
const BORDER_W: f32 = 1.0;
/// Content this wide or narrower gets the short title.
const SHORT_TITLE_MAX_W: f32 = 330.0;
/// Content this wide or narrower has no legend.
const NO_LEGEND_MAX_W: f32 = 540.0;
const PAD_LEFT: f32 = 11.0;
const PAD_RIGHT: f32 = 5.0;
/// Gap between the title and the legend.
const ITEM_GAP: f32 = 14.0;
/// The gear and close buttons, square.
const GEAR_SIZE: f32 = 22.0;
const GEAR_ICON: f32 = 16.0;
/// Space between the gear and the close button.
const BUTTON_GAP: f32 = 2.0;
/// Half the width of the close button's ×.
const CLOSE_ARM: f32 = 4.0;
/// The gear turns by half a tooth while the settings window is open.
const GEAR_OPEN_TURN: f32 = 67.5;
/// Header items are padded by this much when handed to the graph as obstacles.
const OBSTACLE_PAD: f32 = 2.0;
const FADE_S: f32 = 0.12;
/// Resize anchor fill (the prototype's `#0b1110`).
const ANCHOR_FILL: Color32 = Color32::from_rgb(0x0b, 0x11, 0x10);

/// The prototype's gear outline (24 × 24 view box): eight teeth around a ring.
const GEAR_OUTLINE: [(f32, f32); 32] = [
    (9.97, 4.68),
    (10.27, 1.95),
    (13.73, 1.95),
    (14.03, 4.68),
    (15.75, 5.39),
    (17.89, 3.67),
    (20.33, 6.11),
    (18.61, 8.25),
    (19.32, 9.97),
    (22.05, 10.27),
    (22.05, 13.73),
    (19.32, 14.03),
    (18.61, 15.75),
    (20.33, 17.89),
    (17.89, 20.33),
    (15.75, 18.61),
    (14.03, 19.32),
    (13.73, 22.05),
    (10.27, 22.05),
    (9.97, 19.32),
    (8.25, 18.61),
    (6.11, 20.33),
    (3.67, 17.89),
    (5.39, 15.75),
    (4.68, 14.03),
    (1.95, 13.73),
    (1.95, 10.27),
    (4.68, 9.97),
    (5.39, 8.25),
    (3.67, 6.11),
    (6.11, 3.67),
    (8.25, 5.39),
];
const GEAR_HOLE_R: f32 = 3.2;
const GEAR_STROKE: f32 = 1.7;

/// What the chrome shows this frame.
#[derive(Debug, Clone, Copy)]
pub struct Chrome<'a> {
    /// Panel background opacity, 0..1.
    pub opacity: f32,
    /// Click-through: no hover effects, grip, outline or anchors.
    pub locked: bool,
    pub settings_open: bool,
    /// Lap time for the legend's reference item; `None` hides the item.
    pub reference_time: Option<&'a str>,
    /// Status badges right of the legend, most important first (e.g. "DEMO").
    pub badges: &'a [&'a str],
    /// Files are being dragged over the window.
    pub file_hover: bool,
    /// The window is being (or was just) resized: show the size readout.
    pub resizing: bool,
}

/// Something the user started this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Pressed on the header: move the window.
    Move,
    /// Pressed on an anchor: resize the window.
    Resize(ResizeDirection),
    ToggleSettings,
    /// The close button: quit the overlay.
    Close,
}

pub struct Header {
    /// Header items the graph's labels must avoid, in painter coordinates.
    pub obstacles: Vec<Rect>,
    pub intent: Option<Intent>,
}

/// The panel inside the window: the window minus the anchor margin.
pub fn panel_rect(window: Rect) -> Rect {
    window.shrink(MARGIN)
}

/// Inside the panel's border: where the header and the graph go.
pub fn content_rect(window: Rect) -> Rect {
    panel_rect(window).shrink(BORDER_W)
}

/// The header strip at the top of the content.
fn header_rect(window: Rect) -> Rect {
    let content = content_rect(window);
    Rect::from_min_size(content.min, vec2(content.width(), HEADER_HEIGHT))
}

/// The close button, at the header's right end; the gear sits just left of it.
fn close_rect(header: Rect) -> Rect {
    Rect::from_center_size(
        pos2(header.right() - PAD_RIGHT - GEAR_SIZE / 2.0, header.center().y),
        Vec2::splat(GEAR_SIZE),
    )
}

/// Paints the panel background and the header, and senses the header drag, the gear
/// and the close button. Call before painting the graph.
pub fn panel_and_header(ui: &Ui, window: Rect, chrome: &Chrome) -> Header {
    let painter = ui.painter();
    paint_panel(painter, window, chrome.opacity);

    let content = content_rect(window);
    let header = header_rect(window);
    let layout = HeaderLayout::new(painter, header, chrome);

    let hover = hover_fade(ui, window, chrome);
    if hover > 0.0 {
        paint_grip(painter, content, hover);
    }
    layout.paint(painter);

    let mut intent = None;
    if !chrome.locked {
        // Stop short of the gear, or pressing it would start a move instead of a click.
        let drag_area = header.with_max_x(layout.gear.min.x - 4.0);
        let drag = ui.interact(drag_area, Id::new("ito-header"), Sense::drag()).on_hover_cursor(CursorIcon::Grab);
        if drag.drag_started_by(PointerButton::Primary) {
            intent = Some(Intent::Move);
        }
    }
    let gear = ui.interact(layout.gear, Id::new("ito-gear"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    if gear.clicked() {
        intent = Some(Intent::ToggleSettings);
    }
    let turn = ui.ctx().animate_bool_with_time_and_easing(
        Id::new("ito-gear-turn"),
        chrome.settings_open,
        0.3,
        egui::emath::easing::cubic_out,
    );
    paint_gear(painter, layout.gear, gear.hovered(), chrome.settings_open, turn);

    let close = ui
        .interact(layout.close, Id::new("ito-close"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("Close the overlay");
    if close.clicked() {
        intent = Some(Intent::Close);
    }
    paint_close(painter, layout.close, close.hovered());

    Header { obstacles: layout.obstacles(), intent }
}

/// Paints what sits above the graph (hover outline, drop hint, anchors, size readout)
/// and senses the anchors.
pub fn frame_controls(ui: &Ui, window: Rect, chrome: &Chrome) -> Option<Intent> {
    let painter = ui.painter();
    let panel = panel_rect(window);
    let hover = hover_fade(ui, window, chrome);
    if hover > 0.0 {
        let outline = theme::alpha(theme::ACCENT, 0.55 * hover);
        painter.rect_stroke(panel, CornerRadius::same(RADIUS), Stroke::new(1.0, outline), StrokeKind::Outside);
    }

    let drop = ui.ctx().animate_bool_with_time(Id::new("ito-drop"), chrome.file_hover, FADE_S);
    if drop > 0.0 {
        paint_drop_hint(painter, content_rect(window).shrink(5.0), drop);
    }

    let mut intent = None;
    if !chrome.locked {
        let content = content_rect(window);
        for (i, (dir, hit)) in anchor_hit_rects(window).into_iter().enumerate() {
            let resp = ui.interact(hit, Id::new(("ito-anchor", i)), Sense::drag()).on_hover_cursor(resize_cursor(dir));
            if resp.drag_started_by(PointerButton::Primary) {
                intent = Some(Intent::Resize(dir));
            }
            if hover > 0.0 {
                paint_anchor(painter, anchor_rect(content, dir), dir, resp.hovered() || resp.dragged(), hover);
            }
        }
    }

    let readout = ui.ctx().animate_bool_with_time(Id::new("ito-size"), chrome.resizing && !chrome.locked, FADE_S);
    if readout > 0.0 {
        paint_size_readout(painter, panel, readout);
    }
    intent
}

/// Where each resize anchor grabs the pointer: the whole margin plus a few points into
/// the panel, corners taking precedence over edges. The rects don't overlap each other
/// or the header's buttons, which are sensed first and would lose presses to them.
pub fn anchor_hit_rects(window: Rect) -> [(ResizeDirection, Rect); 8] {
    use ResizeDirection::*;
    let corner = MARGIN + BORDER_W + 9.0;
    let edge = MARGIN + BORDER_W + 5.0;
    let (l, r, t, b) = (window.left(), window.right(), window.top(), window.bottom());
    let square = |x: f32, y: f32| Rect::from_min_size(pos2(x, y), Vec2::splat(corner));
    let close = close_rect(header_rect(window));
    let north_bottom = (t + edge).min(close.top());
    let north_east = Rect::from_min_max(pos2((r - corner).max(close.right()), t), pos2(r, t + corner));
    [
        (NorthWest, square(l, t)),
        (NorthEast, north_east),
        (SouthWest, square(l, b - corner)),
        (SouthEast, square(r - corner, b - corner)),
        (North, Rect::from_min_max(pos2(l + corner, t), pos2(north_east.left(), north_bottom))),
        (South, Rect::from_min_max(pos2(l + corner, b - edge), pos2(r - corner, b))),
        (West, Rect::from_min_max(pos2(l, t + corner), pos2(l + edge, b - corner))),
        (East, Rect::from_min_max(pos2(r - edge, t + corner), pos2(r, b - corner))),
    ]
}

/// The visible handle: a 9 × 9 square on each corner, a 22 × 6 pill on each edge.
fn anchor_rect(content: Rect, dir: ResizeDirection) -> Rect {
    use ResizeDirection::*;
    let (center, size) = match dir {
        NorthWest => (content.left_top(), vec2(9.0, 9.0)),
        NorthEast => (content.right_top(), vec2(9.0, 9.0)),
        SouthWest => (content.left_bottom(), vec2(9.0, 9.0)),
        SouthEast => (content.right_bottom(), vec2(9.0, 9.0)),
        North => (content.center_top(), vec2(22.0, 6.0)),
        South => (content.center_bottom(), vec2(22.0, 6.0)),
        West => (content.left_center(), vec2(6.0, 22.0)),
        East => (content.right_center(), vec2(6.0, 22.0)),
    };
    Rect::from_center_size(center, size)
}

fn resize_cursor(dir: ResizeDirection) -> CursorIcon {
    use ResizeDirection::*;
    match dir {
        North | South => CursorIcon::ResizeVertical,
        East | West => CursorIcon::ResizeHorizontal,
        NorthWest | SouthEast => CursorIcon::ResizeNwSe,
        NorthEast | SouthWest => CursorIcon::ResizeNeSw,
    }
}

/// 0..1: how far the hover state (grip, outline, anchors) has faded in.
fn hover_fade(ui: &Ui, window: Rect, chrome: &Chrome) -> f32 {
    let pointer_in = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| window.contains(p));
    let active = !chrome.locked && (pointer_in || chrome.resizing);
    ui.ctx().animate_bool_with_time(Id::new("ito-hover"), active, FADE_S)
}

fn paint_panel(painter: &Painter, window: Rect, opacity: f32) {
    let panel = panel_rect(window);
    // A compact version of the prototype's shadow, so it fits in the margin. Clipped to
    // the outside of the panel, like a CSS box-shadow, so it doesn't darken the panel.
    let shadow = Shadow { offset: [0, 3], blur: 10, spread: 0, color: theme::alpha(Color32::BLACK, 0.45 * opacity) };
    let outside = [
        Rect::from_min_max(window.min, pos2(window.right(), panel.top())),
        Rect::from_min_max(pos2(window.left(), panel.bottom()), window.max),
        Rect::from_min_max(pos2(window.left(), panel.top()), pos2(panel.left(), panel.bottom())),
        Rect::from_min_max(pos2(panel.right(), panel.top()), pos2(window.right(), panel.bottom())),
    ];
    for clip in outside {
        painter.with_clip_rect(clip).add(shadow.as_shape(panel, CornerRadius::same(RADIUS)));
    }
    painter.rect(
        panel,
        CornerRadius::same(RADIUS),
        theme::alpha(theme::SURFACE, opacity),
        Stroke::new(BORDER_W, theme::alpha(theme::BORDER, 0.08 + 0.2 * opacity)),
        StrokeKind::Inside,
    );
}

/// Three dots left of the title: "drag here".
fn paint_grip(painter: &Painter, content: Rect, alpha: f32) {
    let color = theme::alpha(theme::HUD_MUTED, alpha);
    for k in 0..3 {
        painter.circle_filled(content.min + vec2(5.0, 10.0 + 4.0 * k as f32), 1.0, color);
    }
}

/// Text with CSS-style letter spacing (`tracking` in em).
fn spaced(job: &mut LayoutJob, text: &str, weight: Weight, size: f32, tracking: f32, color: Color32, leading: f32) {
    let format = TextFormat {
        font_id: theme::font(weight, size),
        color,
        extra_letter_spacing: tracking * size,
        ..Default::default()
    };
    job.append(text, leading, format);
}

fn text_galley(painter: &Painter, text: &str, weight: Weight, size: f32, tracking: f32, color: Color32) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    spaced(&mut job, text, weight, size, tracking, color, 0.0);
    painter.layout_job(job)
}

/// A legend entry: two small keys, then its text.
struct LegendItem {
    keys: LegendKeys,
    /// Where the two keys go (both keys and the gap between them).
    key_rect: Rect,
    text_pos: Pos2,
    galley: Arc<Galley>,
}

#[derive(Clone, Copy)]
enum LegendKeys {
    /// Live traces: two short lines.
    Lines,
    /// Reference: two gradient fills.
    Fills,
}

impl LegendItem {
    const KEY_GAP: f32 = 2.0;
    const TEXT_GAP: f32 = 5.0;

    fn new(keys: LegendKeys, galley: Arc<Galley>, left: f32, center_y: f32) -> Self {
        let key_size = match keys {
            LegendKeys::Lines => vec2(10.0, 2.0),
            LegendKeys::Fills => vec2(7.0, 9.0),
        };
        let key_rect = Rect::from_min_size(
            pos2(left, center_y - key_size.y / 2.0),
            vec2(2.0 * key_size.x + Self::KEY_GAP, key_size.y),
        );
        let text_pos = pos2(key_rect.right() + Self::TEXT_GAP, center_y - galley.size().y / 2.0);
        Self { keys, key_rect, text_pos, galley }
    }

    fn rect(&self) -> Rect {
        self.key_rect.union(Rect::from_min_size(self.text_pos, self.galley.size()))
    }

    fn paint(&self, painter: &Painter) {
        let w = (self.key_rect.width() - Self::KEY_GAP) / 2.0;
        for (i, color) in [theme::THROTTLE, theme::BRAKE].into_iter().enumerate() {
            let key = Rect::from_min_size(
                self.key_rect.min + vec2(i as f32 * (w + Self::KEY_GAP), 0.0),
                vec2(w, self.key_rect.height()),
            );
            match self.keys {
                LegendKeys::Lines => {
                    painter.rect_filled(key, CornerRadius::same(1), color);
                }
                LegendKeys::Fills => paint_fill_swatch(painter, key, color),
            }
        }
        painter.galley(self.text_pos, Arc::clone(&self.galley), theme::HUD_MUTED);
    }
}

/// A reference-fill swatch: colour fading to transparent downwards, rounded on top.
fn paint_fill_swatch(painter: &Painter, rect: Rect, color: Color32) {
    const R: f32 = 2.0;
    let top = theme::alpha(color, 0.75);
    let shade = |p: Pos2| top.lerp_to_gamma(Color32::TRANSPARENT, (p.y - rect.top()) / rect.height());
    let mut outline: Vec<Pos2> = arc(rect.left_top() + vec2(R, R), R, 180.0, 270.0)
        .chain(arc(rect.right_top() + vec2(-R, R), R, 270.0, 360.0))
        .collect();
    outline.extend([rect.right_bottom(), rect.left_bottom()]);

    // Convex, so a fan around the centre covers it.
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), shade(rect.center()));
    for &p in &outline {
        mesh.colored_vertex(p, shade(p));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(mesh);
}

/// Title, legend, badges, gear and close button, positioned for this header width.
struct HeaderLayout {
    title_pos: Pos2,
    title: Arc<Galley>,
    legend: Vec<LegendItem>,
    badges: Vec<(Rect, Arc<Galley>)>,
    gear: Rect,
    close: Rect,
}

impl HeaderLayout {
    const BADGE_GAP: f32 = 6.0;
    const BADGE_PAD: Vec2 = vec2(6.0, 3.0);

    fn new(painter: &Painter, header: Rect, chrome: &Chrome) -> Self {
        let cy = header.center().y;
        let title = title_galley(painter, header.width());
        let title_pos = pos2(header.left() + PAD_LEFT, cy - title.size().y / 2.0);
        let title_right = title_pos.x + title.size().x;
        let close = close_rect(header);
        let gear = close.translate(vec2(-(GEAR_SIZE + BUTTON_GAP), 0.0));

        let badges = place_badges(painter, chrome.badges, gear.left() - 8.0, title_right + ITEM_GAP, cy);
        let right_limit = badges.last().map_or(gear.left(), |(r, _)| r.left()) - ITEM_GAP;
        let mut legend = Vec::new();
        if header.width() > NO_LEGEND_MAX_W {
            legend = legend_items(painter, chrome.reference_time, title_right + ITEM_GAP, cy);
            if legend.last().is_some_and(|item| item.rect().right() > right_limit) {
                legend.clear();
            }
        }
        Self { title_pos, title, legend, badges, gear, close }
    }

    fn obstacles(&self) -> Vec<Rect> {
        let title = Rect::from_min_size(self.title_pos, self.title.size());
        let legend = self.legend.iter().map(LegendItem::rect).reduce(Rect::union);
        [Some(title), legend, Some(self.gear), Some(self.close)]
            .into_iter()
            .flatten()
            .chain(self.badges.iter().map(|(r, _)| *r))
            .map(|r| r.expand(OBSTACLE_PAD))
            .collect()
    }

    fn paint(&self, painter: &Painter) {
        painter.galley(self.title_pos, Arc::clone(&self.title), theme::HUD_TEXT);
        for item in &self.legend {
            item.paint(painter);
        }
        for (rect, galley) in &self.badges {
            painter.rect_filled(*rect, CornerRadius::same(5), theme::alpha(theme::WARN, 0.16));
            painter.galley(rect.min + Self::BADGE_PAD, Arc::clone(galley), theme::WARN);
        }
    }
}

/// "THROTTLE / BRAKE %", or "THR / BRK %" when the panel is narrow.
fn title_galley(painter: &Painter, width: f32) -> Arc<Galley> {
    let name = if width <= SHORT_TITLE_MAX_W { "THR / BRK" } else { "THROTTLE / BRAKE" };
    let mut job = LayoutJob::default();
    spaced(&mut job, name, Weight::Bold, 11.0, 0.07, theme::HUD_TEXT, 0.0);
    spaced(&mut job, "%", Weight::Bold, 11.0, 0.07, theme::HUD_MUTED, 5.0);
    painter.layout_job(job)
}

/// "LIVE" with line keys, then "REF 1:55.992" with fill keys when a reference is drawn.
fn legend_items(painter: &Painter, reference_time: Option<&str>, left: f32, cy: f32) -> Vec<LegendItem> {
    const SIZE: f32 = 10.0;
    const TRACKING: f32 = 0.06;
    const GAP: f32 = 12.0;
    let live = text_galley(painter, "LIVE", Weight::SemiBold, SIZE, TRACKING, theme::HUD_MUTED);
    let mut items = vec![LegendItem::new(LegendKeys::Lines, live, left, cy)];
    if let Some(time) = reference_time {
        let mut job = LayoutJob::default();
        spaced(&mut job, "REF ", Weight::SemiBold, SIZE, TRACKING, theme::HUD_MUTED, 0.0);
        spaced(&mut job, time, Weight::Bold, SIZE, TRACKING, theme::HUD_TEXT, 0.0);
        let left = items[0].rect().right() + GAP;
        items.push(LegendItem::new(LegendKeys::Fills, painter.layout_job(job), left, cy));
    }
    items
}

/// Badges right-aligned against `right`, dropping any that would cross `left_limit`.
fn place_badges(painter: &Painter, badges: &[&str], right: f32, left_limit: f32, cy: f32) -> Vec<(Rect, Arc<Galley>)> {
    let mut placed = Vec::new();
    let mut right = right;
    for text in badges {
        let galley = text_galley(painter, text, Weight::Bold, 10.0, 0.1, theme::WARN);
        let size = galley.size() + 2.0 * HeaderLayout::BADGE_PAD;
        let rect = Rect::from_min_size(pos2(right - size.x, cy - size.y / 2.0), size);
        if rect.left() < left_limit {
            break;
        }
        right = rect.left() - HeaderLayout::BADGE_GAP;
        placed.push((rect, galley));
    }
    placed
}

fn paint_gear(painter: &Painter, rect: Rect, hovered: bool, open: bool, turn: f32) {
    let (color, background) = if open {
        (theme::ACCENT, theme::alpha(theme::ACCENT, 0.12))
    } else if hovered {
        (theme::HUD_TEXT, theme::UI_LINE)
    } else {
        (theme::HUD_MUTED, Color32::TRANSPARENT)
    };
    painter.rect_filled(rect, CornerRadius::same(6), background);

    let scale = GEAR_ICON / 24.0;
    let center = rect.center();
    let rot = Rot2::from_angle((GEAR_OPEN_TURN * turn).to_radians());
    let outline = GEAR_OUTLINE.iter().map(|&(x, y)| center + rot * (vec2(x - 12.0, y - 12.0) * scale)).collect();
    let stroke = Stroke::new(GEAR_STROKE * scale, color);
    painter.add(Shape::closed_line(outline, stroke));
    painter.circle_stroke(center, GEAR_HOLE_R * scale, stroke);
}

/// A muted × that turns red on hover, sized to match the gear.
fn paint_close(painter: &Painter, rect: Rect, hovered: bool) {
    let (color, background) = if hovered {
        (theme::HUD_TEXT, theme::alpha(theme::DANGER, 0.22))
    } else {
        (theme::HUD_MUTED, Color32::TRANSPARENT)
    };
    painter.rect_filled(rect, CornerRadius::same(6), background);
    let c = rect.center();
    let stroke = Stroke::new(GEAR_STROKE * GEAR_ICON / 24.0 * 1.1, color);
    painter.line_segment([c + vec2(-CLOSE_ARM, -CLOSE_ARM), c + vec2(CLOSE_ARM, CLOSE_ARM)], stroke);
    painter.line_segment([c + vec2(CLOSE_ARM, -CLOSE_ARM), c + vec2(-CLOSE_ARM, CLOSE_ARM)], stroke);
}

fn paint_anchor(painter: &Painter, rect: Rect, dir: ResizeDirection, active: bool, alpha: f32) {
    use ResizeDirection::*;
    let radius = if matches!(dir, North | South | East | West) { 3 } else { 2 };
    painter.rect_filled(rect.expand(2.0), CornerRadius::same(radius + 2), theme::alpha(Color32::BLACK, 0.35 * alpha));
    let fill = if active { theme::ACCENT } else { ANCHOR_FILL };
    painter.rect(
        rect,
        CornerRadius::same(radius),
        theme::alpha(fill, alpha),
        Stroke::new(1.5, theme::alpha(theme::ACCENT, alpha)),
        StrokeKind::Inside,
    );
}

/// "680 × 170" in an accent pill, centred on the panel.
fn paint_size_readout(painter: &Painter, panel: Rect, alpha: f32) {
    let text = format!("{} × {}", panel.width().round() as i32, panel.height().round() as i32);
    let galley = text_galley(painter, &text, Weight::Bold, 12.0, 0.04, theme::ACCENT_INK);
    let pad = vec2(8.0, 5.0);
    let rect = Rect::from_center_size(panel.center(), galley.size() + 2.0 * pad);
    painter.rect_filled(rect, CornerRadius::same(5), theme::alpha(theme::ACCENT, alpha));
    painter.galley(rect.min + pad, galley, theme::alpha(theme::ACCENT_INK, alpha));
}

/// "Drop to set reference lap" over the panel while files are dragged over it.
fn paint_drop_hint(painter: &Painter, rect: Rect, alpha: f32) {
    const RADIUS: f32 = 6.0;
    const BORDER: f32 = 1.5;
    painter.rect_filled(rect, CornerRadius::same(RADIUS as u8), theme::alpha(theme::SURFACE, 0.9 * alpha));
    let outline = rounded_rect_path(rect.shrink(BORDER / 2.0), RADIUS - BORDER / 2.0);
    painter.extend(Shape::dashed_line(&outline, Stroke::new(BORDER, theme::alpha(theme::ACCENT, alpha)), 4.5, 3.0));

    let galley = text_galley(painter, "DROP TO SET REFERENCE LAP", Weight::Bold, 12.0, 0.07, theme::HUD_TEXT);
    const ICON: f32 = 20.0;
    const GAP: f32 = 8.0;
    let left = rect.center().x - (ICON + GAP + galley.size().x) / 2.0;
    let icon = Rect::from_min_size(pos2(left, rect.center().y - ICON / 2.0), Vec2::splat(ICON));
    paint_upload_icon(painter, icon, theme::alpha(theme::ACCENT, alpha));
    let text_pos = pos2(icon.right() + GAP, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_pos, galley, theme::alpha(theme::HUD_TEXT, alpha));
}

/// The prototype's upload icon (24 × 24 view box): an arrow out of a tray.
fn paint_upload_icon(painter: &Painter, rect: Rect, color: Color32) {
    let s = rect.width() / 24.0;
    let p = |x: f32, y: f32| rect.min + vec2(x, y) * s;
    let stroke = Stroke::new(1.8 * s, color);
    painter.line_segment([p(12.0, 15.0), p(12.0, 4.0)], stroke);
    painter.add(Shape::line(vec![p(7.5, 8.5), p(12.0, 4.0), p(16.5, 8.5)], stroke));
    let mut tray = vec![p(4.0, 15.0)];
    tray.extend(arc(p(6.0, 18.0), 2.0 * s, 180.0, 90.0));
    tray.extend(arc(p(18.0, 18.0), 2.0 * s, 90.0, 0.0));
    tray.push(p(20.0, 15.0));
    painter.add(Shape::line(tray, stroke));
}

/// Points along a circular arc from `from` to `to` degrees (y down, 0° = +x).
fn arc(center: Pos2, radius: f32, from: f32, to: f32) -> impl Iterator<Item = Pos2> {
    const STEPS: usize = 6;
    (0..=STEPS).map(move |k| {
        let a = (from + (to - from) * k as f32 / STEPS as f32).to_radians();
        center + radius * vec2(a.cos(), a.sin())
    })
}

/// A closed path around a rounded rectangle, clockwise from the top-left corner.
fn rounded_rect_path(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let mut points: Vec<Pos2> = [
        (pos2(rect.left() + r, rect.top() + r), 180.0, 270.0),
        (pos2(rect.right() - r, rect.top() + r), 270.0, 360.0),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0, 90.0),
        (pos2(rect.left() + r, rect.bottom() - r), 90.0, 180.0),
    ]
    .into_iter()
    .flat_map(|(center, from, to)| arc(center, r, from, to))
    .collect();
    if let Some(&first) = points.first() {
        points.push(first);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(w, h) + Vec2::splat(2.0 * MARGIN))
    }

    #[test]
    fn anchor_hit_areas_cover_the_margin_without_overlapping() {
        let win = window(680.0, 170.0);
        let hits = anchor_hit_rects(win);
        let close = close_rect(header_rect(win));
        let buttons = close.union(close.translate(vec2(-(GEAR_SIZE + BUTTON_GAP), 0.0)));
        for (i, (_, a)) in hits.iter().enumerate() {
            assert!(a.intersect(buttons).area() <= 0.0, "{a:?} covers the gear or close button");
            for (_, b) in &hits[i + 1..] {
                assert!(a.intersect(*b).area() <= 0.0, "{a:?} overlaps {b:?}");
            }
        }
        // Every point of the margin ring belongs to some anchor.
        let panel = panel_rect(win);
        for x in (0..=696).step_by(4) {
            for y in (0..=186).step_by(4) {
                let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
                if win.contains(p) && !panel.contains(p) {
                    assert!(hits.iter().any(|(_, r)| r.contains(p)), "{p:?} not covered");
                }
            }
        }
    }

    #[test]
    fn anchors_sit_on_the_panel_edges() {
        let content = content_rect(window(680.0, 170.0));
        assert_eq!(anchor_rect(content, ResizeDirection::NorthWest).center(), content.left_top());
        assert_eq!(anchor_rect(content, ResizeDirection::East).size(), vec2(6.0, 22.0));
        assert_eq!(anchor_rect(content, ResizeDirection::South).center(), content.center_bottom());
    }

    #[test]
    fn rounded_path_is_closed_and_inside() {
        let r = Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 40.0));
        let path = rounded_rect_path(r, 6.0);
        assert_eq!(path.first(), path.last());
        assert!(path.iter().all(|p| r.expand(0.01).contains(*p)));
    }

    /// Lays out the header in a real egui pass (fonts load on the first pass).
    fn header_for(width: f32, chrome: &Chrome) -> Vec<Rect> {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut obstacles = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ui| {
                obstacles = panel_and_header(ui, window(width, 170.0), chrome).obstacles;
            });
            out.textures_delta.clear();
        }
        obstacles
    }

    fn chrome<'a>(badges: &'a [&'a str], reference_time: Option<&'a str>) -> Chrome<'a> {
        Chrome {
            opacity: 0.8,
            locked: false,
            settings_open: false,
            reference_time,
            badges,
            file_hover: false,
            resizing: false,
        }
    }

    #[test]
    fn header_items_shrink_with_the_panel() {
        let wide = header_for(680.0, &chrome(&["DEMO"], Some("1:55.992")));
        assert_eq!(wide.len(), 5, "title, legend, gear, close, badge: {wide:?}");
        for (i, a) in wide.iter().enumerate() {
            for b in &wide[i + 1..] {
                assert!(!a.shrink(OBSTACLE_PAD).intersects(b.shrink(OBSTACLE_PAD)), "{a:?} overlaps {b:?}");
            }
        }
        let title_w = wide[0].width();

        let narrow = header_for(500.0, &chrome(&[], Some("1:55.992")));
        assert_eq!(narrow.len(), 3, "no legend below 540: {narrow:?}");

        let smallest = header_for(MIN_PANEL.x, &chrome(&["REF: OTHER LAYOUT"], None));
        assert_eq!(smallest.len(), 4, "the badge fits at the minimum width: {smallest:?}");

        let tiny = header_for(300.0, &chrome(&[], None));
        assert!(tiny[0].width() < title_w, "short title");
    }
}
