//! Pure layout helpers for the graph (axis ticks, label placement), kept free of
//! egui drawing so they can be unit-tested.
//!
//! All positions are painter coordinates (points). The x axis is the offset from the
//! car in the axis' units (metres or seconds): negative behind, positive ahead.

use std::ops::RangeInclusive;

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

/// Distance-axis tick steps, metres.
const DIST_STEPS: [f64; 8] = [25.0, 50.0, 100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0];
/// Time-axis tick steps, seconds.
const TIME_STEPS: [f64; 5] = [0.5, 1.0, 2.0, 5.0, 10.0];
/// More lap copies than this in view means a nonsensical lap length: none are drawn.
const MAX_LAPS_IN_VIEW: f64 = 16.0;
/// More ticks than this means a nonsensical window: none are made.
const MAX_TICKS: f64 = 200.0;
/// Brake-peak label height.
pub const LABEL_HEIGHT: f32 = 16.0;
/// Lap-distance pill height.
pub const PILL_HEIGHT: f32 = 14.0;
/// X-band tick label height.
pub const TICK_HEIGHT: f32 = 12.0;
/// Room kept between the empty-state message and the car cursor or the plot's edge.
const MESSAGE_PAD: f32 = 12.0;

/// The plot inside the panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlotLayout {
    /// Pedal 0..1 bottom to top; the window from behind to ahead of the car left to right.
    pub plot: Rect,
    /// Narrow panel: slimmer gutter, closer ticks.
    pub compact: bool,
    /// Tall enough for the x band (lap-distance pill and tick labels) under the plot.
    pub x_band: bool,
}

impl PlotLayout {
    /// `None` when the panel is too small to plot anything.
    pub fn new(panel: Rect, header_height: f32) -> Option<Self> {
        let compact = panel.width() < 400.0;
        let x_band = panel.height() >= 118.0;
        let plot = Rect::from_min_max(
            pos2(panel.left() + if compact { 26.0 } else { 32.0 }, panel.top() + header_height + 8.0),
            pos2(panel.right() - 10.0, panel.bottom() - if x_band { 20.0 } else { 8.0 }),
        );
        (plot.width() >= 40.0 && plot.height() >= 16.0).then_some(Self { plot, compact, x_band })
    }
}

/// Maps axis offsets (x) and pedal travel (y, 0..1) into the plot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    plot: Rect,
    behind: f64,
    span: f64,
}

impl Scale {
    /// `None` unless `behind` and `ahead` are finite, non-negative and not both zero.
    pub fn new(plot: Rect, behind: f64, ahead: f64) -> Option<Self> {
        let span = behind + ahead;
        (behind >= 0.0 && ahead >= 0.0 && span > 0.0 && span.is_finite()).then_some(Self { plot, behind, span })
    }

    pub fn x(&self, v: f64) -> f32 {
        self.plot.left() + ((v + self.behind) / self.span) as f32 * self.plot.width()
    }

    pub fn y(&self, pedal: f32) -> f32 {
        self.plot.top() + (1.0 - pedal) * self.plot.height()
    }

    /// Visible span behind the car.
    pub fn behind(&self) -> f64 {
        self.behind
    }

    /// Visible span ahead of the car.
    pub fn ahead(&self) -> f64 {
        self.span - self.behind
    }

    /// Points per axis unit.
    pub fn px_per_unit(&self) -> f64 {
        f64::from(self.plot.width()) / self.span
    }
}

/// `v` limited to `lo..=hi`; `lo` wins when the range is empty (unlike `clamp`, never panics).
pub fn fit(v: f32, lo: f32, hi: f32) -> f32 {
    v.min(hi).max(lo)
}

/// Whether `a` and `b` come closer than `margin` (boxes exactly `margin` apart don't).
pub fn overlaps(a: Rect, b: Rect, margin: f32) -> bool {
    a.min.x < b.max.x + margin && b.min.x < a.max.x + margin && a.min.y < b.max.y + margin && b.min.y < a.max.y + margin
}

/// Thins a polyline to screen resolution, for lines with more points than pixels (a slow
/// or stopped car). Points less than `column` to either side of the first point of their
/// run share a column, of which only the first, highest, lowest and last are kept, in
/// order. The line looks the same, with at most four points per column however many it
/// was given.
#[derive(Debug, Clone)]
pub struct Decimator {
    column: f32,
    out: Vec<Pos2>,
    run: Option<Column>,
}

/// The points of one column that are kept, with their order in it.
#[derive(Debug, Clone, Copy)]
struct Column {
    first: Pos2,
    top: (usize, Pos2),
    bottom: (usize, Pos2),
    last: (usize, Pos2),
    len: usize,
}

impl Decimator {
    /// `column`: the width (points) that counts as one pixel column.
    pub fn new(column: f32) -> Self {
        Self { column, out: Vec::new(), run: None }
    }

