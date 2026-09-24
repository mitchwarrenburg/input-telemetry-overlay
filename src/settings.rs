//! User settings, persisted as JSON in the app's config folder.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cue::CueConfig;

pub const APP_DIR_NAME: &str = "input-telemetry-overlay";

/// `%APPDATA%\input-telemetry-overlay` (or the platform equivalent).
pub fn app_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR_NAME)
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LabelMode {
    Live,
    Reference,
    Both,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Distance,
    Time,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SettingsTab {
    // The graph's
    Display,
    Labels,
    Timing,
    // The brake point window's
    Countdown,
    Grades,
    Window,
    // Both
    Reference,
}

impl SettingsTab {
    /// The brake point window's tabs (the rest are the graph's; Reference is both's).
    pub fn is_cue(self) -> bool {
        matches!(self, SettingsTab::Countdown | SettingsTab::Grades | SettingsTab::Window)
    }
}

/// Overlay window position and size. The position is the window's outer top-left in
/// physical pixels (`px`), which says which monitor it's on; the size is in points, so
/// it looks the same on any monitor.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WindowRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// False in files from before positions were saved in pixels: `x` and `y` are
    /// points there, as the primary monitor counts them.
    #[serde(default)]
    pub px: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Panel background opacity, percent.
    pub bg_opacity: f32,
    /// Reference fill intensity, percent.
    pub ref_opacity: f32,
    /// Brake peak labels on/off.
    pub labels: bool,
    pub label_mode: LabelMode,
    /// Peaks below this percentage get no label.
    pub label_min: f32,
    /// Click-through, anchors hidden. Unlock from the tray or the hotkey.
    pub locked: bool,
    pub axis: Axis,
    /// Distance axis: metres of history behind the car.
    pub history_m: f32,
    /// Distance axis: metres of reference preview ahead of the car.
    pub ahead_m: f32,
    /// Time axis: seconds behind.
    pub history_s: f32,
    /// Time axis: seconds ahead.
    pub ahead_s: f32,
    /// Redraw rate: 30 or 60.
    pub update_hz: u32,
    pub show_ref: bool,
    /// Load a saved lap that matches the session's track and car automatically.
    pub auto_reference: bool,
    /// Run the simulated driver while iRacing isn't running.
    pub demo_when_idle: bool,
    /// Global shortcut that locks/unlocks the overlay (it's click-through while locked).
    pub unlock_hotkey: String,
    /// Last settings tab shown from the graph's gear.
    pub tab: SettingsTab,
    /// Last overlay position and size.
    pub window: Option<WindowRect>,

    // ---- Brake point window ----
    /// The countdown window is shown.
    pub cue_on: bool,
    /// Just the bar, with the target, your pressure and the distance in it.
    pub cue_compact: bool,
    /// Its background opacity, percent.
    pub cue_bg_opacity: f32,
    /// Opacity of what's drawn on it (bar, text, header), percent.
    pub cue_fg_opacity: f32,
    /// Seconds for the three counts.
    pub cue_lead: f32,
    /// Show BRAKE this many seconds before the reference brake point.
    pub cue_early: f32,
    /// Zones whose reference peak is below this percentage get no countdown.
    pub cue_min: f32,
    /// A beep on each count and a long one on BRAKE.
    pub cue_beep: bool,
    /// Both windows' backgrounds pulse red at the brake point.
    pub cue_pulse: bool,
    /// ± seconds of the good window.
    pub cue_tol: f32,
    /// ± seconds of the perfect window.
    pub cue_perfect: f32,
    /// Brake point marks and your gap to them on the graph.
    pub cue_graph: bool,
    /// Last settings tab shown from the brake point window's gear.
    pub cue_tab: SettingsTab,
    /// Last position and size of the window, full and compact.
    pub cue_window: Option<WindowRect>,
    pub cue_compact_window: Option<WindowRect>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bg_opacity: 80.0,
            ref_opacity: 100.0,
            labels: true,
            label_mode: LabelMode::Both,
            label_min: 10.0,
            locked: false,
            axis: Axis::Distance,
            history_m: 500.0,
            ahead_m: 500.0,
            history_s: 8.0,
            ahead_s: 6.0,
            update_hz: 60,
            show_ref: true,
            auto_reference: true,
            demo_when_idle: true,
            unlock_hotkey: "Ctrl+Alt+Shift+O".into(),
            tab: SettingsTab::Display,
            window: None,
            cue_on: true,
            cue_compact: false,
            cue_bg_opacity: 80.0,
            cue_fg_opacity: 100.0,
            cue_lead: 3.0,
            cue_early: 0.0,
            cue_min: 15.0,
            cue_beep: false,
            cue_pulse: true,
            cue_tol: 0.08,
            cue_perfect: 0.03,
            cue_graph: true,
            cue_tab: SettingsTab::Countdown,
            cue_window: None,
            cue_compact_window: None,
        }
    }
}

