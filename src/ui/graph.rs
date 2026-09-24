//! The throttle/brake graph: a port of `prototype/src/graph.js` to egui's painter.

use std::sync::Arc;

use eframe::egui::{
    Align2, Color32, CornerRadius, Galley, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use crate::cue::Mark;
use crate::lap::Lap;
use crate::settings::{Axis, LabelMode};
use crate::trace::{LiveSample, LiveTrace};
use crate::ui::graph_layout::{self as layout, Decimator, Pin, PlotLayout, Scale, XBand, laps_in_view, place_rail};
use crate::ui::overlay;
use crate::ui::theme::{self, Weight};

/// The car right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarNow {
    /// Session seconds (same clock as `LiveSample::t`).
    pub t: f64,
    /// Cumulative lap position: laps since the trace started + lap fraction.
    /// Continuous across start/finish; `lap_pos * track_length` is `LiveSample::d`.
    pub lap_pos: f64,
    pub throttle: f32,
    pub brake: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelOptions {
    pub show: bool,
    pub mode: LabelMode,
    /// Peaks below this (0..1) get no label.
    pub min: f32,
}

pub struct GraphScene<'a> {
    pub axis: Axis,
    /// Visible span behind and ahead of the car, in the axis' units (metres or seconds).
    pub behind: f64,
    pub ahead: f64,
    /// Metres per lap (the session's `TrackLength`); live `d` values were computed with it.
    pub track_length: f64,
    /// `None`: nothing to plot yet. The grid and `message` are still drawn.
    pub now: Option<CarNow>,
    pub live: &'a LiveTrace,
    /// Reference lap to draw; `None` when hidden, missing, or for another track.
    pub reference: Option<&'a Lap>,
    /// Reference fill intensity, 0..1.
    pub ref_opacity: f32,
    pub labels: LabelOptions,
    /// Height of the header strip at the top of the panel; the plot starts below it.
    pub header_height: f32,
    /// Header items (title, legend, badges, gear) that labels must avoid, in painter coordinates.
    pub obstacles: &'a [Rect],
    /// Centred two-line message (title, detail), e.g. ("NO REFERENCE LAP", "Drop a Garage 61 CSV here").
    pub message: Option<(&'a str, &'a str)>,
    /// The reference brake points to mark, and your gaps to them; `None`: not shown.
    pub brake_points: Option<BrakePoints<'a>>,
    /// How far the panel's background has faded (1 − its opacity): the fills and text
    /// get a dark backing of their own in proportion, so they don't wash out.
    pub fade: f32,
}

/// What the brake point countdown knows, for the graph.
#[derive(Debug, Clone, Copy)]
pub struct BrakePoints<'a> {
    /// Indices of the reference zones that get a countdown.
    pub zones: &'a [usize],
    /// Your graded brake-ons.
    pub marks: &'a [Mark],
}

/// Start/finish hairline: white @ 22%.
const START_FINISH: Color32 = Color32::from_rgba_premultiplied(56, 56, 56, 56);
/// Car cursor: white @ 92%.
const CURSOR: Color32 = Color32::from_rgba_premultiplied(235, 235, 235, 235);
/// Dark outline around your peaks' numbers and pointers.
const PIN_OUTLINE: Color32 = Color32::from_rgba_premultiplied(6, 8, 9, 235); // surface @ 92%
/// Reference samples at or below this are on the floor: no edge line there.
const FLOOR: f32 = 0.004;
/// Extra window drawn past each side (fraction of the span) so lines run off the edges.
const OVERSCAN: f64 = 0.02;
/// egui centres a text's whole row box, which puts Barlow a point below where the
/// canvas' `middle` baseline (used for all the prototype's text) puts it.
const MIDDLE_BASELINE: Vec2 = vec2(0.0, -1.0);

/// Paints the graph into `panel` (the overlay panel's inner rect; its background is
/// already painted). Draws nothing outside `panel`.
pub fn paint(painter: &Painter, panel: Rect, scene: &GraphScene) {
    let painter = painter.with_clip_rect(panel);
    let rail = scene.labels.show && scene.reference.is_some() && scene.labels.mode != LabelMode::Live;
    let Some(PlotLayout { plot, compact, x_band }) = PlotLayout::new(panel, scene.header_height, rail) else {
        return;
    };
    let muted = theme::muted(scene.fade);
    paint_grid(&painter, plot, muted, scene.fade);
    let view = View::new(&painter, panel, plot, scene);
    if let Some(view) = &view {
        let ref_peaks = if rail { view.ref_peaks() } else { Vec::new() };
        let live_peaks =
            if scene.labels.show && scene.labels.mode != LabelMode::Reference { view.live_peaks() } else { Vec::new() };
        view.paint_data(&ref_peaks, &live_peaks);
        if let Some(points) = scene.brake_points {
            view.paint_brake_points(points);
        }
        view.paint_cursor();
        let rail_labels = view.paint_rail(&ref_peaks);
        view.paint_pins(&live_peaks, scene.obstacles.iter().copied().chain(rail_labels).collect());
        if x_band {
            view.paint_x_band(compact, muted);
        }
    }
    if let Some(message) = scene.message {
        let cursor_x = view.map(|v| v.scale.x(0.0)).filter(|_| scene.ahead > 0.0);
        paint_message(&painter, plot, cursor_x, message);
    }
}