    pub fn push(&mut self, p: Pos2) {
        match &mut self.run {
            Some(c) if (p.x - c.first.x).abs() < self.column => {
                if p.y < c.top.1.y {
                    c.top = (c.len, p);
                }
                if p.y > c.bottom.1.y {
                    c.bottom = (c.len, p);
                }
                c.last = (c.len, p);
                c.len += 1;
            }
            _ => {
                self.flush();
                self.run = Some(Column { first: p, top: (0, p), bottom: (0, p), last: (0, p), len: 1 });
            }
        }
    }

    /// The thinned line.
    pub fn finish(mut self) -> Vec<Pos2> {
        self.flush();
        self.out
    }

    /// Emits the current column; repeated points are dropped (the tessellator needs distinct ones).
    fn flush(&mut self) {
        let Some(c) = self.run.take() else { return };
        let mut kept = [(0, c.first), c.top, c.bottom, c.last];
        kept.sort_by_key(|&(i, _)| i);
        for (_, p) in kept {
            if self.out.last() != Some(&p) {
                self.out.push(p);
            }
        }
    }
}

/// Where the two-line empty-state message is centred: in the look-ahead right of the car
/// (`cursor_x`) when text `text_width` wide fits there with room to spare, else across the
/// plot. Always inside the plot (from its left edge when the text is wider).
pub fn message_center_x(plot: Rect, cursor_x: Option<f32>, text_width: f32) -> f32 {
    let need = text_width + 2.0 * MESSAGE_PAD;
    let half = text_width / 2.0;
    let (lo, hi) = match cursor_x {
        // Clear of the car: the look-ahead first, then the history behind it.
        Some(x) if plot.right() - x >= need => (x, plot.right()),
        Some(x) if x - plot.left() >= need => (plot.left(), x),
        _ => (plot.left(), plot.right()),
    };
    fit((lo + hi) / 2.0, plot.left() + half, plot.right() - half)
}

/// Laps `k` whose copy `[k·period, (k+1)·period)` overlaps `[lo, hi]` (absolute axis
/// positions). `None` when the period is unusable or implausibly many laps fit.
pub fn laps_in_view(lo: f64, hi: f64, period: f64) -> Option<RangeInclusive<i64>> {
    let (first, last) = ((lo / period).floor(), (hi / period).floor());
    let plausible = period > 0.0 && first.is_finite() && last.is_finite() && first <= last;
    (plausible && last - first < MAX_LAPS_IN_VIEW).then_some(first as i64..=last as i64)
}

/// Start/finish crossings in view on the distance axis, as offsets from the car at `now_d`.
pub fn start_finish_offsets(now_d: f64, scale: &Scale, track_length: f64) -> impl Iterator<Item = f64> {
    let (behind, ahead) = (scale.behind(), scale.ahead());
    laps_in_view(now_d - behind, now_d + ahead, track_length)
        .into_iter()
        .flatten()
        .map(move |k| k as f64 * track_length - now_d)
        .filter(move |&v| v >= -behind && v <= ahead)
}

/// An x-axis label.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    /// Offset from the car.
    pub v: f64,
    pub text: String,
    /// Start/finish: drawn brighter.
    pub strong: bool,
}

/// The first step that puts ticks at least `min_px` apart (the largest if none does).
fn tick_step(steps: &[f64], px_per_unit: f64, min_px: f64) -> f64 {
    steps.iter().copied().find(|s| s * px_per_unit >= min_px).unwrap_or(steps[steps.len() - 1])
}

/// Distance ticks: "S/F" at each start/finish crossing first, then round lap distances
/// ("3400m") at least 90 pt apart (70 when compact). `now_d` is the car's cumulative metres.
pub fn distance_ticks(now_d: f64, scale: &Scale, track_length: f64, compact: bool) -> Vec<Tick> {
    let mut ticks: Vec<Tick> = start_finish_offsets(now_d, scale, track_length)
        .map(|v| Tick { v, text: "S/F".into(), strong: true })
        .collect();
    let step = tick_step(&DIST_STEPS, scale.px_per_unit(), if compact { 70.0 } else { 90.0 });
    let (lo, hi) = (now_d - scale.behind(), now_d + scale.ahead());
    if (hi - lo) / step > MAX_TICKS {
        return ticks;
    }
    for k in laps_in_view(lo, hi, track_length).into_iter().flatten() {
        let lap_start = k as f64 * track_length;
        // Multiples of the step strictly inside the lap: 0 is the S/F tick.
        let first = ((lo - lap_start) / step).ceil().max(1.0) as i64;
        let last = ((hi - lap_start) / step).floor() as i64;
        for d in (first..=last).map(|j| j as f64 * step).take_while(|&d| d < track_length) {
            ticks.push(Tick { v: lap_start + d - now_d, text: format!("{d}m"), strong: false });
        }
    }
    ticks
}

