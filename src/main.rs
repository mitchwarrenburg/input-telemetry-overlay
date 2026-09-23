#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Starts the overlay: command line, logging, then the app.

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    start::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Input Telemetry Overlay runs on Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
mod start {
    use std::process::ExitCode;

    use ito::app::LaunchOptions;
    use ito::settings::SettingsTab;

    const USAGE: &str = "\
Input Telemetry Overlay: your throttle and brake against a Garage 61 reference lap.

Usage: input-telemetry-overlay [options] [lap.csv ...]

CSV files given on the command line (or dropped on the program) are added to the
lap library, and the last one becomes the reference.

Options:
  --demo                       Drive the simulated car, even if iRacing is running
  --settings <tab>             Open the settings on a tab: display, labels, timing, reference
  --screenshot <png>           Save the overlay as a PNG after a second, then quit
  --settings-screenshot <png>  Save the settings window as a PNG too (opens it), then quit
  --data-dir <dir>             Keep settings and laps here instead of %APPDATA%\\input-telemetry-overlay
  -h, --help                   Show this help";

    pub fn main() -> ExitCode {
        let opts = match parse_args(std::env::args().skip(1)) {
            Ok(Command::Run(opts)) => opts,
            Ok(Command::Help) => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("{e}\n\n{USAGE}");
                return ExitCode::from(2);
            }
        };
        logger::init(&opts.data_dir());
        // `run` logs its error.
        match ito::app::run(opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        }
    }

    #[derive(Debug)]
    enum Command {
        Run(LaunchOptions),
        Help,
    }

    fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
        let mut opts = LaunchOptions::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "-h" | "--help" => return Ok(Command::Help),
                "--demo" => opts.demo = true,
                "--settings" => opts.open_settings = Some(parse_tab(&value()?)?),
                "--screenshot" => opts.screenshot = Some(value()?.into()),
                "--settings-screenshot" => opts.settings_screenshot = Some(value()?.into()),
                "--data-dir" => opts.data_dir = Some(value()?.into()),
                other if other.starts_with('-') => return Err(format!("Unknown option: {other}")),
                path => opts.import.push(path.into()),
            }
        }
        Ok(Command::Run(opts))
    }

    fn parse_tab(name: &str) -> Result<SettingsTab, String> {
        match name.to_ascii_lowercase().as_str() {
            "display" => Ok(SettingsTab::Display),
            "labels" => Ok(SettingsTab::Labels),
            "timing" => Ok(SettingsTab::Timing),
            "reference" => Ok(SettingsTab::Reference),
            _ => Err(format!("No settings tab called “{name}”")),
        }
    }

    /// A small `log` backend. Debug builds: this app's messages (and libraries'
    /// warnings) on stderr. Release builds: warnings and errors appended to
    /// `<data dir>\overlay.log`.
    mod logger {
        use std::fs::{File, OpenOptions};
        use std::io::{self, Write};
        use std::path::Path;
        use std::sync::Mutex;
        use std::time::{SystemTime, UNIX_EPOCH};

        use log::{LevelFilter, Metadata, Record};

        /// The log file starts over when it grows past this.
        const MAX_LOG_BYTES: u64 = 1 << 20;

        struct Logger {
            out: Mutex<Box<dyn Write + Send>>,
            /// Level for this app's own messages; libraries log warnings and errors only.
            ours: LevelFilter,
        }

        impl Logger {
            fn level_for(&self, target: &str) -> LevelFilter {
                let ours =
                    target == "ito" || target.starts_with("ito::") || target.starts_with("input_telemetry_overlay");
                if ours { self.ours } else { LevelFilter::Warn.min(self.ours) }
            }
        }

        impl log::Log for Logger {
            fn enabled(&self, metadata: &Metadata) -> bool {
                metadata.level() <= self.level_for(metadata.target())
            }

            fn log(&self, record: &Record) {
                if !self.enabled(record.metadata()) {
                    return;
                }
                let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                if let Ok(mut out) = self.out.lock() {
                    let _ = writeln!(
                        out,
                        "{}.{:03} {:<5} {}: {}",
                        t.as_secs(),
                        t.subsec_millis(),
                        record.level(),
                        record.target(),
                        record.args()
                    );
                }
            }

            fn flush(&self) {
                if let Ok(mut out) = self.out.lock() {
                    let _ = out.flush();
                }
            }
        }

        pub fn init(data_dir: &Path) {
            let (out, ours): (Box<dyn Write + Send>, _) = if cfg!(debug_assertions) {
                (Box::new(io::stderr()), LevelFilter::Debug)
            } else {
                match open_log_file(data_dir) {
                    Ok(file) => (Box::new(file), LevelFilter::Warn),
                    Err(_) => return, // nowhere to write
                }
            };
            if log::set_boxed_logger(Box::new(Logger { out: Mutex::new(out), ours })).is_ok() {
                log::set_max_level(ours);
            }
        }

        fn open_log_file(dir: &Path) -> io::Result<File> {
            std::fs::create_dir_all(dir)?;
            let path = dir.join("overlay.log");
            let too_big = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
            OpenOptions::new().create(true).append(!too_big).write(true).truncate(too_big).open(path)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn parse(args: &[&str]) -> Result<Command, String> {
            parse_args(args.iter().map(|s| s.to_string()))
        }

        #[test]
        fn parses_every_option() {
            let Ok(Command::Run(o)) = parse(&[
                "--demo",
                "--settings",
                "Reference",
                "--screenshot",
                "a.png",
                "--settings-screenshot",
                "b.png",
                "--data-dir",
                "d",
            ]) else {
                panic!()
            };
            assert!(o.demo);
            assert_eq!(o.open_settings, Some(SettingsTab::Reference));
            assert_eq!(o.screenshot.as_deref(), Some("a.png".as_ref()));
            assert_eq!(o.settings_screenshot.as_deref(), Some("b.png".as_ref()));
            assert_eq!(o.data_dir.as_deref(), Some("d".as_ref()));
        }

        #[test]
        fn rejects_bad_input() {
            assert!(matches!(parse(&[]), Ok(Command::Run(_))));
            assert!(matches!(parse(&["--demo", "-h"]), Ok(Command::Help)));
            assert_eq!(parse(&["--settings"]).unwrap_err(), "--settings needs a value");
            assert!(parse(&["--settings", "audio"]).is_err());
            assert!(parse(&["--fast"]).is_err());
        }

        #[test]
        fn other_arguments_are_laps_to_import() {
            let Ok(Command::Run(o)) = parse(&["a.csv", "--demo", r"C:\laps\b.csv"]) else { panic!() };
            assert_eq!(o.import, [std::path::PathBuf::from("a.csv"), r"C:\laps\b.csv".into()]);
            assert!(o.demo);
        }
    }
}