/// Hairlines at 100, 50 and 0 % with their labels in the left gutter.
fn paint_grid(painter: &Painter, plot: Rect, muted: Color32, fade: f32) {
    let y = |p: f32| plot.top() + (1.0 - p) * plot.height();
    for (p, color) in [(1.0, theme::GRID), (0.5, theme::GRID), (0.0, theme::BASELINE)] {
        painter.hline(plot.x_range(), painter.round_to_pixel_center(y(p)), Stroke::new(1.0, color));
    }
    let labels: &[(f32, &str)] =
        if plot.height() >= 56.0 { &[(1.0, "100"), (0.5, "50"), (0.0, "0")] } else { &[(1.0, "100"), (0.0, "0")] };
    for &(p, text) in labels {
        let galley = painter.layout_no_wrap(text.to_owned(), theme::font(Weight::SemiBold, 10.0), muted);
        paint_galley_halo(painter, pos2(plot.left() - 6.0, y(p) + 0.5), Align2::RIGHT_CENTER, galley, muted, fade);
    }
}

/// The two-line message, centred in the look-ahead region when it fits there (`cursor_x`
/// is the car when there's a look-ahead), else across the plot.
fn paint_message(painter: &Painter, plot: Rect, cursor_x: Option<f32>, (title, detail): (&str, &str)) {
    let title = painter.layout_no_wrap(title.to_uppercase(), theme::font(Weight::Bold, 11.0), theme::HUD_TEXT);
    let detail = painter.layout_no_wrap(detail.to_string(), theme::font(Weight::Regular, 10.5), theme::HUD_MUTED);
    let mid = layout::message_center_x(plot, cursor_x, title.size().x.max(detail.size().x));
    let cy = plot.center().y;
    paint_galley(painter, pos2(mid, cy - 7.0), Align2::CENTER_CENTER, title, theme::HUD_TEXT);
    paint_galley(painter, pos2(mid, cy + 8.0), Align2::CENTER_CENTER, detail, theme::HUD_MUTED);
}

/// A current-value or peak dot on a surface-coloured ring; `hollow` for reference peaks.
fn paint_dot(painter: &Painter, at: Pos2, color: Color32, hollow: bool) {
    if hollow {
        painter.circle_filled(at, 4.5, theme::SURFACE);
        // egui strokes circles outside the radius: this is the canvas' 1.5 wide ring at 2.75.
        painter.circle_stroke(at, 2.0, Stroke::new(1.5, color));
    } else {
        painter.circle_filled(at, 6.0, theme::SURFACE);
        painter.circle_filled(at, 4.0, color);
    }
}

/// Paints laid-out text anchored at `pos` the way the prototype's canvas places text
/// with a middle baseline.
fn paint_galley(painter: &Painter, pos: Pos2, align: Align2, galley: Arc<Galley>, color: Color32) {
    let rect = align.anchor_size(pos + MIDDLE_BASELINE, galley.size());
    painter.galley(rect.min, galley, color);
}

/// As [`paint_galley`], on a dark halo as the panel's background fades by `fade`.
fn paint_galley_halo(painter: &Painter, pos: Pos2, align: Align2, galley: Arc<Galley>, color: Color32, fade: f32) {
    let rect = align.anchor_size(pos + MIDDLE_BASELINE, galley.size());
    overlay::halo_galley(painter, rect.min, galley, color, fade);
}

/// Dotted vertical lines (1 pt dots every 3 pt) at each of `xs`, from `top` down to
/// `bottom`, as one mesh.
fn dotted_vlines(painter: &Painter, xs: impl Iterator<Item = f32>, top: f32, bottom: f32, color: Color32) {
    let mut mesh = Mesh::default();
    for x in xs {
        let x = painter.round_to_pixel_center(x);
        let mut y = top;
        while y <= bottom {
            mesh.add_colored_rect(Rect::from_center_size(pos2(x, y), Vec2::splat(1.0)), color);
            y += 3.0;
        }
    }
    painter.add(mesh);
}

/// The data around the car: everything that needs a position.
struct View<'a> {
    painter: &'a Painter,
    scene: &'a GraphScene<'a>,
    panel: Rect,
    plot: Rect,
    scale: Scale,
    now: CarNow,
    /// The car on the x axis: cumulative metres or session seconds.
    key: f64,
    reference: Option<RefFrame<'a>>,
}

impl<'a> View<'a> {
    /// `None` when there's no car, or the window or track length can't be drawn.
    fn new(painter: &'a Painter, panel: Rect, plot: Rect, scene: &'a GraphScene<'a>) -> Option<Self> {
        let now = scene.now.filter(|n| n.t.is_finite() && n.lap_pos.is_finite())?;
        let now = CarNow { throttle: pedal(now.throttle), brake: pedal(now.brake), ..now };
        let scale = Scale::new(plot, scene.behind, scene.ahead)?;
        let has_length = scene.track_length.is_finite() && scene.track_length > 0.0;
        let key = match scene.axis {
            Axis::Distance if has_length => now.lap_pos * scene.track_length,
            Axis::Distance => return None,
            Axis::Time => now.t,
        };
        let reference = scene.reference.and_then(|lap| RefFrame::new(lap, scene, &now));
        Some(Self { painter, scene, panel, plot, scale, now, key, reference })
    }

    fn by_time(&self) -> bool {
        self.scene.axis == Axis::Time
    }