/// Time ticks ("−4s", "+2s") at least 70 pt apart (56 when compact); none at the car.
pub fn time_ticks(scale: &Scale, compact: bool) -> Vec<Tick> {
    let step = tick_step(&TIME_STEPS, scale.px_per_unit(), if compact { 56.0 } else { 70.0 });
    if (scale.behind() + scale.ahead()) / step > MAX_TICKS {
        return Vec::new();
    }
    let first = (-scale.behind() / step).ceil() as i64;
    let last = ((scale.ahead() + 1e-9) / step).floor() as i64;
    (first..=last)
        .filter(|&j| j != 0)
        .map(|j| {
            let v = j as f64 * step;
            Tick { v, text: time_label(v), strong: false }
        })
        .collect()
}

/// `+2s`, `−0.5s` (with a typographic minus).
pub fn time_label(v: f64) -> String {
    let secs = (v.abs() * 10.0).round() / 10.0;
    format!("{}{secs}s", if v > 0.0 { '+' } else { '\u{2212}' })
}

/// The strip under the plot: the car's lap-distance pill, then the tick labels that fit
/// around it.
#[derive(Debug, Clone)]
pub struct XBand {
    /// Top of the tick labels.
    pub top: f32,
    plot_left: f32,
    panel_right: f32,
    taken: Vec<Rect>,
}

impl XBand {
    pub fn new(plot: Rect, panel: Rect) -> Self {
        Self { top: plot.bottom() + 4.0, plot_left: plot.left(), panel_right: panel.right(), taken: Vec::new() }
    }

    /// Places a `width`-wide pill centred under the car at `x`, kept inside the panel.
    pub fn place_pill(&mut self, x: f32, width: f32) -> Rect {
        let left = fit(x - width / 2.0, self.plot_left - 4.0, self.panel_right - 2.0 - width);
        let pill = Rect::from_min_size(pos2(left, self.top - 1.0), vec2(width, PILL_HEIGHT));
        self.taken.push(pill);
        pill
    }

    /// Places a tick label of text width `text_width` centred on `x`. `None` when it would
    /// leave the band or come within 3 pt of the pill or an earlier tick.
    pub fn place_tick(&mut self, x: f32, text_width: f32) -> Option<Rect> {
        let tick = Rect::from_min_size(pos2(x - text_width / 2.0 - 2.0, self.top), vec2(text_width + 4.0, TICK_HEIGHT));
        let fits = tick.left() >= self.plot_left - 6.0
            && tick.right() <= self.panel_right - 2.0
            && !self.taken.iter().any(|t| overlaps(*t, tick, 3.0));
        fits.then(|| {
            self.taken.push(tick);
            tick
        })
    }

    /// The pill and ticks placed so far; peak labels keep clear of them.
    pub fn into_taken(self) -> Vec<Rect> {
        self.taken
    }
}

/// A brake peak that wants a label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peak {
    /// The driver's own peak (solid pill), not the reference's (outlined pill).
    pub live: bool,
    /// Offset from the car.
    pub v: f64,
    /// Pedal, 0..1.
    pub value: f32,
}

impl Peak {
    /// `68%`.
    pub fn text(&self) -> String {
        format!("{}%", (self.value * 100.0).round() as i32)
    }
}

/// Placement order: live peaks win collisions, then the ones nearest the car.
pub fn sort_for_placement(peaks: &mut [Peak]) {
    peaks.sort_by(|a, b| b.live.cmp(&a.live).then(a.v.abs().total_cmp(&b.v.abs())));
}

/// Where a label sits relative to its peak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelSpot {
    Above,
    /// Above, one label higher (clear of a neighbour's label).
    Stacked,
    Right,
    Left,
    Below,
}

/// Places brake-peak labels one at a time, each clear of everything placed before it.
#[derive(Debug, Clone)]
pub struct LabelPlacer {
    plot: Rect,
    top: f32,
    taken: Vec<Rect>,
}

impl LabelPlacer {
    /// Labels stay inside the plot horizontally and between `top` and the plot's bottom
    /// vertically: they may rise into the header strip, where `obstacles` (header items,
    /// the x band, the cursor dots) keep them off what's drawn there.
    pub fn new(plot: Rect, top: f32, obstacles: impl IntoIterator<Item = Rect>) -> Self {
        Self { plot, top, taken: obstacles.into_iter().collect() }
    }

