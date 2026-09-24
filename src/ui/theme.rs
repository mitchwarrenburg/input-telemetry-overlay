//! Colours and fonts, shared by the overlay and the settings window.
//! Values come from the HTML prototype (`prototype/src/styles.css`).

use eframe::egui::{self, Color32, FontFamily, FontId};

use crate::cue::Grade;

// ---- Data ----
/// Throttle: lines and fills.
pub const THROTTLE: Color32 = Color32::from_rgb(0x22, 0xe0, 0x7a);
/// Brake: lines and fills.
pub const BRAKE: Color32 = Color32::from_rgb(0xff, 0x3d, 0x2e);
/// Live brake-peak pill: a step darker than the line so white text clears 4.5:1.
pub const BRAKE_PILL: Color32 = Color32::from_rgb(0xe0, 0x30, 0x1f);

// ---- Peaks and the brake point countdown ----
/// Your peak labels and the lines through your peaks.
pub const YOU: Color32 = Color32::from_rgb(127, 209, 255);
/// The reference's peaks: the target.
pub const TARGET: Color32 = Color32::from_rgb(245, 200, 80);
/// The countdown's fill runs from this red…
pub const COUNT_LIGHT: Color32 = Color32::from_rgb(0xff, 0x6f, 0x62);
/// …through [`BRAKE`] to this one at the brake point.
pub const COUNT_DEEP: Color32 = Color32::from_rgb(0x8a, 0x1a, 0x11);

/// A timing grade's colour: blue early, green good, "fastest lap" purple perfect, amber
/// to orange late, grey no brake.
pub fn grade_color(grade: Grade) -> Color32 {
    match grade {
        Grade::VeryEarly => Color32::from_rgb(91, 140, 255),
        Grade::Early => Color32::from_rgb(111, 193, 255),
        Grade::Good => Color32::from_rgb(46, 230, 160),
        Grade::Perfect => Color32::from_rgb(200, 36, 208),
        Grade::Late => Color32::from_rgb(255, 177, 59),
        Grade::VeryLate => Color32::from_rgb(255, 107, 61),
        Grade::NoBrake => Color32::from_rgb(127, 139, 137),
    }
}

/// Text on a grade's colour: white on purple, near-black on the rest.
pub fn grade_ink(grade: Grade) -> Color32 {
    if grade == Grade::Perfect { Color32::WHITE } else { Color32::from_rgb(7, 16, 12) }
}

// ---- HUD ----
/// Overlay panel background (drawn with the user's opacity).
pub const SURFACE: Color32 = Color32::from_rgb(7, 9, 10);
pub const HUD_TEXT: Color32 = Color32::from_rgb(0xe8, 0xef, 0xee);
pub const HUD_MUTED: Color32 = Color32::from_rgb(0x7f, 0x8b, 0x89);
/// Hairline grid.
pub const GRID: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18); // white @ 7%
/// 0% baseline.
pub const BASELINE: Color32 = Color32::from_rgba_premultiplied(41, 41, 41, 41); // white @ 16%
/// Panel border colour at full background opacity (scale alpha with opacity).
pub const BORDER: Color32 = Color32::from_rgb(46, 224, 150);
pub const ACCENT: Color32 = Color32::from_rgb(0x2e, 0xe6, 0xa0);
pub const ACCENT_INK: Color32 = Color32::from_rgb(0x04, 0x15, 0x0d);

// ---- Settings window ----
pub const UI_SURFACE: Color32 = Color32::from_rgb(0x0e, 0x13, 0x14);
pub const UI_RAISED: Color32 = Color32::from_rgba_premultiplied(10, 10, 10, 10); // white @ 4%
pub const UI_LINE: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20); // white @ 8%
pub const UI_LINE_STRONG: Color32 = Color32::from_rgba_premultiplied(41, 41, 41, 41); // white @ 16%
pub const UI_TEXT: Color32 = Color32::from_rgb(0xe6, 0xec, 0xeb);
pub const UI_MUTED: Color32 = Color32::from_rgb(0x8b, 0x97, 0x95);
pub const OK: Color32 = Color32::from_rgb(0x37, 0xd8, 0x8f);
pub const WARN: Color32 = Color32::from_rgb(0xf5, 0xb4, 0x3c);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x7a, 0x70);

/// A dark backing for what's drawn on a panel whose background has faded by `fade`
/// (1 − its opacity), so the bar, fills and text keep their contrast over a bright sim
/// instead of washing out with the background.
pub fn scrim(fade: f32) -> Color32 {
    alpha(SURFACE, 0.62 * fade)
}

/// Muted text on a panel faded by `fade`: lighter, since grey reads worse over the sim
/// than over the dark panel.
pub fn muted(fade: f32) -> Color32 {
    HUD_MUTED.lerp_to_gamma(HUD_TEXT, 0.55 * fade.clamp(0.0, 1.0))
}

/// `color` at `alpha` (0..1) of its own alpha.
pub fn alpha(color: Color32, alpha: f32) -> Color32 {
    color.gamma_multiply(alpha.clamp(0.0, 1.0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    SemiBold,
    Bold,
}

impl Weight {
    fn family_name(self) -> &'static str {
        match self {
            Weight::Regular => "barlow-sc-regular",
            Weight::SemiBold => "barlow-sc-semibold",
            Weight::Bold => "barlow-sc-bold",
        }
    }
}

/// Barlow Semi Condensed at a weight and size (points).
pub fn font(weight: Weight, size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(weight.family_name().into()))
}

/// Registers the embedded Barlow Semi Condensed faces (SIL OFL 1.1) as named
/// families and makes Regular the default proportional font.
pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = egui::FontDefinitions::default();
    for (weight, bytes) in [
        (Weight::Regular, &include_bytes!("../../assets/fonts/BarlowSemiCondensed-Regular.ttf")[..]),
        (Weight::SemiBold, &include_bytes!("../../assets/fonts/BarlowSemiCondensed-SemiBold.ttf")[..]),
        (Weight::Bold, &include_bytes!("../../assets/fonts/BarlowSemiCondensed-Bold.ttf")[..]),
    ] {
        let name = weight.family_name().to_string();
        defs.font_data.insert(name.clone(), std::sync::Arc::new(egui::FontData::from_static(bytes)));
        // Fall back to egui's fonts for glyphs Barlow lacks (arrows, symbols).
        let mut chain = vec![name.clone()];
        chain.extend(defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
        defs.families.insert(FontFamily::Name(name.into()), chain);
    }
    defs.families.entry(FontFamily::Proportional).or_default().insert(0, Weight::Regular.family_name().to_string());
    ctx.set_fonts(defs);
}