    /// Start/finish line, the reference fills, the dotted lines through the peaks and
    /// the live lines.
    fn paint_data(&self, ref_peaks: &[PeakMark], live_peaks: &[PeakMark]) {
        if !self.by_time() {
            for v in layout::start_finish_offsets(self.key, &self.scale, self.scene.track_length) {
                let x = self.painter.round_to_pixel_center(self.scale.x(v));
                self.painter.vline(x, self.plot.y_range(), Stroke::new(1.0, START_FINISH));
            }
        }
        // A little vertical slack so the 0 and 100 % lines aren't cut in half.
        let painter = self.painter.with_clip_rect(self.plot.expand2(vec2(0.0, 4.0)));
        let overscan = (self.scale.behind() + self.scale.ahead()) * OVERSCAN;
        let (lo, hi) = (-self.scale.behind() - overscan, self.scale.ahead() + overscan);
        if let Some(rf) = self.reference.as_ref().filter(|_| self.scene.ref_opacity > 0.0) {
            let samples = rf.samples(lo, hi);
            let opacity = self.scene.ref_opacity.min(1.0);
            self.paint_area(&painter, &samples, &rf.lap.throttle, theme::THROTTLE, opacity);
            self.paint_area(&painter, &samples, &rf.lap.brake, theme::BRAKE, opacity);
        }
        // A dotted line down through each peak, under your lines: gold for the
        // reference's, light blue for yours.
        let (top, bottom) = (self.plot.top(), self.scale.y(0.0));
        for (peaks, color) in [(ref_peaks, theme::TARGET), (live_peaks, theme::YOU)] {
            dotted_vlines(&painter, peaks.iter().map(|p| p.at.x), top, bottom, theme::alpha(color, 0.55));
        }
        let (throttle, brake) = self.live_lines(lo);
        paint_live_line(&painter, throttle, theme::THROTTLE);
        paint_live_line(&painter, brake, theme::BRAKE);
    }

    /// A reference series as a vertical-gradient area plus an edge line that lifts off
    /// along zero stretches, so the baseline stays clean. Thinned to one physical pixel
    /// per column first, like the live lines: a lap has more samples than the plot has
    /// pixels. As the panel's background fades, the area gets a dark backing of its own.
    fn paint_area(&self, painter: &Painter, samples: &[(f64, usize)], values: &[f32], color: Color32, opacity: f32) {
        let mut thin = Decimator::new(1.0 / self.painter.pixels_per_point());
        for &(v, i) in samples {
            thin.push(pos2(self.scale.x(v), self.scale.y(pedal(values[i]))));
        }
        let points = thin.finish();
        if points.len() < 2 {
            return;
        }
        let fade = self.scene.fade.clamp(0.0, 1.0);
        let shades = [
            (theme::alpha(theme::SURFACE, 0.55 * fade * opacity), theme::alpha(theme::SURFACE, 0.12 * fade * opacity)),
            (theme::alpha(color, 0.36 * opacity), theme::alpha(color, 0.02 * opacity)),
        ];
        let edge = Stroke::new(1.25, theme::alpha(color, 0.6 * opacity));
        let base = self.plot.bottom();
        let value_at = |p: Pos2| ((base - p.y) / self.plot.height()).clamp(0.0, 1.0);
        let lifted = |j: Option<usize>| j.and_then(|j| points.get(j)).is_some_and(|&p| value_at(p) > FLOOR);

        for (k, (top, bottom)) in shades.into_iter().enumerate() {
            if k == 0 && fade <= 0.0 {
                continue;
            }
            let mut mesh = Mesh::default();
            mesh.reserve_vertices(2 * points.len());
            mesh.reserve_triangles(2 * (points.len() - 1));
            for &p in &points {
                // Colour by height: the gradient runs from the plot top to the baseline.
                let n = mesh.vertices.len() as u32;
                mesh.colored_vertex(p, bottom.lerp_to_gamma(top, value_at(p)));
                mesh.colored_vertex(pos2(p.x, base), bottom);
                if n > 0 {
                    mesh.add_triangle(n - 2, n - 1, n);
                    mesh.add_triangle(n - 1, n + 1, n);
                }
            }
            painter.add(mesh);
        }
        let mut edges = Vec::new();
        let mut run = Vec::new();
        for (j, &p) in points.iter().enumerate() {
            if value_at(p) > FLOOR || lifted(j.checked_sub(1)) || lifted(Some(j + 1)) {
                run.push(p);
            } else {
                flush_run(&mut run, &mut edges, edge);
            }
        }
        flush_run(&mut run, &mut edges, edge);
        painter.extend(edges);
    }

    /// Live throttle and brake points from the window's left edge up to the car, thinned to
    /// one physical pixel per column: a slow or stopped car puts many samples on one x.
    fn live_lines(&self, lo: f64) -> (Vec<Pos2>, Vec<Pos2>) {
        let live = self.scene.live;
        let by_time = self.by_time();
        let start = live.first_at_or_after(self.key + lo, by_time).saturating_sub(1);
        let key = |s: &LiveSample| if by_time { s.t } else { s.d };
        let column = 1.0 / self.painter.pixels_per_point();
        let (mut throttle, mut brake) = (Decimator::new(column), Decimator::new(column));
        for s in live.samples().range(start..) {
            let x = self.scale.x(key(s) - self.key);
            throttle.push(pos2(x, self.scale.y(pedal(s.throttle))));
            brake.push(pos2(x, self.scale.y(pedal(s.brake))));
        }
        (throttle.finish(), brake.finish())
    }

    /// Cursor, playhead and current-value dots.
    fn paint_cursor(&self) {
        let (x, top) = (self.scale.x(0.0), self.plot.top());
        self.painter.line_segment([pos2(x, top - 2.0), pos2(x, self.plot.bottom())], Stroke::new(1.5, CURSOR));
        let playhead = vec![pos2(x - 4.0, top - 7.0), pos2(x + 4.0, top - 7.0), pos2(x, top - 2.0)];
        self.painter.add(Shape::convex_polygon(playhead, Color32::WHITE, Stroke::NONE));
        for (value, color) in [(self.now.throttle, theme::THROTTLE), (self.now.brake, theme::BRAKE)] {
            paint_dot(self.painter, pos2(x, self.scale.y(value)), color, false);
        }
    }

