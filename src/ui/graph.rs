//! The throttle/brake graph: a port of `prototype/src/graph.js` to egui's painter.

use eframe::egui::{Painter, Rect};

use crate::lap::Lap;
use crate::settings::{Axis, LabelMode};
use crate::trace::LiveTrace;

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

/// Paints the graph into `panel` (the overlay panel's inner rect; its background is
/// already painted). Draws nothing outside `panel`.
pub fn paint(_painter: &Painter, _panel: Rect, _scene: &GraphScene) {
    // Implemented by the graph module owner.
}
