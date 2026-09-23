//! User settings, persisted as JSON in the app's config folder.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
    Display,
    Labels,
    Timing,
    Reference,
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
    /// Last settings tab shown.
    pub tab: SettingsTab,
    /// Last overlay position and size.
    pub window: Option<WindowRect>,
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
        if self.update_hz != 30 {
            self.update_hz = 60;
        }
        if let Some(w) = self.window
            && (![w.x, w.y, w.w, w.h].iter().all(|v| v.is_finite()) || w.w < 1.0 || w.h < 1.0)
        {
            self.window = None;
        }
        self
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