    /// The car's lap-distance pill and the axis ticks that fit around it.
    fn paint_x_band(&self, compact: bool, muted: Color32) {
        let painter = self.painter;
        let mut band = XBand::new(self.plot, self.panel);
        let lap_distance = self.now.lap_pos.rem_euclid(1.0) * self.scene.track_length;
        if lap_distance.is_finite() && self.scene.track_length > 0.0 {
            let text = format!("{}m", lap_distance.round() as i64);
            let galley = painter.layout_no_wrap(text, theme::font(Weight::Bold, 10.0), theme::SURFACE);
            let pill = band.place_pill(self.scale.x(0.0), galley.size().x.ceil() + 10.0);
            painter.rect_filled(pill, CornerRadius::same(3), theme::HUD_TEXT);
            paint_galley(painter, pill.center() + vec2(0.0, 0.5), Align2::CENTER_CENTER, galley, theme::SURFACE);
        }
        let ticks = if self.by_time() {
            layout::time_ticks(&self.scale, compact)
        } else {
            layout::distance_ticks(self.key, &self.scale, self.scene.track_length, compact)
        };
        for tick in ticks {
            let color = if tick.strong { theme::HUD_TEXT } else { muted };
            let galley = painter.layout_no_wrap(tick.text, theme::font(Weight::SemiBold, 10.0), color);
            let x = self.scale.x(tick.v);
            if band.place_tick(x, galley.size().x).is_some() {
                let at = pos2(x, band.top + 6.5);
                paint_galley_halo(painter, at, Align2::CENTER_CENTER, galley, color, self.scene.fade);
            }
        }
    }

    fn peak(&self, v: f64, value: f32) -> PeakMark {
        PeakMark { v, at: pos2(self.scale.x(v), self.scale.y(pedal(value))), value }
    }

    /// The reference's brake peaks in view, on every lap copy, above the labels' minimum.
    fn ref_peaks(&self) -> Vec<PeakMark> {
        let Some(rf) = self.reference.as_ref().filter(|_| self.scene.labels.mode != LabelMode::Live) else {
            return Vec::new();
        };
        let (behind, ahead) = (self.scale.behind(), self.scale.ahead());
        let mut peaks = Vec::new();
        for z in rf.lap.zones.iter().filter(|z| z.peak >= self.scene.labels.min && z.peak_idx < rf.n) {
            for base in rf.lap_offsets(-behind, ahead) {
                let v = base + rf.at(z.peak_idx);
                if (-behind..=ahead).contains(&v) {
                    peaks.push(self.peak(v, z.peak));
                }
            }
        }
        peaks
    }

    /// Your brake peaks in the history behind the car, nearest the car first.
    fn live_peaks(&self) -> Vec<PeakMark> {
        if self.scene.labels.mode == LabelMode::Reference {
            return Vec::new();
        }
        let behind = self.scale.behind();
        let mut peaks: Vec<PeakMark> = self
            .scene
            .live
            .events()
            .filter(|e| e.peak >= self.scene.labels.min)
            .filter_map(|e| {
                let v = if self.by_time() { e.peak_t } else { e.peak_d } - self.key;
                (-behind..=0.0).contains(&v).then(|| self.peak(v, e.peak))
            })
            .collect();
        peaks.sort_by(|a, b| b.v.total_cmp(&a.v));
        // Peaks on top of each other (a car stopped with the brake pumping) can't be told
        // apart: keep the latest.
        let mut kept: Vec<PeakMark> = Vec::with_capacity(peaks.len());
        for p in peaks {
            if kept.iter().all(|k| (k.at.x - p.at.x).abs() >= 1.0) {
                kept.push(p);
            }
        }
        kept
    }

    /// Gold rings on the reference's peaks, then each peak's label on the rail above the
    /// plot, at the top of its dotted line. Returns the labels' boxes.
    fn paint_rail(&self, peaks: &[PeakMark]) -> Vec<Rect> {
        let painter = self.painter;
        for p in peaks {
            paint_dot(painter, p.at, theme::TARGET, true);
        }
        let font = theme::font(Weight::SemiBold, 10.0);
        let galleys: Vec<Arc<Galley>> =
            peaks.iter().map(|p| painter.layout_no_wrap(p.text(), font.clone(), theme::TARGET)).collect();
        let spots: Vec<(f32, f32)> =
            peaks.iter().zip(&galleys).map(|(p, g)| (p.at.x, g.size().x.ceil() + 8.0)).collect();
        let mut labels = Vec::new();
        for (spot, galley) in place_rail(self.plot, self.scale.x(0.0), &spots).into_iter().zip(galleys) {
            let Some(rect) = spot else { continue };
            let stroke = Stroke::new(1.0, theme::alpha(theme::TARGET, 0.8));
            painter.rect(
                rect.shrink(0.5),
                CornerRadius::same(3),
                theme::alpha(theme::SURFACE, 0.72),
                stroke,
                StrokeKind::Middle,
            );
            paint_galley(painter, rect.center() + vec2(0.0, 0.5), Align2::CENTER_CENTER, galley, theme::TARGET);
            labels.push(rect);
        }
        labels
    }

    /// Your peaks' numbers, light blue on a dark outline, pinned to the apex of your
    /// brake line and clear of `taken` (the header, the rail labels, each other).
    fn paint_pins(&self, peaks: &[PeakMark], mut taken: Vec<Rect>) {
        let painter = self.painter;
        let top = self.panel.top() + 2.0;
        let font = theme::font(Weight::Bold, 10.5);
        let pins: Vec<(Pin, Pos2, Arc<Galley>)> = peaks
            .iter()
            .map(|p| {
                let galley = painter.layout_no_wrap(p.text(), font.clone(), theme::YOU);
                let pin = Pin::place(p.at, galley.size().x.ceil() + 4.0, self.plot, top, &taken);
                taken.push(pin.rect);
                (pin, p.at, galley)
            })
            .collect();
        for (pin, apex, galley) in pins {
            let pointer = pin.pointer(apex).to_vec();
            painter.add(Shape::closed_line(pointer.clone(), Stroke::new(2.0, PIN_OUTLINE)));
            painter.add(Shape::convex_polygon(pointer, theme::YOU, Stroke::NONE));
            let at = pin.rect.center() + vec2(0.0, 0.5) + MIDDLE_BASELINE;
            paint_outlined(painter, Align2::CENTER_CENTER.anchor_size(at, galley.size()).min, galley, theme::YOU);
        }
    }

