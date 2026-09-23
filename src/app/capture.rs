//! PNG in and out (window icons, `--screenshot`), and the screenshot sequence.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, Rect, ViewportCommand, ViewportId};

/// RGBA pixels (unmultiplied) and their size.
pub struct Rgba {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Decodes an 8-bit RGB or RGBA PNG.
pub fn decode_png(bytes: &[u8]) -> Result<Rgba, png::DecodingError> {
    let mut decoder = png::Decoder::new(io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info()?;
    let size = reader.output_buffer_size().ok_or(png::DecodingError::LimitsExceeded)?;
    let mut pixels = vec![0; size];
    let info = reader.next_frame(&mut pixels)?;
    if info.color_type != png::ColorType::Rgba {
        return Err(png::DecodingError::IoError(io::Error::other("expected an RGB or RGBA image")));
    }
    pixels.truncate(info.buffer_size());
    Ok(Rgba { pixels, width: info.width, height: info.height })
}

/// Writes an image as an RGBA PNG (alpha unmultiplied, so transparency survives).
pub fn write_png(path: &Path, image: &ColorImage) -> io::Result<()> {
    let file = io::BufWriter::new(std::fs::File::create(path)?);
    let [w, h] = image.size;
    let mut encoder = png::Encoder::new(file, w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(&rgba).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)
}

/// `--screenshot` / `--settings-screenshot`: once the windows have rendered for a
/// moment, save them and ask to quit.
pub struct Screenshots {
    overlay: Option<PathBuf>,
    settings: Option<PathBuf>,
    started: Instant,
    requested: bool,
}

impl Screenshots {
    const SETTLE: Duration = Duration::from_secs(1);
    /// Give up on the overlay screenshot (e.g. the window never painted) after this.
    const GIVE_UP: Duration = Duration::from_secs(10);

    /// `None` when neither screenshot was asked for.
    pub fn new(overlay: Option<PathBuf>, settings: Option<PathBuf>) -> Option<Self> {
        (overlay.is_some() || settings.is_some()).then(|| Self {
            overlay,
            settings,
            started: Instant::now(),
            requested: false,
        })
    }

    /// Call every overlay frame. `settings_px` is the settings window's screen rectangle
    /// in physical pixels, if it's open. Returns true when done.
    pub fn update(&mut self, ctx: &egui::Context, settings_px: Option<Rect>) -> bool {
        // The capture happens on the paint after the request, and the result arrives
        // as an input event on the frame after that: keep frames coming.
        ctx.request_repaint();
        if self.started.elapsed() < Self::SETTLE {
            return false;
        }
        if let Some(path) = &self.overlay {
            if !self.requested {
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Screenshot(egui::UserData::default()));
                self.requested = true;
                return false;
            }
            match root_screenshot(ctx) {
                Some(image) => save(path, &image),
                None if self.started.elapsed() > Self::GIVE_UP => log::error!("No overlay screenshot arrived"),
                None => return false,
            }
            self.overlay = None;
        }
        if let Some(path) = self.settings.take() {
            match settings_px.and_then(capture_rect) {
                Some(mut image) => {
                    // The screen behind the panel's rounded corners isn't ours to save.
                    clear_outside_corners(&mut image, crate::ui::settings_panel::RADIUS * ctx.pixels_per_point());
                    save(&path, &image);
                }
                None => log::error!("No settings window to capture for {}", path.display()),
            }
        }
        true
    }
}

fn root_screenshot(ctx: &egui::Context) -> Option<std::sync::Arc<ColorImage>> {
    ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { viewport_id, image, .. } if *viewport_id == ViewportId::ROOT => {
                Some(image.clone())
            }
            _ => None,
        })
    })
}

fn capture_rect(r: Rect) -> Option<ColorImage> {
    let (min, max) = (r.min.round(), r.max.round());
    crate::platform::capture_screen(min.x as i32, min.y as i32, (max.x - min.x) as i32, (max.y - min.y) as i32)
}

/// Makes the pixels outside rounded corners of `radius` px fully transparent.
fn clear_outside_corners(image: &mut ColorImage, radius: f32) {
    let [w, h] = image.size;
    let r = radius.max(0.0);
    for y in 0..h {
        for x in 0..w {
            // Distance past the nearest corner's circle, if the pixel is in a corner box.
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = if px < r {
                r
            } else if px > w as f32 - r {
                w as f32 - r
            } else {
                continue;
            };
            let cy = if py < r {
                r
            } else if py > h as f32 - r {
                h as f32 - r
            } else {
                continue;
            };
            if (px - cx).hypot(py - cy) > r {
                image.pixels[y * w + x] = egui::Color32::TRANSPARENT;
            }
        }
    }
}

fn save(path: &Path, image: &ColorImage) {
    match write_png(path, image) {
        Ok(()) => log::info!("Saved {}", path.display()),
        Err(e) => log::error!("Couldn't save {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;

    #[test]
    fn png_round_trips_with_alpha() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.png");
        let pixels = vec![
            Color32::from_rgba_unmultiplied(255, 0, 0, 255),
            Color32::TRANSPARENT,
            Color32::from_rgba_unmultiplied(0, 200, 100, 128),
            Color32::WHITE,
        ];
        write_png(&path, &ColorImage::new([2, 2], pixels)).unwrap();
        let back = decode_png(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((back.width, back.height), (2, 2));
        assert_eq!(&back.pixels[0..8], &[255, 0, 0, 255, 0, 0, 0, 0]);
        assert_eq!(back.pixels[11], 128);
        assert!((i32::from(back.pixels[9]) - 200).abs() <= 2, "unmultiplied: {}", back.pixels[9]);
    }

    #[test]
    fn decodes_the_app_icons() {
        for (bytes, size) in [
            (&include_bytes!("../../assets/icon/icon-256.png")[..], 256),
            (&include_bytes!("../../assets/icon/icon-32.png")[..], 32),
        ] {
            let icon = decode_png(bytes).unwrap();
            assert_eq!((icon.width, icon.height), (size, size));
            assert_eq!(icon.pixels.len(), (size * size * 4) as usize);
        }
    }

    #[test]
    fn corners_outside_the_radius_are_cleared() {
        let mut image = ColorImage::new([40, 30], vec![egui::Color32::WHITE; 40 * 30]);
        clear_outside_corners(&mut image, 12.0);
        let at = |x: usize, y: usize| image.pixels[y * 40 + x];
        for (x, y) in [(0, 0), (39, 0), (0, 29), (39, 29), (1, 2)] {
            assert_eq!(at(x, y), egui::Color32::TRANSPARENT, "corner pixel {x},{y}");
        }
        for (x, y) in [(20, 0), (0, 15), (12, 12), (20, 15), (39, 15)] {
            assert_eq!(at(x, y), egui::Color32::WHITE, "inside pixel {x},{y}");
        }
    }
}
