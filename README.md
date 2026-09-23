# Input Telemetry Overlay

An iRacing overlay that graphs your throttle and brake live against a reference lap from
[Garage 61](https://garage61.net), and picks the right reference lap for the track and car
by itself.

![The overlay: live throttle and brake lines over a Garage 61 reference lap](docs/images/overlay.png)

- **Your inputs against a fast lap, corner by corner.** The solid lines are your throttle
  and brake. The filled areas are the reference lap, lined up by lap distance, so a lap
  of any pace matches corner for corner.
- **Look-ahead.** The reference runs ahead of the white cursor (your car), so you can see
  the next braking zone coming.
- **Brake peak labels.** Each braking zone shows the highest brake pressure: yours (solid
  red) and the reference's (outlined).
- **A lap library that follows you.** Every Garage 61 lap you load is kept. When a session
  starts, the overlay loads the saved lap for that track, layout and car.
- **Made to sit on top of the sim.** Transparent, resizable, click-through when locked,
  with its own settings window and a tray icon. Written in Rust and drawn with OpenGL, so
  it stays light next to the sim.

## Install

1. Download `input-telemetry-overlay-<version>-windows-x64.zip` from
   [Releases](../../releases) and unzip it anywhere.
2. Run `input-telemetry-overlay.exe`.
3. In iRacing, use **windowed** or **borderless** display mode. Overlays can't draw on
   top of exclusive full screen.

Windows 10 or 11 with OpenGL 2 or newer (any GPU from the last decade).

With iRacing closed, the overlay runs a demo lap so you can position it and try the
settings. Turn that off in **Settings → Display**.

## Load a reference lap from Garage 61

1. Open a lap on garage61.net and export it as **CSV**. The file name looks like
   `Garage 61 - Driver - Car - Track (Layout) - 01.55.992 - ….csv`.
2. Drop the CSV on the overlay, or open **⚙ → Reference → Browse**. You can also drop it
   on the `.exe`, or pass it on the command line.

The lap is copied into the library (`%APPDATA%\input-telemetry-overlay\laps`), becomes the
reference, and is picked again automatically whenever you drive that track and car.

![Reference settings with the lap library](docs/images/settings-reference.png)

### How a lap is matched to a session

Garage 61 and the sim name tracks and cars differently: Garage 61 says "Silverstone
Circuit (Grand Prix)" where iRacing's session says "Silverstone Circuit" / "Arena Grand
Prix". So the overlay checks the lap itself:

- **Track and layout:** where the lap was driven (its GPS columns) against the track's
  location, and the lap's length (from its Speed column) against the session's track
  length, which tells layouts of one circuit apart.
- **Car:** the car name, allowing for suffixes ("Dallara P217" vs "Dallara P217 LMP2").
- Names are the fallback when the file has no speed or position columns.

A lap from another car on the same layout is still drawn, with a "Different car" note. A
lap from another track or layout is hidden, because its distances wouldn't line up.

Garage 61 exports the car's throttle, which includes the auto-blip on every downshift: a
50-75% spike for two or three samples. Those are flattened when a lap is loaded, and the
live line reads your pedal (`ThrottleRaw`), so neither side shows them.

## Using the overlay

| Action | How |
| --- | --- |
| Move | Drag the title bar |
| Resize | Hover the overlay, then drag a corner or edge anchor |
| Settings | The gear in the top-right corner, or the tray icon |
| Close | The × next to the gear, or the tray icon's Quit |
| Lock (click-through) | Settings → Display → Lock size & position |
| Unlock | **Ctrl+Alt+Shift+O** (change it in Settings → Display), or the tray icon |

### Settings

| Tab | What's there |
| --- | --- |
| Display | Background and reference opacity, lock, unlock shortcut, demo mode, reset size & position |
| Labels | Brake peak labels on or off; for your laps, the reference or both; minimum peak |
| Timing | X-axis in distance or time; history behind the car; look-ahead; update rate (30 or 60 Hz) |
| Reference | The current reference and how it matches the session, the saved laps, loading CSVs, auto-pick |

Settings and the library live in `%APPDATA%\input-telemetry-overlay`.

![Display settings](docs/images/settings-display.png)

## Command line

```text
input-telemetry-overlay [options] [lap.csv ...]

  --demo                       Drive the simulated car, even if iRacing is running
  --settings <tab>             Open the settings on a tab: display, labels, timing, reference
  --screenshot <png>           Save the overlay as a PNG after a second, then quit
  --settings-screenshot <png>  Save the settings window as a PNG too (opens it), then quit
  --data-dir <dir>             Keep settings and laps here instead of %APPDATA%\input-telemetry-overlay
```

CSV files given on the command line are added to the library; the last one becomes the
reference. If the overlay is already running, they're handed to it.

## Build from source

Requires Rust 1.95 or newer on Windows.

```sh
cargo run --release              # the overlay
cargo run --release -- --demo    # with the simulated driver
cargo test
```

Without iRacing, a fake sim can stand in for the real one:

```sh
cargo run --example fake_iracing     # publishes iRacing's shared memory with a looping lap
cargo run --example iracing_probe    # prints what the telemetry reader sees
```

`cargo run --example graph_preview` and `cargo run --example settings_preview` render the
graph and settings panel on their own.

### How it's put together

| Path | What |
| --- | --- |
| `src/telemetry/` | iRacing reader thread (via the [kerb](https://crates.io/crates/kerb) crate) and the session-info parser |
| `src/lap.rs` | Garage 61 CSV parser and lap model |
| `src/library.rs`, `src/matching.rs` | Lap library and session matching |
| `src/trace.rs` | Live trace, brake events and lap-distance tracking |
| `src/ui/` | Graph renderer, overlay chrome, settings panel and widgets (egui) |
| `src/app.rs`, `src/app/` | The app: windows, telemetry and demo, reference selection, tray and shortcut |
| `prototype/` | The HTML design prototype the UI was built from (`python -m http.server --directory prototype`) |

## Credits

- [egui/eframe](https://github.com/emilk/egui) for the UI and
  [kerb](https://github.com/mvoof/kerb) for iRacing's shared memory.
- [Barlow Semi Condensed](https://github.com/jpt/barlow) by The Barlow Project Authors,
  under the SIL Open Font License 1.1 (`assets/fonts/OFL.txt`).
- The bundled demo lap (Ferrari 296 GT3, Silverstone) is an anonymized Garage 61 export.

Not affiliated with iRacing or Garage 61.

## License

[MIT](LICENSE)