    /// The first free spot for a `width`-wide label on the peak at `at`: above, stacked
    /// above, right, left, then below. Keeps clear (2 pt) of everything placed, including
    /// earlier peaks' dots, and off its own dot. `None` when nothing fits: skip the label.
    pub fn place(&mut self, at: Pos2, width: f32) -> Option<(Rect, LabelSpot)> {
        let h = LABEL_HEIGHT;
        let (x_min, x_max) = (self.plot.left(), self.plot.right() - width);
        let centred = fit(at.x - width / 2.0, x_min, x_max);
        let dot = Rect::from_center_size(at, Vec2::splat(10.0));
        let candidates = [
            (LabelSpot::Above, pos2(centred, at.y - 7.0 - h)),
            (LabelSpot::Stacked, pos2(centred, at.y - 10.0 - 2.0 * h)),
            (LabelSpot::Right, pos2(at.x + 8.0, at.y - h / 2.0)),
            (LabelSpot::Left, pos2(at.x - 8.0 - width, at.y - h / 2.0)),
            (LabelSpot::Below, pos2(centred, at.y + 7.0)),
        ];
        let (label, spot) = candidates
            .into_iter()
            .map(|(spot, min)| (Rect::from_min_size(min, vec2(width, h)), spot))
            .find(|&(r, _)| {
                (x_min - 0.5..=x_max + 0.5).contains(&r.left())
                    && r.top() >= self.top
                    && r.bottom() <= self.plot.bottom() - 1.0
                    && !overlaps(dot, r, 0.0)
                    && !self.taken.iter().any(|t| overlaps(*t, r, 2.0))
            })?;
        self.taken.extend([label, dot]);
        Some((label, spot))
    }
}

/// How a label points at its peak.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Connector {
    /// A pointer under the pill, straight above the peak.
    Pointer([Pos2; 3]),
    /// A hairline from the pill's edge to the peak.
    Line(Pos2, Pos2),
}

