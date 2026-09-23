//! The throttle/brake graph: a port of `prototype/src/graph.js` to egui's painter.

use std::sync::Arc;

use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Galley, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2,
    vec2,
};

use crate::lap::Lap;
use crate::settings::{Axis, LabelMode};
use crate::trace::{LiveSample, LiveTrace};
use crate::ui::graph_layout::{
    self as layout, Connector, Decimator, LabelPlacer, LabelSpot, Peak, PlotLayout, Scale, XBand, laps_in_view,
};
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
}

/// Start/finish hairline: white @ 22%.
const START_FINISH: Color32 = Color32::from_rgba_premultiplied(56, 56, 56, 56);
/// Car cursor: white @ 92%.
const CURSOR: Color32 = Color32::from_rgba_premultiplied(235, 235, 235, 235);
/// Reference-label connector: white @ 40%.
const REF_CONNECTOR: Color32 = Color32::from_rgba_premultiplied(102, 102, 102, 102);
/// Reference samples at or below this are on the floor: no edge line there.
const FLOOR: f32 = 0.004;
/// Extra window drawn past each side (fraction of the span) so lines run off the edges.
const OVERSCAN: f64 = 0.02;
/// The canvas' 6 px shadow blur (series colour @ 55%) under a live line, approximated
/// by two faint strokes (width, alpha of the series colour); each stroke is a full
/// tessellation of the line, so this is kept short.
const GLOW: [(f32, f32); 2] = [(12.0, 0.035), (6.0, 0.06)];
/// egui centres a text's whole row box, which puts Barlow a point below where the
/// canvas' `middle` baseline (used for all the prototype's text) puts it.
const MIDDLE_BASELINE: Vec2 = vec2(0.0, -1.0);

/// Paints the graph into `panel` (the overlay panel's inner rect; its background is
/// already painted). Draws nothing outside `panel`.
pub fn paint(painter: &Painter, panel: Rect, scene: &GraphScene) {
    let painter = painter.with_clip_rect(panel);
    let Some(PlotLayout { plot, compact, x_band }) = PlotLayout::new(panel, scene.header_height) else {
        return;
    };
    paint_grid(&painter, plot);
    let view = View::new(&painter, panel, plot, scene);
    if let Some(view) = &view {
        view.paint_data();
        let dots = view.paint_cursor();
        let mut obstacles: Vec<Rect> = scene.obstacles.iter().copied().chain(dots).collect();
        if x_band {
            obstacles.extend(view.paint_x_band(compact));
        }
        if scene.labels.show {
            view.paint_peak_labels(obstacles);
        }
    }
    if let Some(message) = scene.message {
        let cursor_x = view.map(|v| v.scale.x(0.0)).filter(|_| scene.ahead > 0.0);
        paint_message(&painter, plot, cursor_x, message);
    }
}

