#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    ito::app::run(ito::app::LaunchOptions::default())
}
