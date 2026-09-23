//! The settings window's content: Display, Labels, Timing and Reference tabs.

use eframe::egui;

use crate::lap::Lap;
use crate::library::Library;
use crate::matching::MatchStatus;
use crate::settings::Settings;
use crate::telemetry::SessionInfo;

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
    /// The lap drawn as the reference and how it matches the session.
    pub reference: Option<(&'a Lap, MatchStatus)>,
    /// Library id of the reference; `None` for the bundled demo lap or no reference.
    pub active_id: Option<&'a str>,
    /// Last import/load error to show in the Reference tab.
    pub error: Option<&'a str>,
    pub connection: ConnectionState,
    /// A file dialog is open (disable Browse).
    pub browsing: bool,
    /// Files are being dragged over the settings window (highlight the drop zone).
    pub file_hover: bool,
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
    ResetLayout,
    /// Settings back to defaults (keeps window position, library and hotkey).
    ResetAll,
    DismissError,
}

pub struct PanelOutput {
    pub actions: Vec<PanelAction>,
    /// Height the content needs, points; the window is resized to it.
    pub desired_height: f32,
}

/// Draws the settings panel into `ui` (the whole settings window, which is
/// transparent: the panel paints its own rounded background). Edits `settings` in
/// place; the caller saves and applies changes.
pub fn show(_ui: &mut egui::Ui, _settings: &mut Settings, _cx: &PanelContext) -> PanelOutput {
    PanelOutput { actions: Vec::new(), desired_height: 400.0 }
}
