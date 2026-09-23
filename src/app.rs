//! The overlay application (eframe).

use std::path::PathBuf;

use crate::settings::SettingsTab;

/// Command-line options.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// Run the simulated driver even if iRacing is running.
    pub demo: bool,
    /// Open the settings window on this tab at start.
    pub open_settings: Option<SettingsTab>,
    /// Save a PNG of the overlay window here after it has rendered, then…
    pub screenshot: Option<PathBuf>,
    /// …a PNG of the settings window here (requires `open_settings`), then exit.
    pub settings_screenshot: Option<PathBuf>,
    /// Use this folder instead of %APPDATA%\input-telemetry-overlay.
    pub data_dir: Option<PathBuf>,
}

pub fn run(_opts: LaunchOptions) -> eframe::Result {
    Ok(())
}