/// The connector for a label placed at `spot`; `pointer` allows the pointer (live labels).
pub fn connector(label: Rect, spot: LabelSpot, at: Pos2, pointer: bool) -> Connector {
    let ax = fit(at.x, label.left() + 4.0, label.right() - 4.0);
    if pointer && spot == LabelSpot::Above && ax == at.x {
        let y = label.bottom() - 0.5;
        return Connector::Pointer([pos2(at.x - 4.0, y), pos2(at.x + 4.0, y), pos2(at.x, label.bottom() + 4.0)]);
    }
    let from = match spot {
        LabelSpot::Right => pos2(label.left(), label.center().y),
        LabelSpot::Left => pos2(label.right(), label.center().y),
        _ => pos2(ax, if label.bottom() <= at.y { label.bottom() } else { label.top() }),
    };
    Connector::Line(from, at)
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: f64 = 5796.1;

    fn panel(w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(10.0, 10.0), vec2(w, h))
    }

    fn layout(w: f32, h: f32) -> PlotLayout {
        PlotLayout::new(panel(w, h), 26.0).unwrap()
    }

    /// The prototype's defaults: 500 m behind and ahead.
    fn metres(w: f32, h: f32) -> Scale {
        Scale::new(layout(w, h).plot, 500.0, 500.0).unwrap()
    }

    fn texts(ticks: &[Tick]) -> Vec<&str> {
        ticks.iter().map(|t| t.text.as_str()).collect()
    }

    #[test]
    fn plot_insets_follow_panel_size() {
        let l = layout(680.0, 170.0);
        assert_eq!(l.plot, Rect::from_min_max(pos2(42.0, 44.0), pos2(680.0, 160.0)));
        assert!(!l.compact && l.x_band);

        let l = layout(300.0, 190.0);
        assert_eq!(l.plot.left(), 36.0);
        assert!(l.compact && l.x_band);

        let l = layout(680.0, 100.0);
        assert_eq!(l.plot.bottom(), 102.0);
        assert!(!l.x_band);

        assert!(PlotLayout::new(panel(70.0, 170.0), 26.0).is_none());
        assert!(PlotLayout::new(panel(680.0, 50.0), 26.0).is_none());
    }

    #[test]
    fn scale_maps_window_and_pedal() {
        let s = metres(680.0, 170.0);
        assert_eq!(s.x(-500.0), 42.0);
        assert_eq!(s.x(0.0), 361.0);
        assert_eq!(s.x(500.0), 680.0);
        assert_eq!((s.y(1.0), s.y(0.0)), (44.0, 160.0));
        assert_eq!((s.behind(), s.ahead()), (500.0, 500.0));

        let plot = layout(680.0, 170.0).plot;
        assert!(Scale::new(plot, 0.0, 0.0).is_none());
        assert!(Scale::new(plot, f64::NAN, 10.0).is_none());
        assert!(Scale::new(plot, -1.0, 10.0).is_none());
        assert!(Scale::new(plot, 8.0, 0.0).is_some());
    }

    #[test]
    fn overlap_margin_is_exclusive() {
        let a = Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 10.0));
        let b = Rect::from_min_size(pos2(13.0, 0.0), vec2(10.0, 10.0));
        assert!(overlaps(a, b, 3.1));
        assert!(!overlaps(a, b, 3.0));
        assert!(!overlaps(a, b.translate(vec2(0.0, 20.0)), 3.1));
    }

    #[test]
    fn fit_never_panics_on_an_empty_range() {
        assert_eq!(fit(5.0, 0.0, 10.0), 5.0);
        assert_eq!(fit(-5.0, 0.0, 10.0), 0.0);
        assert_eq!(fit(50.0, 0.0, 10.0), 10.0);
        assert_eq!(fit(5.0, 10.0, 0.0), 10.0);
    }

    #[test]
    fn laps_in_view_spans_start_finish_and_rejects_nonsense() {
        assert_eq!(laps_in_view(3300.0, 4300.0, L), Some(0..=0));
        assert_eq!(laps_in_view(L - 200.0, L + 300.0, L), Some(0..=1));
        assert_eq!(laps_in_view(-100.0, 400.0, L), Some(-1..=0));
        assert_eq!(laps_in_view(0.0, 1000.0, 0.0), None);
        assert_eq!(laps_in_view(0.0, 1000.0, f64::NAN), None);
        assert_eq!(laps_in_view(0.0, 1000.0, 1.0), None);
    }

    #[test]
    fn distance_step_keeps_ticks_apart() {
        // 638 pt for 1000 m: 200 m is the first step at least 90 pt wide.
        assert_eq!(tick_step(&DIST_STEPS, metres(680.0, 170.0).px_per_unit(), 90.0), 200.0);
        // 264 pt compact: 250 m is 66 pt, under 70.
        assert_eq!(tick_step(&DIST_STEPS, metres(300.0, 190.0).px_per_unit(), 70.0), 500.0);
        assert_eq!(tick_step(&DIST_STEPS, 0.001, 90.0), 2000.0);
    }

    #[test]
    fn distance_ticks_mid_lap() {
        let ticks = distance_ticks(3800.0, &metres(680.0, 170.0), L, false);
        assert_eq!(texts(&ticks), ["3400m", "3600m", "3800m", "4000m", "4200m"]);
        assert!(ticks.iter().all(|t| !t.strong));
        assert!((ticks[0].v + 400.0).abs() < 1e-9);

        let ticks = distance_ticks(3800.0, &metres(300.0, 190.0), L, true);
        assert_eq!(texts(&ticks), ["3500m", "4000m"]);
    }

    #[test]
    fn distance_ticks_wrap_across_start_finish() {
        // 150 m into lap 2: the window runs from 5446 m on lap 1 to 650 m on lap 2.
        let now = L + 150.0;
        let ticks = distance_ticks(now, &metres(680.0, 170.0), L, false);
        assert_eq!(texts(&ticks), ["S/F", "5600m", "200m", "400m", "600m"]);
        assert!(ticks[0].strong && (ticks[0].v + 150.0).abs() < 1e-9);
        assert!((ticks[1].v - (5600.0 - L - 150.0)).abs() < 1e-9);
        assert!((ticks[2].v - 50.0).abs() < 1e-9);
        // No round-distance tick doubles the S/F one ("0m", "5796m").
        assert!(ticks[1..].iter().all(|t| !t.strong && (t.v - ticks[0].v).abs() > 1.0));
    }

    #[test]
    fn start_finish_offsets_in_view() {
        let s = metres(680.0, 170.0);
        let sf = |now: f64| start_finish_offsets(now, &s, L).collect::<Vec<_>>();
        assert!(matches!(sf(L - 499.0)[..], [v] if (v - 499.0).abs() < 1e-6));
        assert!(matches!(sf(L + 499.0)[..], [v] if (v + 499.0).abs() < 1e-6));
        assert!(matches!(sf(2.0 * L + 10.0)[..], [v] if (v + 10.0).abs() < 1e-6));
        assert!(sf(3800.0).is_empty());
        assert_eq!(start_finish_offsets(3800.0, &s, 0.0).count(), 0);
    }

    #[test]
    fn time_ticks_follow_spacing() {
        let plot = layout(680.0, 170.0).plot;
        let s = Scale::new(plot, 8.0, 6.0).unwrap();
        // 638 pt for 14 s: 45.6 pt/s, so 2 s steps.
        let ticks = time_ticks(&s, false);
        assert_eq!(texts(&ticks), ["\u{2212}8s", "\u{2212}6s", "\u{2212}4s", "\u{2212}2s", "+2s", "+4s", "+6s"]);

        let plot = layout(300.0, 190.0).plot;
        let ticks = time_ticks(&Scale::new(plot, 8.0, 6.0).unwrap(), true);
        assert_eq!(texts(&ticks), ["\u{2212}5s", "+5s"]);

        let ticks = time_ticks(&Scale::new(layout(1400.0, 170.0).plot, 1.0, 0.0).unwrap(), false);
        assert_eq!(texts(&ticks), ["\u{2212}1s", "\u{2212}0.5s"]);
    }

    #[test]
    fn time_labels() {
        assert_eq!(time_label(2.0), "+2s");
        assert_eq!(time_label(-0.5), "\u{2212}0.5s");
        assert_eq!(time_label(1.5000001), "+1.5s");
    }

    #[test]
    fn x_band_places_pill_then_ticks_that_fit() {
        let (p, l) = (panel(680.0, 170.0), layout(680.0, 170.0));
        let s = metres(680.0, 170.0);
        let mut band = XBand::new(l.plot, p);
        let pill = band.place_pill(s.x(0.0), 34.0);
        assert_eq!(pill.center().x, s.x(0.0));
        assert_eq!((pill.top(), pill.height()), (l.plot.bottom() + 3.0, PILL_HEIGHT));
        // The 3800m tick lands on the pill; its neighbours fit.
        assert!(band.place_tick(s.x(0.0), 26.0).is_none());
        assert!(band.place_tick(s.x(-200.0), 26.0).is_some());
        assert!(band.place_tick(s.x(200.0), 26.0).is_some());
        // Too close to a placed tick, and past either end of the band.
        assert!(band.place_tick(s.x(-180.0), 26.0).is_none());
        assert!(band.place_tick(s.x(-500.0), 26.0).is_none());
        assert!(band.place_tick(s.x(500.0), 26.0).is_none());
        assert_eq!(band.into_taken().len(), 3);
    }

    #[test]
    fn x_band_pill_stays_inside_panel() {
        let (p, l) = (panel(300.0, 190.0), layout(300.0, 190.0));
        let mut band = XBand::new(l.plot, p);
        assert_eq!(band.place_pill(l.plot.left(), 34.0).left(), l.plot.left() - 4.0);
        assert_eq!(band.place_pill(l.plot.right(), 34.0).right(), p.right() - 2.0);
    }

    #[test]
    fn start_finish_wins_over_a_close_distance_tick() {
        // 398 pt for 1000 m: 250 m steps, so 5750m sits 46 m (18 pt) before S/F.
        let (p, l) = (panel(440.0, 170.0), layout(440.0, 170.0));
        let s = metres(440.0, 170.0);
        let ticks = distance_ticks(L - 300.0, &s, L, false);
        assert_eq!(texts(&ticks), ["S/F", "5000m", "5250m", "5500m", "5750m"]);
        let mut band = XBand::new(l.plot, p);
        band.place_pill(s.x(0.0), 34.0);
        let placed: Vec<&str> = ticks
            .iter()
            .filter(|t| band.place_tick(s.x(t.v), 6.0 * t.text.len() as f32).is_some())
            .map(|t| t.text.as_str())
            .collect();
        // 5000m hangs off the left end and 5500m is under the pill.
        assert_eq!(placed, ["S/F", "5250m"]);
    }

    #[test]
    fn peaks_sort_live_first_then_nearest() {
        let p = |live, v| Peak { live, v, value: 0.5 };
        let mut peaks = [p(false, 10.0), p(true, -300.0), p(false, -5.0), p(true, -20.0)];
        sort_for_placement(&mut peaks);
        assert_eq!(peaks, [p(true, -20.0), p(true, -300.0), p(false, -5.0), p(false, 10.0)]);
        assert_eq!(Peak { live: true, v: 0.0, value: 0.681 }.text(), "68%");
        assert_eq!(Peak { live: true, v: 0.0, value: 1.0 }.text(), "100%");
    }

    #[test]
    fn label_goes_above_its_peak_when_free() {
        let l = layout(680.0, 170.0);
        let mut placer = LabelPlacer::new(l.plot, 12.0, []);
        let at = pos2(300.0, 120.0);
        let (r, spot) = placer.place(at, 30.0).unwrap();
        assert_eq!(spot, LabelSpot::Above);
        assert_eq!(r, Rect::from_min_size(pos2(285.0, 97.0), vec2(30.0, LABEL_HEIGHT)));
        assert_eq!(
            connector(r, spot, at, true),
            Connector::Pointer([pos2(296.0, 112.5), pos2(304.0, 112.5), pos2(300.0, 117.0)])
        );
        assert_eq!(connector(r, spot, at, false), Connector::Line(pos2(300.0, 113.0), at));
    }

    #[test]
    fn labels_honour_obstacles() {
        let l = layout(680.0, 170.0);
        let at = pos2(300.0, 120.0);
        // Something right above the peak pushes the label up a row.
        let block = Rect::from_min_size(pos2(280.0, 100.0), vec2(40.0, 10.0));
        let mut placer = LabelPlacer::new(l.plot, 12.0, [block]);
        let (r, spot) = placer.place(at, 30.0).unwrap();
        assert_eq!(spot, LabelSpot::Stacked);
        assert!(!overlaps(r, block, 2.0));
        // A wall across the plot's upper part leaves only the side spots.
        let wall = Rect::from_min_max(pos2(0.0, 0.0), pos2(700.0, 116.0));
        let mut placer = LabelPlacer::new(l.plot, 12.0, [wall]);
        let (r, spot) = placer.place(at, 30.0).unwrap();
        assert_eq!(spot, LabelSpot::Below);
        assert_eq!(connector(r, spot, at, true), Connector::Line(pos2(300.0, 127.0), at));
        // Blocked everywhere: skipped.
        let mut placer = LabelPlacer::new(l.plot, 12.0, [l.plot.expand(20.0)]);
        assert!(placer.place(at, 30.0).is_none());
    }

    #[test]
    fn labels_avoid_each_other_and_other_peaks() {
        let l = layout(680.0, 170.0);
        let mut placer = LabelPlacer::new(l.plot, 12.0, []);
        let a = pos2(300.0, 120.0);
        let (ra, _) = placer.place(a, 30.0).unwrap();
        // Same peak again: the spot above is taken, so it stacks.
        let (rb, spot) = placer.place(a, 30.0).unwrap();
        assert_eq!(spot, LabelSpot::Stacked);
        assert!(!overlaps(ra, rb, 2.0));
        // Just under the first peak, the spot above would cover that peak's dot.
        let (rc, spot) = placer.place(pos2(300.0, 140.0), 30.0).unwrap();
        assert_eq!(spot, LabelSpot::Right);
        assert!(!overlaps(rc, Rect::from_center_size(a, Vec2::splat(10.0)), 2.0));
        assert!(!overlaps(rc, ra, 2.0) && !overlaps(rc, rb, 2.0));
    }

    #[test]
    fn side_labels_connect_from_their_near_edge() {
        let l = layout(680.0, 170.0);
        // No room above (the header reaches down to 40), so it goes right.
        let at = pos2(300.0, 50.0);
        let mut placer = LabelPlacer::new(l.plot, 40.0, []);
        let (r, spot) = placer.place(at, 30.0).unwrap();
        assert_eq!(spot, LabelSpot::Right);
        assert_eq!(connector(r, spot, at, true), Connector::Line(pos2(308.0, 50.0), at));
        // At the plot's right edge it goes left instead.
        let at = pos2(l.plot.right() - 2.0, 50.0);
        let (r, spot) = placer.place(at, 30.0).unwrap();
        assert_eq!(spot, LabelSpot::Left);
        assert_eq!(connector(r, spot, at, true), Connector::Line(pos2(at.x - 8.0, 50.0), at));
    }

    #[test]
    fn close_reference_pair_on_a_narrow_panel_does_not_collide() {
        // The prototype's sample-lap scene on a 300 pt panel: reference peaks of 59% at
        // 3756 m and 33% at 3894 m (138 m is only 36 pt here), live peaks of 56% and 14%
        // just behind the car at 3800 m.
        let (p, l) = (panel(300.0, 190.0), layout(300.0, 190.0));
        let s = metres(300.0, 190.0);
        let mut band = XBand::new(l.plot, p);
        band.place_pill(s.x(0.0), 34.0);
        let cursor_dots = [0.0, 0.09].map(|v| Rect::from_center_size(pos2(s.x(0.0), s.y(v)), Vec2::splat(12.0)));
        let mut placer = LabelPlacer::new(l.plot, p.top() + 2.0, band.into_taken().into_iter().chain(cursor_dots));
        let mut peaks = [
            Peak { live: false, v: 94.0, value: 0.33 },
            Peak { live: false, v: -44.0, value: 0.59 },
            Peak { live: true, v: -180.0, value: 0.14 },
            Peak { live: true, v: -30.0, value: 0.56 },
        ];
        sort_for_placement(&mut peaks);
        let at = |pk: &Peak| pos2(s.x(pk.v), s.y(pk.value));
        let placed: Vec<(Rect, LabelSpot)> = peaks.iter().filter_map(|pk| placer.place(at(pk), 30.0)).collect();
        let spots: Vec<LabelSpot> = placed.iter().map(|&(_, spot)| spot).collect();
        // Like the prototype: the reference 59% stacks over the live 56%.
        assert_eq!(spots, [LabelSpot::Above, LabelSpot::Above, LabelSpot::Stacked, LabelSpot::Above]);
        let rects: Vec<Rect> = placed.iter().map(|&(r, _)| r).collect();
        let dots: Vec<Rect> = peaks.iter().map(|pk| Rect::from_center_size(at(pk), Vec2::splat(10.0))).collect();
        for (i, r) in rects.iter().enumerate() {
            assert!(rects[i + 1..].iter().all(|o| !overlaps(*r, *o, 2.0)));
            // Clear of its own dot and those of labels placed before it.
            assert!(!overlaps(*r, dots[i], 0.0));
            assert!(dots[..i].iter().all(|d| !overlaps(*r, *d, 2.0)));
            assert!(cursor_dots.iter().all(|d| !overlaps(*r, *d, 2.0)));
        }
    }

    #[test]
    fn labels_never_leave_the_plot() {
        for (w, h) in [(680.0, 170.0), (300.0, 190.0), (260.0, 90.0)] {
            let (p, l) = (panel(w, h), layout(w, h));
            let top = p.top() + 2.0;
            let mut placer = LabelPlacer::new(l.plot, top, []);
            let mut placed = Vec::new();
            for i in 0..=40 {
                for j in 0..=10 {
                    let at = pos2(
                        l.plot.left() + l.plot.width() * i as f32 / 40.0,
                        l.plot.top() + l.plot.height() * j as f32 / 10.0,
                    );
                    let width = 24.0 + (i % 3) as f32 * 6.0;
                    if let Some((r, _)) = placer.place(at, width) {
                        assert!(r.left() >= l.plot.left() - 0.5 && r.right() <= l.plot.right() + 0.5, "{r:?}");
                        assert!(r.top() >= top && r.bottom() <= l.plot.bottom() - 1.0, "{r:?}");
                        placed.push(r);
                    }
                }
            }
            assert!(!placed.is_empty());
            for (i, a) in placed.iter().enumerate() {
                assert!(placed[i + 1..].iter().all(|b| !overlaps(*a, *b, 2.0)));
            }
        }
    }

    fn decimate(points: impl IntoIterator<Item = Pos2>) -> Vec<Pos2> {
        let mut d = Decimator::new(1.0);
        points.into_iter().for_each(|p| d.push(p));
        d.finish()
    }

    #[test]
    fn decimation_leaves_a_sparse_line_alone() {
        let line: Vec<Pos2> = (0..50).map(|i| pos2(10.0 + i as f32 * 1.5, 50.0 + (i % 5) as f32)).collect();
        assert_eq!(decimate(line.clone()), line);
    }

    #[test]
    fn decimation_keeps_peaks_and_order() {
        // 200 points in one column, a one-sample spike up and one down, then a second column.
        let mut line: Vec<Pos2> = (0..200).map(|i| pos2(100.0 + i as f32 * 0.004, 80.0)).collect();
        line[70].y = 10.0;
        line[140].y = 95.0;
        line.push(pos2(101.5, 60.0));
        assert_eq!(
            decimate(line.clone()),
            [pos2(100.0, 80.0), line[70], line[140], line[199], pos2(101.5, 60.0)],
            "first, highest, lowest, last, in order"
        );
        // Stationary jitter across a column boundary stays one column.
        let jitter = (0..1000).map(|i| pos2(361.0 + if i % 2 == 0 { 1e-4 } else { -1e-4 }, (i % 17) as f32));
        assert!(decimate(jitter).len() <= 4);
    }

    #[test]
    fn decimated_line_is_bounded_by_the_width() {
        // 36,000 samples over 300 points of width: at most four per column.
        let dense = (0..36_000).map(|i| pos2(i as f32 / 120.0, if i % 3 == 0 { 0.0 } else { 100.0 }));
        let thin = decimate(dense);
        assert!(thin.len() <= 4 * 301, "{}", thin.len());
        assert!(thin.iter().any(|p| p.y == 0.0) && thin.iter().any(|p| p.y == 100.0));
        assert!(thin.windows(2).all(|w| w[0] != w[1]), "no repeated points");
        assert!(decimate([]).is_empty());
    }

    #[test]
    fn message_uses_the_look_ahead_only_when_it_fits() {
        // "Drop a Garage 61 CSV here, or load one in ⚙ settings" is 210.7 pt wide.
        let w = 210.7;
        let wide = layout(680.0, 170.0).plot;
        let cursor = metres(680.0, 170.0).x(0.0);
        let mid = message_center_x(wide, Some(cursor), w);
        assert_eq!(mid, (cursor + wide.right()) / 2.0);
        assert!(mid - w / 2.0 > cursor + MESSAGE_PAD - 0.01);

        // 380x170: a 172 pt look-ahead is too narrow, so the text centres across the plot.
        let compact = layout(380.0, 170.0).plot;
        let cursor = metres(380.0, 170.0).x(0.0);
        assert_eq!(compact.right() - cursor, 172.0);
        assert_eq!(message_center_x(compact, Some(cursor), w), compact.center().x);
        assert_eq!(message_center_x(compact, None, w), compact.center().x);

        // Little look-ahead but plenty of history: the text goes behind the car.
        let cursor = wide.right() - 40.0;
        assert_eq!(message_center_x(wide, Some(cursor), w), (wide.left() + cursor) / 2.0);

        // Always inside the plot; text wider than the plot starts at its left edge.
        for plot in [wide, compact] {
            for cursor in [None, Some(plot.left()), Some(plot.center().x), Some(plot.right() - 1.0)] {
                let mid = message_center_x(plot, cursor, w);
                assert!(mid - w / 2.0 >= plot.left() - 0.01 && mid + w / 2.0 <= plot.right() + 0.01);
            }
        }
        assert_eq!(message_center_x(compact, None, 400.0), compact.left() + 200.0);
    }

    #[test]
    fn a_label_wider_than_the_plot_is_skipped() {
        let l = layout(260.0, 170.0);
        let mut placer = LabelPlacer::new(l.plot, 12.0, []);
        assert!(placer.place(l.plot.center(), l.plot.width() + 10.0).is_none());
    }
}
