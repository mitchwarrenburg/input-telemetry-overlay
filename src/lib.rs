//! Input Telemetry Overlay: graphs live iRacing throttle and brake against a
//! Garage 61 reference lap.
//!
//! The library holds everything testable without a window; `main.rs` runs the app.

pub mod app;
pub mod demo;
pub mod lap;
pub mod library;
pub mod matching;
#[cfg(windows)]
pub mod platform;
pub mod settings;
pub mod telemetry;
pub mod trace;
pub mod ui;