/// Hairlines at 100, 50 and 0 % with their labels in the left gutter.
fn paint_grid(painter: &Painter, plot: Rect) {
    let y = |p: f32| plot.top() + (1.0 - p) * plot.height();
    for (p, color) in [(1.0, theme::GRID), (0.5, theme::GRID), (0.0, theme::BASELINE)] {
        painter.hline(plot.x_range(), painter.round_to_pixel_center(y(p)), Stroke::new(1.0, color));
    }
    let labels: &[(f32, &str)] =
        if plot.height() >= 56.0 { &[(1.0, "100"), (0.5, "50"), (0.0, "0")] } else { &[(1.0, "100"), (0.0, "0")] };
    for &(p, text) in labels {
        let pos = pos2(plot.left() - 6.0, y(p) + 0.5);
        paint_text(painter, pos, Align2::RIGHT_CENTER, text, theme::font(Weight::SemiBold, 10.0), theme::HUD_MUTED);
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

fn paint_text(painter: &Painter, pos: Pos2, align: Align2, text: impl ToString, font: FontId, color: Color32) {
    let galley = painter.layout_no_wrap(text.to_string(), font, color);
    paint_galley(painter, pos, align, galley, color);
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

    /// Start/finish line, the reference fills and the live lines.
    fn paint_data(&self) {
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
        let (throttle, brake) = self.live_lines(lo);
        paint_live_line(&painter, throttle, theme::THROTTLE);
        paint_live_line(&painter, brake, theme::BRAKE);
    }

    /// A reference series as a vertical-gradient area plus an edge line that lifts off
    /// along zero stretches, so the baseline stays clean. Thinned to one physical pixel
    /// per column first, like the live lines: a lap has more samples than the plot has
    /// pixels.
    fn paint_area(&self, painter: &Painter, samples: &[(f64, usize)], values: &[f32], color: Color32, opacity: f32) {
        let mut thin = Decimator::new(1.0 / self.painter.pixels_per_point());
        for &(v, i) in samples {
            thin.push(pos2(self.scale.x(v), self.scale.y(pedal(values[i]))));
        }
        let points = thin.finish();
        if points.len() < 2 {
            return;
        }
        let top = theme::alpha(color, 0.36 * opacity);
        let bottom = theme::alpha(color, 0.02 * opacity);
        let edge = Stroke::new(1.25, theme::alpha(color, 0.6 * opacity));
        let base = self.plot.bottom();
        let value_at = |p: Pos2| ((base - p.y) / self.plot.height()).clamp(0.0, 1.0);
        let lifted = |j: Option<usize>| j.and_then(|j| points.get(j)).is_some_and(|&p| value_at(p) > FLOOR);

        let mut mesh = Mesh::default();
        mesh.reserve_vertices(2 * points.len());
        mesh.reserve_triangles(2 * (points.len() - 1));
        let mut edges = Vec::new();
        let mut run = Vec::new();
        for (j, &p) in points.iter().enumerate() {
            let value = value_at(p);
            // Colour by height: the gradient runs from the plot top to the baseline.
            let n = mesh.vertices.len() as u32;
            mesh.colored_vertex(p, bottom.lerp_to_gamma(top, value));
            mesh.colored_vertex(pos2(p.x, base), bottom);
            if n > 0 {
                mesh.add_triangle(n - 2, n - 1, n);
                mesh.add_triangle(n - 1, n + 1, n);
            }
            if value > FLOOR || lifted(j.checked_sub(1)) || lifted(Some(j + 1)) {
                run.push(p);
            } else {
                flush_run(&mut run, &mut edges, edge);
            }
        }
        flush_run(&mut run, &mut edges, edge);
        painter.add(mesh);
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

    /// Cursor, playhead and current-value dots. Returns the dots' boxes.
    fn paint_cursor(&self) -> [Rect; 2] {
        let (x, top) = (self.scale.x(0.0), self.plot.top());
        self.painter.line_segment([pos2(x, top - 2.0), pos2(x, self.plot.bottom())], Stroke::new(1.5, CURSOR));
        let playhead = vec![pos2(x - 4.0, top - 7.0), pos2(x + 4.0, top - 7.0), pos2(x, top - 2.0)];
        self.painter.add(Shape::convex_polygon(playhead, Color32::WHITE, Stroke::NONE));
        [(self.now.throttle, theme::THROTTLE), (self.now.brake, theme::BRAKE)].map(|(value, color)| {
            let at = pos2(x, self.scale.y(value));
            paint_dot(self.painter, at, color, false);
            Rect::from_center_size(at, Vec2::splat(12.0))
        })
    }

    /// The car's lap-distance pill and the axis ticks that fit around it. Returns what
    /// was placed, for the peak labels to avoid.
    fn paint_x_band(&self, compact: bool) -> Vec<Rect> {
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
            let color = if tick.strong { theme::HUD_TEXT } else { theme::HUD_MUTED };
            let galley = painter.layout_no_wrap(tick.text, theme::font(Weight::SemiBold, 10.0), color);
            let x = self.scale.x(tick.v);
            if band.place_tick(x, galley.size().x).is_some() {
                paint_galley(painter, pos2(x, band.top + 6.5), Align2::CENTER_CENTER, galley, color);
            }
        }
        band.into_taken()
    }

    /// Brake peaks the labels settings ask for: live events in the history window and
    /// reference zones on every lap copy in view.
    fn peaks(&self) -> Vec<Peak> {
        let LabelOptions { mode, min, .. } = self.scene.labels;
        let (behind, ahead) = (self.scale.behind(), self.scale.ahead());
        let mut peaks = Vec::new();
        if mode != LabelMode::Reference {
            for e in self.scene.live.events().filter(|e| e.peak >= min) {
                let v = if self.by_time() { e.peak_t } else { e.peak_d } - self.key;
                if (-behind..=0.0).contains(&v) {
                    peaks.push(Peak { live: true, v, value: e.peak });
                }
            }
        }
        if let Some(rf) = self.reference.as_ref().filter(|_| mode != LabelMode::Live) {
            for z in rf.lap.zones.iter().filter(|z| z.peak >= min && z.peak_idx < rf.n) {
                for base in rf.lap_offsets(-behind, ahead) {
                    let v = base + rf.at(z.peak_idx);
                    if (-behind..=ahead).contains(&v) {
                        peaks.push(Peak { live: false, v, value: z.peak });
                    }
                }
            }
        }
        layout::sort_for_placement(&mut peaks);
        peaks
    }

    /// Brake-peak labels, skipped where they'd collide. Connectors and dots go under all
    /// pills.
    fn paint_peak_labels(&self, obstacles: Vec<Rect>) {
        let painter = self.painter;
        let mut placer = LabelPlacer::new(self.plot, self.panel.top() + 2.0, obstacles);
        let placed: Vec<(bool, Rect, LabelSpot, Pos2, Arc<Galley>)> = self
            .peaks()
            .into_iter()
            .filter_map(|peak| {
                let (weight, size, color) = if peak.live {
                    (Weight::Bold, 11.0, Color32::WHITE)
                } else {
                    (Weight::SemiBold, 10.5, theme::HUD_TEXT)
                };
                let galley = painter.layout_no_wrap(peak.text(), theme::font(weight, size), color);
                let at = pos2(self.scale.x(peak.v), self.scale.y(pedal(peak.value)));
                let (rect, spot) = placer.place(at, galley.size().x.ceil() + 10.0)?;
                Some((peak.live, rect, spot, at, galley))
            })
            .collect();

        for &(live, rect, spot, at, _) in &placed {
            match layout::connector(rect, spot, at, live) {
                Connector::Pointer(points) => {
                    painter.add(Shape::convex_polygon(points.to_vec(), theme::BRAKE_PILL, Stroke::NONE));
                }
                Connector::Line(from, to) => {
                    let color = if live { theme::BRAKE_PILL } else { REF_CONNECTOR };
                    painter.line_segment([from, to], Stroke::new(1.0, color));
                }
            }
            paint_dot(painter, at, theme::BRAKE, !live);
        }
        for (live, rect, _, _, galley) in placed {
            let pill = rect.shrink(0.5);
            if live {
                painter.rect_filled(pill, CornerRadius::same(4), theme::BRAKE_PILL);
            } else {
                let stroke = Stroke::new(1.0, theme::alpha(theme::BRAKE, 0.9));
                painter.rect(
                    pill,
                    CornerRadius::same(4),
                    theme::alpha(theme::SURFACE, 0.9),
                    stroke,
                    StrokeKind::Middle,
                );
            }
            let color = if live { Color32::WHITE } else { theme::HUD_TEXT };
            paint_galley(painter, rect.center() + vec2(0.0, 0.5), Align2::CENTER_CENTER, galley, color);
        }
    }
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

/// A live series: dark underlay (keeps it legible over its own fill), soft glow, line.
fn paint_live_line(painter: &Painter, points: Vec<Pos2>, color: Color32) {
    if points.len() < 2 {
        return;
    }
    painter.line(points.clone(), Stroke::new(4.0, theme::alpha(theme::SURFACE, 0.55)));
    for (width, alpha) in GLOW {
        painter.line(points.clone(), Stroke::new(width, theme::alpha(color, alpha)));
    }
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
        let plot = PlotLayout::new(panel, 26.0).unwrap().plot;
        let peaks = View::new(&painter, panel, plot, &s).unwrap().peaks();
        // Live first (the active 8% event is under the 10% minimum), then reference zones
        // nearest first: 77% and 67% on this lap, 68% and 46% on the next.
        assert!(peaks[0].live && peaks[0].value == 0.7 && (peaks[0].v + 80.0).abs() < 1e-6);
        let refs: Vec<u32> = peaks[1..].iter().map(|p| (p.value * 100.0).round() as u32).collect();
        assert_eq!(refs, [77, 67, 68, 46]);
        assert!(peaks[1..].iter().all(|p| !p.live));
        assert!((peaks[3].v - (2.0 * L + lap.pct[lap.zones[0].peak_idx] * L - now.lap_pos * L)).abs() < 1e-6);

        s.labels.mode = LabelMode::Live;
        assert!(View::new(&painter, panel, plot, &s).unwrap().peaks().iter().all(|p| p.live));
        s.labels.mode = LabelMode::Reference;
        assert!(View::new(&painter, panel, plot, &s).unwrap().peaks().iter().all(|p| !p.live));
    }

    #[test]
    fn nothing_to_view_without_a_usable_car_or_window() {
        let live = LiveTrace::new();
        let painter =
            Painter::new(eframe::egui::Context::default(), eframe::egui::LayerId::background(), Rect::EVERYTHING);
        let panel = Rect::from_min_size(Pos2::ZERO, vec2(680.0, 170.0));
        let plot = PlotLayout::new(panel, 26.0).unwrap().plot;
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
        let plot = PlotLayout::new(panel, 26.0).unwrap().plot;
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
        let plot = PlotLayout::new(panel, s.header_height).unwrap().plot;
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
