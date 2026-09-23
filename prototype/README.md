# Input Telemetry Overlay: UI design prototype

A working prototype of an iRacing overlay that graphs throttle and brake input against a
comparison lap. The live inputs are simulated for now; the reference lap comes from a
Garage 61 CSV export.

Open `index.html` in Chrome or Edge. No build step or install is needed. You can also
serve the folder (`python -m http.server 8617`) and visit `http://localhost:8617`.

## What's on screen

- **Reference lap (filled areas):** throttle in green and brake in red, drawn from the CSV.
  It's lined up with the car by `LapDistPct`, and it runs ahead of the car as a preview.
- **Live inputs (solid lines):** your throttle and brake, drawn up to the car position (the
  white cursor, with the current lap distance in the pill below it).
- **Brake peak labels:** the highest % in each braking zone. Solid red pills mark your
  peaks and update while you're still on the brake; the reference's are plain numbers with
  a dark halo. Your peak and the reference's in one zone share a tag. Each label takes the
  spot around its peak that covers the least of the traces, keeps it from frame to frame,
  and is left out when no spot is clear (`src/peak-labels.js`).
- **Brake point countdown:** a second window that counts 3-2-1-BRAKE into each of the
  reference lap's brake points, with a bar filling in step and the zone's target peak
  pressure beside it; the three segments fill green, yellow and red. When you brake, it
  grades your timing against the reference brake point (very early, early, good, perfect,
  late, very late, or no brake) in seconds and metres, and
  keeps a strip of pips with your grade in every zone. With **Brake points on the graph**
  on, the graph marks each reference brake point (red ▼) and underlines your gap to it.
  [`brake-cue.html`](brake-cue.html) is the design board: every state, the grades, sizes,
  the behaviour spec, and the peak label rules with examples.
- **Resizing:** hover the overlay to show 8 anchors (corners and edges). Drag one to
  resize, or drag the header to move the overlay. Size and position are saved.
- **Gear (top right):** opens the settings. The panel sits outside the overlay so you can
  see changes as you make them.

| Tab | Settings |
|---|---|
| Display | Background opacity, reference-fill opacity, lock size and position, reset layout |
| Brakes | Brake point countdown on or off, countdown length, cue early by (reaction allowance), skip zones below a peak, beeps; the "good" and "perfect" timing windows; brake points on the graph; brake peak labels on or off, which traces get them (live, reference or both), minimum peak |
| Timing | X-axis in distance or time; history behind the car; look-ahead; update rate (30 or 60 Hz) |
| Reference | Loaded lap card (driver, car, track and lap time from the file name, plus a warning if it doesn't match the session); drop or browse for a CSV; show or hide the reference |

The overlay reflows as it's resized. The legend hides below 540 px wide, the title
shortens to "THR / BRK" below 330 px, and the x-axis labels hide below 118 px tall.

## Files

| Path | Role |
|---|---|
| `index.html` | Overlay, settings popover and prototype harness markup |
| `src/styles.css` | Design tokens and all styling |
| `src/graph.js` | Canvas renderer: fills, lines, cursor, axes, S/F marker, peak labels |
| `src/peak-labels.js` | Where peak labels go: a per-column mask of the drawn traces and a scored search for each label's spot |
| `src/lap.js` | Garage 61 CSV parser and lap model (brake zones, `LapDistPct` lookups, downshift blips flattened as `src/lap.rs` does) |
| `src/live-trace.js` | Rolling buffer of live samples and brake events (brake-on point and peak) |
| `src/brake-cue.js` | Brake point countdown: zone timing and grading, the window's renderer, beeps |
| `brake-cue.html`, `src/board.js` | Design board for the brake point countdown and the peak labels |
| `src/frame.js` | Move and 8-anchor resize |
| `src/settings.js` | Persisted settings store and the settings popover |
| `src/main.js` | Wiring and render loop |
| `src/sim.js`, `src/stage.js` | **Prototype only:** simulated driver and the track backdrop. The harness's **Next zone** button (or **N**) jumps to just before the next countdown |
| `data/sample-lap.js` | The provided Garage 61 lap (5 columns, with Gear for the blips), embedded so `file://` works |

## Connecting to iRacing later

Replace `SimulatedDriver` with a reader for the iRacing SDK. Each sample needs `SessionTime`,
`LapDistPct`, `Lap`, `Brake` and `Throttle`, and `D = (Lap + LapDistPct) × TrackLength`.
Take the track length and name from the session info (`WeekendInfo.TrackLength`,
`TrackDisplayName`). The prototype estimates the track length by integrating the lap's
speed column instead. In an Electron build, `frame.js` would call `setBounds()` on a
transparent, always-on-top window.