    /// A mark on the baseline at each reference brake point the countdown uses, and just
    /// under it your gap to it in the grade's colour: from the reference brake-on to yours.
    fn paint_brake_points(&self, points: BrakePoints) {
        let Some(rf) = &self.reference else { return };
        let (behind, ahead) = (self.scale.behind(), self.scale.ahead());
        let columns = Rect::from_x_y_ranges(self.plot.x_range(), self.panel.y_range());
        let painter = self.painter.with_clip_rect(columns.intersect(self.painter.clip_rect()));
        let zones = &rf.lap.zones;
        let brake_on = |k: usize| rf.at(zones[k].start.min(rf.n - 1));
        let base = painter.round_to_pixel_center(self.scale.y(0.0));
        for lap in rf.lap_offsets(-behind, ahead) {
            for &k in points.zones.iter().filter(|&&k| k < zones.len()) {
                let v = lap + brake_on(k);
                if (-behind..=ahead).contains(&v) {
                    let x = painter.round_to_pixel_center(self.scale.x(v));
                    let mark = vec![pos2(x - 3.5, base - 0.5), pos2(x, base - 6.5), pos2(x + 3.5, base - 0.5)];
                    painter.add(Shape::convex_polygon(mark, theme::BRAKE, Stroke::NONE));
                }
            }
        }
        let y = base + 2.0;
        for mark in points.marks.iter().filter(|m| m.zone < zones.len()) {
            let v0 = mark.lap as f64 * rf.period + brake_on(mark.zone) - rf.center;
            let v1 = if self.by_time() { mark.on_t - self.now.t } else { mark.on_d - self.key };
            if v0.max(v1) < -behind || v0.min(v1) > ahead {
                continue;
            }
            let (x0, x1) = (self.scale.x(v0), self.scale.x(v1));
            let color = theme::grade_color(mark.grade);
            painter.line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(2.5, color));
            painter.line_segment([pos2(x1, y - 4.0), pos2(x1, y + 2.0)], Stroke::new(1.5, color));
        }
    }
}

/// A brake peak in view.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PeakMark {
    /// Offset from the car.
    v: f64,
    /// The peak on the plot.
    at: Pos2,
    /// Pedal, 0..1.
    value: f32,
}

impl PeakMark {
    /// `68%`.
    fn text(&self) -> String {
        format!("{}%", (self.value * 100.0).round() as i32)
    }
}

/// Text on a solid dark outline (about 1.75 pt), so it reads over the traces and fills.
fn paint_outlined(painter: &Painter, pos: Pos2, galley: Arc<Galley>, color: Color32) {
    for r in [1.0, 1.75] {
        for (dx, dy) in
            [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (0.7, 0.7), (-0.7, 0.7), (0.7, -0.7), (-0.7, -0.7)]
        {
            painter.galley_with_override_text_color(pos + r * vec2(dx, dy), Arc::clone(&galley), PIN_OUTLINE);
        }
    }
    painter.galley(pos, galley, color);
}

/// Where the reference lap sits on the x axis: one copy per lap, placed relative to the car.
struct RefFrame<'a> {
    lap: &'a Lap,
    /// Samples present in every column.
    n: usize,
    axis: Axis,
    track_length: f64,
    /// The car's position in the reference's own units, laps included.
    center: f64,
    /// One lap in axis units.
    period: f64,
}

impl<'a> RefFrame<'a> {
    fn new(lap: &'a Lap, scene: &GraphScene, now: &CarNow) -> Option<Self> {
        let n = lap.pct.len().min(lap.brake.len()).min(lap.throttle.len());
        if n < 2 {
            return None;
        }
        let (center, period) = match scene.axis {
            Axis::Distance => (now.lap_pos * scene.track_length, scene.track_length),
            Axis::Time => {
                let laps = now.lap_pos.floor();
                (laps * lap.lap_time + lap.index_at_pct(now.lap_pos - laps) / lap.hz, lap.lap_time)
            }
        };
        (center.is_finite() && period > 0.0 && lap.hz > 0.0).then_some(Self {
            lap,
            n,
            axis: scene.axis,
            track_length: scene.track_length,
            center,
            period,
        })
    }

    /// Axis position of sample `i` within its lap.
    fn at(&self, i: usize) -> f64 {
        match self.axis {
            Axis::Distance => self.lap.pct[i] * self.track_length,
            Axis::Time => i as f64 / self.lap.hz,
        }
    }

    /// First sample at or after `pos` within the lap.
    fn first_at_or_after(&self, pos: f64) -> usize {
        match self.axis {
            Axis::Distance => self.lap.pct[..self.n].partition_point(|&p| p * self.track_length < pos),
            Axis::Time => ((pos * self.lap.hz).ceil().max(0.0) as usize).min(self.n),
        }
    }

    /// Offset from the car of the start of each lap copy overlapping `[lo, hi]`.
    fn lap_offsets(&self, lo: f64, hi: f64) -> impl Iterator<Item = f64> {
        let (center, period) = (self.center, self.period);
        laps_in_view(center + lo, center + hi, period).into_iter().flatten().map(move |k| k as f64 * period - center)
    }

    /// Samples in `[lo, hi]` plus one past each end, wrapping across start/finish:
    /// (offset from the car, sample index).
    fn samples(&self, lo: f64, hi: f64) -> Vec<(f64, usize)> {
        let mut out = Vec::new();
        for base in self.lap_offsets(lo, hi) {
            for i in self.first_at_or_after(lo - base).saturating_sub(1)..self.n {
                let v = base + self.at(i);
                out.push((v, i));
                if v > hi {
                    break;
                }
            }
        }
        out
    }
}