impl Settings {
    /// Reads settings, falling back to defaults for a missing or unreadable file.
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else { return Settings::default() };
        // Notepad and PowerShell 5 save UTF-8 with a byte-order mark.
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        match serde_json::from_str::<Settings>(text) {
            Ok(settings) => settings.sanitized(),
            Err(e) => {
                log::warn!("{} couldn't be read ({e}); using the default settings", path.display());
                Settings::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        write_atomic(path, serde_json::to_string_pretty(self).map_err(io::Error::other)?.as_bytes())
    }

    /// Clamps every value into the range the UI offers.
    pub fn sanitized(mut self) -> Self {
        let c = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        let d = Settings::default();
        self.bg_opacity = c(self.bg_opacity, 0.0, 100.0, d.bg_opacity);
        self.ref_opacity = c(self.ref_opacity, 0.0, 100.0, d.ref_opacity);
        self.label_min = c(self.label_min, 0.0, 50.0, d.label_min);
        self.history_m = c(self.history_m, 100.0, 1500.0, d.history_m);
        self.ahead_m = c(self.ahead_m, 0.0, 1500.0, d.ahead_m);
        self.history_s = c(self.history_s, 1.0, 20.0, d.history_s);
        self.ahead_s = c(self.ahead_s, 0.0, 15.0, d.ahead_s);
        if self.unlock_hotkey.trim().is_empty() {
            self.unlock_hotkey = d.unlock_hotkey.clone();
        }
        self.cue_bg_opacity = c(self.cue_bg_opacity, 0.0, 100.0, d.cue_bg_opacity);
        self.cue_fg_opacity = c(self.cue_fg_opacity, 30.0, 100.0, d.cue_fg_opacity);
        self.cue_lead = c(self.cue_lead, 1.5, 4.5, d.cue_lead);
        self.cue_early = c(self.cue_early, 0.0, 0.3, d.cue_early);
        self.cue_min = c(self.cue_min, 0.0, 50.0, d.cue_min);
        self.cue_tol = c(self.cue_tol, 0.02, 0.2, d.cue_tol);
        self.cue_perfect = c(self.cue_perfect, 0.01, 0.06, d.cue_perfect);
        if self.update_hz != 30 {
            self.update_hz = 60;
        }
        // Each window's gear shows its own tabs.
        if self.tab.is_cue() {
            self.tab = SettingsTab::Display;
        }
        if !self.cue_tab.is_cue() && self.cue_tab != SettingsTab::Reference {
            self.cue_tab = SettingsTab::Countdown;
        }
        for w in [&mut self.window, &mut self.cue_window, &mut self.cue_compact_window] {
            if let Some(r) = *w
                && (![r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()) || r.w < 1.0 || r.h < 1.0)
            {
                *w = None;
            }
        }
        self
    }

    /// The countdown's settings in the model's units.
    pub fn cue_config(&self) -> CueConfig {
        CueConfig {
            lead: f64::from(self.cue_lead),
            early: f64::from(self.cue_early),
            min_peak: self.cue_min / 100.0,
            tol: f64::from(self.cue_tol),
            perfect: f64::from(self.cue_perfect.min(self.cue_tol)),
        }
    }

    /// (behind, ahead) of the car in the current axis' units (metres or seconds).
    pub fn window_span(&self) -> (f64, f64) {
        match self.axis {
            Axis::Distance => (self.history_m as f64, self.ahead_m as f64),
            Axis::Time => (self.history_s as f64, self.ahead_s as f64),
        }
    }
}

/// Writes a temp file, flushes it to disk and renames it over `path`, so a crash or a
/// power loss leaves either the old file or the new one, never a partial or empty one.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    // The rename is journaled but the data isn't: without this, NTFS can keep the
    // rename and lose the contents.
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_fills_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("settings.json");
        let s = Settings {
            bg_opacity: 35.0,
            axis: Axis::Time,
            window: Some(WindowRect { x: 2100.0, y: 300.0, w: 680.0, h: 170.0, px: true }),
            ..Default::default()
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);

        std::fs::write(&path, r#"{"window": {"x": 10, "y": 20, "w": 680, "h": 170}}"#).unwrap();
        let old = WindowRect { x: 10.0, y: 20.0, w: 680.0, h: 170.0, px: false };
        assert_eq!(Settings::load(&path).window, Some(old), "an older file's points");

        std::fs::write(&path, r#"{"bg_opacity": 500, "label_mode": "live", "unknown": 1}"#).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.bg_opacity, 100.0);
        assert_eq!(s.label_mode, LabelMode::Live);
        assert_eq!(s.history_m, 500.0);
    }

    #[test]
    fn garbage_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "\u{feff}{\"bg_opacity\": 35}").unwrap();
        assert_eq!(Settings::load(&path).bg_opacity, 35.0, "a byte-order mark is fine");
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        assert_eq!(Settings::load(&dir.path().join("missing.json")), Settings::default());
    }

    #[test]
    fn write_atomic_replaces_the_file_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new").join("library.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn window_span_follows_axis() {
        let mut s = Settings::default();
        assert_eq!(s.window_span(), (500.0, 500.0));
        s.axis = Axis::Time;
        assert_eq!(s.window_span(), (8.0, 6.0));
    }
}