/// Pedal travel limited to 0..1 (0 when unknown).
fn pedal(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// Ends the current edge-line run, keeping it if it has a segment.
fn flush_run(run: &mut Vec<Pos2>, edges: &mut Vec<Shape>, stroke: Stroke) {
    if run.len() >= 2 {
        edges.push(Shape::line(std::mem::take(run), stroke));
    } else {
        run.clear();
    }
}

/// A live series: dark underlay (keeps it legible over its own fill), then the line.
fn paint_live_line(painter: &Painter, points: Vec<Pos2>, color: Color32) {
    if points.len() < 2 {
        return;
    }
    painter.line(points.clone(), Stroke::new(4.0, theme::alpha(theme::SURFACE, 0.55)));
    painter.line(points, Stroke::new(2.0, color));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::sample_lap;

    const L: f64 = 5796.1;

    fn scene<'a>(axis: Axis, live: &'a LiveTrace, reference: Option<&'a Lap>, now: CarNow) -> GraphScene<'a> {
        let (behind, ahead) = match axis {
            Axis::Distance => (500.0, 500.0),
            Axis::Time => (8.0, 6.0),
        };
        GraphScene {
            axis,
            behind,
            ahead,
            track_length: L,
            now: Some(now),
            live,
            reference,
            ref_opacity: 1.0,
            labels: LabelOptions { show: true, mode: LabelMode::Both, min: 0.1 },
            header_height: 26.0,
            obstacles: &[],
            message: None,
            brake_points: None,
            fade: 0.0,
        }
    }

    fn car(lap_pos: f64) -> CarNow {
        CarNow { t: 100.0, lap_pos, throttle: 0.0, brake: 0.0 }
    }

    #[test]
    fn reference_samples_wrap_across_start_finish() {
        let lap = sample_lap();
        let live = LiveTrace::new();
        let s = scene(Axis::Distance, &live, Some(&lap), car(1.02)); // 116 m into lap 2
        let rf = RefFrame::new(&lap, &s, &s.now.unwrap()).unwrap();
        let samples = rf.samples(-500.0, 500.0);
        let xs: Vec<f64> = samples.iter().map(|&(v, _)| v).collect();
        assert!(xs.windows(2).all(|w| w[0] <= w[1]), "offsets increase through the wrap");
        assert!(xs[0] < -500.0 && xs[1] >= -500.0);
        assert!(xs[xs.len() - 1] > 500.0 && xs[xs.len() - 2] <= 500.0);
        // The end of lap 1 then the start of lap 2.
        let wrap = samples.windows(2).position(|w| w[1].1 < w[0].1).unwrap();
        assert_eq!((samples[wrap].1, samples[wrap + 1].1), (lap.n() - 1, 0));
    }

    #[test]
    fn reference_time_axis_lines_up_with_the_car() {
        let lap = sample_lap();
        let live = LiveTrace::new();
        let s = scene(Axis::Time, &live, Some(&lap), car(2.5));
        let rf = RefFrame::new(&lap, &s, &s.now.unwrap()).unwrap();
        // Two laps done, half way round the third: the car sits at the reference's own
        // time for half a lap.
        let half = lap.index_at_pct(0.5) / lap.hz;
        assert!((rf.center - (2.0 * lap.lap_time + half)).abs() < 1e-9);
        let samples = rf.samples(-8.0, 6.0);
        let (v, i) = samples[samples.len() / 2];
        assert!((v - (i as f64 / lap.hz - half)).abs() < 1e-9);
        assert!(samples.len() > 14 * 59 && samples.len() < 14 * 61 + 3);
    }

    #[test]
    fn peaks_cover_live_events_and_every_reference_copy() {
        let lap = sample_lap();
        let mut live = LiveTrace::new();
        for (i, b) in [0.0, 0.3, 0.7, 0.2, 0.0, 0.05, 0.08].into_iter().enumerate() {
            let d = L * 1.9 + i as f64 * 20.0;
            live.push(LiveSample { t: i as f64, d, brake: b, throttle: 0.0 });
        }
        // 5336 m into lap 2 with 1500 m of look-ahead: S/F is 460 m ahead.
        let now = car((L * 1.9 + 120.0) / L);
        let mut s = scene(Axis::Distance, &live, Some(&lap), now);
        s.ahead = 1500.0;
        let painter =
            Painter::new(eframe::egui::Context::default(), eframe::egui::LayerId::background(), Rect::EVERYTHING);
        let panel = Rect::from_min_size(Pos2::ZERO, vec2(680.0, 170.0));
        let plot = PlotLayout::new(panel, 26.0, true).unwrap().plot;
        let view = View::new(&painter, panel, plot, &s).unwrap();
        // Yours behind the car (the active 8% event is under the 10% minimum).
        let yours = view.live_peaks();
        assert_eq!(yours.len(), 1);
        assert!(yours[0].value == 0.7 && (yours[0].v + 80.0).abs() < 1e-6);
        assert_eq!(yours[0].at, pos2(view.scale.x(yours[0].v), view.scale.y(0.7)));
        // The reference's, nearest first: 77% and 67% on this lap, 68% and 46% on the next.
        let mut refs = view.ref_peaks();
        refs.sort_by(|a, b| a.v.abs().total_cmp(&b.v.abs()));
        let values: Vec<u32> = refs.iter().map(|p| (p.value * 100.0).round() as u32).collect();
        assert_eq!(values, [77, 67, 68, 46]);
        assert!((refs[2].v - (2.0 * L + lap.pct[lap.zones[0].peak_idx] * L - now.lap_pos * L)).abs() < 1e-6);
        assert_eq!(refs[0].text(), "77%");

        s.labels.mode = LabelMode::Live;
        assert!(View::new(&painter, panel, plot, &s).unwrap().ref_peaks().is_empty());
        s.labels.mode = LabelMode::Reference;
        assert!(View::new(&painter, panel, plot, &s).unwrap().live_peaks().is_empty());
    }

    /// Everything `paint` draws for `scene` on a `size` panel.
    fn painted(scene: &GraphScene, size: Vec2) -> Vec<Shape> {
        let ctx = eframe::egui::Context::default();
        theme::install_fonts(&ctx);
        let panel = Rect::from_min_size(Pos2::ZERO, size);
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ui| paint(ui.painter(), panel, scene));
            out.textures_delta.clear();
            shapes = out.shapes.into_iter().map(|c| c.shape).collect();
        }
        shapes
    }

    fn texts(shapes: &[Shape]) -> Vec<String> {
        let mut out = Vec::new();
        for shape in shapes {
            match shape {
                Shape::Text(t) => out.push(t.galley.text().to_owned()),
                Shape::Vec(v) => out.extend(texts(v)),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn reference_peaks_are_labelled_on_the_rail_and_yours_at_the_apex() {
        let lap = sample_lap();
        let mut live = LiveTrace::new();
        for (i, b) in [0.0, 0.3, 0.7, 0.2, 0.0].into_iter().enumerate() {
            live.push(LiveSample { t: i as f64, d: L * 0.6 + i as f64 * 20.0, brake: b, throttle: 0.0 });
        }
        let s = scene(Axis::Distance, &live, Some(&lap), car(0.6 + 120.0 / L));
        let labels = texts(&painted(&s, vec2(680.0, 170.0)));
        assert!(labels.iter().any(|t| t == "70%"), "your peak: {labels:?}");
        let rail: Vec<&String> = labels.iter().filter(|t| t.ends_with('%') && *t != "70%").collect();
        assert!(!rail.is_empty(), "reference peaks: {labels:?}");

        let off = GraphScene {
            labels: LabelOptions { show: false, ..s.labels },
            ..scene(Axis::Distance, &live, Some(&lap), car(0.6 + 120.0 / L))
        };
        assert!(texts(&painted(&off, vec2(680.0, 170.0))).iter().all(|t| !t.ends_with('%')), "labels off");
    }

    #[test]
    fn brake_points_are_marked_on_the_baseline() {
        let lap = sample_lap();
        let live = LiveTrace::new();
        let zones: Vec<usize> = (0..lap.zones.len()).collect();
        let now = car(lap.pct[lap.zones[0].start] + 100.0 / L);
        let mut s = scene(Axis::Distance, &live, Some(&lap), now);
        s.labels.show = false;
        let marks = [Mark {
            lap: 0,
            zone: 0,
            grade: crate::cue::Grade::Late,
            on_t: 0.0,
            on_d: lap.pct[lap.zones[0].start] * L + 12.0,
        }];
        s.brake_points = Some(BrakePoints { zones: &zones, marks: &marks });
        let shapes = painted(&s, vec2(680.0, 170.0));
        let is_mark = |shape: &Shape| matches!(shape, Shape::Path(p) if p.fill == theme::BRAKE && p.points.len() == 3);
        let panel = Rect::from_min_size(Pos2::ZERO, vec2(680.0, 170.0));
        let plot = PlotLayout::new(panel, 26.0, false).unwrap().plot;
        let scale = Scale::new(plot, 500.0, 500.0).unwrap();
        // Zone 0's brake point is 100 m behind the car.
        let x = scale.x(-100.0);
        let tips: Vec<f32> = shapes
            .iter()
            .filter(|shape| is_mark(shape))
            .map(|shape| match shape {
                Shape::Path(p) => p.points[1].x,
                _ => unreachable!(),
            })
            .collect();
        assert!(tips.iter().any(|t| (t - x).abs() <= 1.0), "{x} in {tips:?}");
        // Your late brake-on, 12 m after it: an orange bar under the baseline.
        let late = theme::grade_color(crate::cue::Grade::Late);
        let bar = shapes.iter().any(|shape| match shape {
            Shape::LineSegment { points, stroke } => {
                stroke.color == late && (points[0].x - x).abs() < 1.5 && (points[1].x - scale.x(-88.0)).abs() < 1.5
            }
            _ => false,
        });
        assert!(bar, "grade bar from the reference brake-on to yours");

        s.brake_points = None;
        assert!(!painted(&s, vec2(680.0, 170.0)).iter().any(is_mark), "off");
    }

    #[test]
    fn nothing_to_view_without_a_usable_car_or_window() {
        let live = LiveTrace::new();
        let painter =
            Painter::new(eframe::egui::Context::default(), eframe::egui::LayerId::background(), Rect::EVERYTHING);
        let panel = Rect::from_min_size(Pos2::ZERO, vec2(680.0, 170.0));
        let plot = PlotLayout::new(panel, 26.0, false).unwrap().plot;
        let mut s = scene(Axis::Distance, &live, None, car(f64::NAN));
        assert!(View::new(&painter, panel, plot, &s).is_none());
        s.now = Some(car(0.5));
        s.track_length = 0.0;
        assert!(View::new(&painter, panel, plot, &s).is_none());
        s.axis = Axis::Time;
        assert!(View::new(&painter, panel, plot, &s).is_some());
    }

    /// 70 s of driving the sample lap, then 10 minutes stopped with both pedals going
    /// through their whole travel, fed like `LiveFeed::push`.
    fn stopped_with_pedals_moving() -> (LiveTrace, CarNow) {
        let mut live = LiveTrace::new();
        let mut driver = crate::demo::SimulatedDriver::new(std::sync::Arc::new(sample_lap()), 3);
        let mut tracker = crate::trace::DistanceTracker::new();
        let mut now = car(0.0);
        for i in 0..(670 * 60) {
            let mut f = driver.step(1.0 / 60.0);
            if i >= 70 * 60 {
                f.lap_dist_pct = now.lap_pos.rem_euclid(1.0);
                (f.throttle, f.brake) = ((i % 11) as f32 / 10.0, (i % 60) as f32 / 59.0);
            }
            let crate::trace::Progress::Continuous(lap_pos) = tracker.update(f.session_time, f.lap_dist_pct) else {
                continue;
            };
            let d = lap_pos * L;
            live.push(LiveSample { t: f.session_time, d, brake: f.brake, throttle: f.throttle });
            live.prune(f.session_time - 25.0, d - 1600.0);
            now = CarNow { t: f.session_time, lap_pos, throttle: f.throttle, brake: f.brake };
        }
        (live, now)
    }

    #[test]
    fn a_stopped_car_draws_a_line_no_wider_than_the_plot() {
        let (live, now) = stopped_with_pedals_moving();
        assert_eq!(live.len(), crate::trace::MAX_SAMPLES);
        let painter =
            Painter::new(eframe::egui::Context::default(), eframe::egui::LayerId::background(), Rect::EVERYTHING);
        let panel = Rect::from_min_size(Pos2::ZERO, vec2(680.0, 170.0));
        let plot = PlotLayout::new(panel, 26.0, false).unwrap().plot;
        for axis in [Axis::Distance, Axis::Time] {
            let s = scene(axis, &live, None, now);
            let view = View::new(&painter, panel, plot, &s).unwrap();
            let (throttle, brake) = view.live_lines(-view.scale.behind() * 1.02);
            for line in [&throttle, &brake] {
                assert!(line.len() <= 4 * (plot.width() as usize + 1), "{axis:?}: {}", line.len());
            }
            // The stop's full pedal travel is still drawn.
            for line in [&throttle, &brake] {
                let has = |pedal: f32| line.iter().any(|p| p.y == view.scale.y(pedal));
                assert!(has(0.0) && has(1.0), "{axis:?}");
            }
        }
    }

    #[test]
    fn a_stopped_car_tessellates_like_a_moving_one() {
        let (live, now) = stopped_with_pedals_moving();
        let lap = sample_lap();
        let ctx = eframe::egui::Context::default();
        theme::install_fonts(&ctx);
        let panel = Rect::from_min_size(Pos2::ZERO, vec2(680.0, 170.0));
        let s = scene(Axis::Distance, &live, Some(&lap), now);
        let mut out = ctx.run_ui(Default::default(), |ui| paint(ui.painter(), panel, &s));
        for _ in 0..2 {
            out.textures_delta.clear();
            out = ctx.run_ui(Default::default(), |ui| paint(ui.painter(), panel, &s));
        }
        let vertices: usize = ctx
            .tessellate(out.shapes, out.pixels_per_point)
            .iter()
            .map(|p| match &p.primitive {
                eframe::egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                eframe::egui::epaint::Primitive::Callback(_) => 0,
            })
            .sum();
        // About 37k while driving; 1.8M before the stop was thinned.
        assert!(vertices < 60_000, "{vertices}");
    }

    /// The text rects of `message` painted on a `size` panel with the car mid-lap, and the
    /// plot and cursor.
    fn painted_message(size: Vec2, message: (&str, &str)) -> (Rect, f32, Vec<Rect>) {
        let ctx = eframe::egui::Context::default();
        theme::install_fonts(&ctx);
        let live = LiveTrace::new();
        let s = GraphScene { message: Some(message), ..scene(Axis::Distance, &live, None, car(0.5)) };
        let panel = Rect::from_min_size(pos2(4.0, 4.0), size);
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ui| paint(ui.painter(), panel, &s));
            out.textures_delta.clear();
            shapes = out.shapes;
        }
        let rects = shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Text(t) if [message.0, message.1].contains(&t.galley.text()) => {
                    Some(c.shape.visual_bounding_rect())
                }
                _ => None,
            })
            .collect();
        let plot = PlotLayout::new(panel, s.header_height, false).unwrap().plot;
        (plot, Scale::new(plot, s.behind, s.ahead).unwrap().x(0.0), rects)
    }

    #[test]
    fn empty_state_message_fits_the_plot() {
        let message = ("NO REFERENCE LAP", "Drop a Garage 61 CSV here, or load one in ⚙ settings");
        // Compact: the 172 pt look-ahead is too narrow, so the text centres across the plot.
        let (plot, _, rects) = painted_message(vec2(380.0, 170.0), message);
        assert_eq!(rects.len(), 2);
        for r in &rects {
            assert!(plot.contains_rect(*r), "{r:?} leaves {plot:?}");
            assert!((r.center().x - plot.center().x).abs() < 2.0, "{r:?}");
        }
        // Wide: in the look-ahead, clear of the cursor.
        let (plot, cursor, rects) = painted_message(vec2(680.0, 170.0), message);
        assert_eq!(rects.len(), 2);
        for r in &rects {
            assert!(plot.contains_rect(*r) && r.left() > cursor + 10.0, "{r:?}, cursor at {cursor}");
        }
    }

    #[test]
    fn broken_reference_laps_are_skipped() {
        let mut lap = sample_lap();
        let live = LiveTrace::new();
        let s = scene(Axis::Time, &live, None, car(0.5));
        lap.lap_time = 0.0;
        assert!(RefFrame::new(&lap, &s, &s.now.unwrap()).is_none());
        lap.brake.truncate(1);
        assert!(RefFrame::new(&lap, &s, &s.now.unwrap()).is_none());
    }
}
