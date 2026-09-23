//! Window geometry: the overlay window around its panel, where it starts and whether
//! a saved spot is still on screen, and where the settings window goes. Sizes are in
//! points; positions across monitors in physical pixels, since a point is a different
//! size on each monitor.

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::settings::WindowRect;
use crate::ui::overlay::{MARGIN, MIN_PANEL};

/// A monitor in physical pixels, and its pixels per point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub rect: Rect,
    pub scale: f32,
}

/// Panel size on first launch and after a layout reset.
pub const DEFAULT_PANEL: Vec2 = vec2(680.0, 170.0);
/// Space between the overlay and the settings window.
const SETTINGS_GAP: f32 = 10.0;
/// Keep windows this far from the monitor edges.
const EDGE_PAD: f32 = 8.0;

/// The window around a panel of `panel` size (the panel plus the anchor margin).
pub fn window_size(panel: Vec2) -> Vec2 {
    panel + Vec2::splat(2.0 * MARGIN)
}

/// Smallest window: the smallest panel plus the margin.
pub fn min_window_size() -> Vec2 {
    window_size(MIN_PANEL)
}

/// The default overlay window on a monitor: the panel centred, a little above the
/// bottom edge (clear of the taskbar).
pub fn default_window(monitor: Rect) -> Rect {
    let w = DEFAULT_PANEL.x.min(monitor.width() - 32.0).max(MIN_PANEL.x);
    let h = DEFAULT_PANEL.y;
    let x = monitor.center().x - w / 2.0;
    let y = (monitor.bottom() - h - 64.0).max(monitor.top() + 16.0);
    Rect::from_min_size(pos2(x, y), vec2(w, h)).expand(MARGIN)
}

/// Differ by more than rounding noise (or in units).
pub fn moved(a: WindowRect, b: WindowRect) -> bool {
    a.px != b.px || [a.x - b.x, a.y - b.y, a.w - b.w, a.h - b.h].iter().any(|d| d.abs() > 0.5)
}

/// Where a saved window goes, physical pixels. Settings from before positions were
/// saved in pixels hold points, which were placed at the primary monitor's scale.
pub fn physical_position(w: WindowRect, primary_scale: f32) -> Pos2 {
    if w.px { pos2(w.x, w.y) } else { pos2(w.x, w.y) * primary_scale }
}

/// Enough of the header of a window at `pos` (physical pixels), `width` points wide,
/// is on one of the monitors to grab it.
pub fn reachable(pos: Pos2, width: f32, monitors: &[Monitor]) -> bool {
    monitors.iter().any(|m| {
        let header = Rect::from_min_size(pos, vec2(width, 26.0 + MARGIN) * m.scale);
        m.rect.intersect(header).area() >= 40.0 * 16.0 * m.scale * m.scale
    })
}

/// Where the settings window goes, outside the overlay `panel` so changes stay
/// visible: right-aligned above it if it fits on the monitor, else below, else beside.
pub fn settings_position(panel: Rect, size: Vec2, monitor: Rect) -> Pos2 {
    let clamp_x = |x: f32| x.min(monitor.right() - EDGE_PAD - size.x).max(monitor.left() + EDGE_PAD);
    let clamp_y = |y: f32| y.min(monitor.bottom() - EDGE_PAD - size.y).max(monitor.top() + EDGE_PAD);
    let above = panel.top() - SETTINGS_GAP - size.y;
    let below = panel.bottom() + SETTINGS_GAP;
    if above >= monitor.top() + EDGE_PAD {
        pos2(clamp_x(panel.right() - size.x), above)
    } else if below + size.y <= monitor.bottom() - EDGE_PAD {
        pos2(clamp_x(panel.right() - size.x), below)
    } else if panel.right() + SETTINGS_GAP + size.x <= monitor.right() - EDGE_PAD {
        pos2(panel.right() + SETTINGS_GAP, clamp_y(panel.top()))
    } else {
        pos2(clamp_x(panel.left() - SETTINGS_GAP - size.x), clamp_y(panel.top()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect { min: Pos2::ZERO, max: pos2(1920.0, 1080.0) };
    const SETTINGS: Vec2 = vec2(304.0, 420.0);

    fn panel_at(x: f32, y: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), DEFAULT_PANEL)
    }

    #[test]
    fn default_window_is_centred_above_the_taskbar() {
        let w = default_window(MONITOR);
        assert_eq!(w.size(), window_size(DEFAULT_PANEL));
        assert_eq!(w.center().x, 960.0);
        assert_eq!(w.shrink(MARGIN).bottom(), 1080.0 - 64.0);

        let second = default_window(MONITOR.translate(vec2(-1920.0, 0.0)));
        assert_eq!(second.center().x, -960.0);
    }

    const SAVED: WindowRect = WindowRect { x: 2100.0, y: 300.0, w: 696.0, h: 186.0, px: true };

    #[test]
    fn saved_windows_move_only_beyond_rounding() {
        assert!(!moved(SAVED, WindowRect { x: 2100.3, ..SAVED }));
        assert!(moved(SAVED, WindowRect { x: 2102.0, ..SAVED }));
        assert!(moved(SAVED, WindowRect { px: false, ..SAVED }), "re-saved in pixels");
    }

    #[test]
    fn old_point_positions_land_where_they_used_to() {
        assert_eq!(physical_position(SAVED, 1.5), pos2(2100.0, 300.0));
        assert_eq!(physical_position(WindowRect { px: false, ..SAVED }, 1.5), pos2(3150.0, 450.0));
    }

    /// 1920×1080 at 100%, with 3840×2160 at 150% to its right.
    const PRIMARY: Monitor = Monitor { rect: MONITOR, scale: 1.0 };
    const SECONDARY: Monitor = Monitor { rect: Rect { min: pos2(1920.0, 0.0), max: pos2(5760.0, 2160.0) }, scale: 1.5 };

    #[test]
    fn off_screen_windows_are_unreachable() {
        let monitors = [PRIMARY, SECONDARY];
        let w = SAVED.w;
        assert!(reachable(default_window(MONITOR).min, w, &[PRIMARY]));
        assert!(reachable(pos2(2100.0, 300.0), w, &monitors), "on the scaled monitor");
        assert!(reachable(pos2(1500.0, 300.0), w, &monitors), "across both");
        assert!(!reachable(pos2(2100.0, 300.0), w, &[PRIMARY]), "that monitor's gone");
        assert!(!reachable(pos2(3000.0, 1500.0), w, &[PRIMARY]), "below the primary");
        assert!(!reachable(pos2(2100.0, 2170.0), w, &monitors), "under the bottom edge");
        assert!(!reachable(default_window(MONITOR).min, w, &[]));
    }

    #[test]
    fn settings_go_above_a_low_overlay() {
        let panel = panel_at(620.0, 846.0);
        let p = settings_position(panel, SETTINGS, MONITOR);
        assert_eq!(p, pos2(panel.right() - SETTINGS.x, panel.top() - 10.0 - SETTINGS.y));
    }

    #[test]
    fn settings_go_below_a_high_overlay() {
        let panel = panel_at(620.0, 100.0);
        assert_eq!(
            settings_position(panel, SETTINGS, MONITOR),
            pos2(panel.right() - SETTINGS.x, panel.bottom() + 10.0)
        );
    }

    #[test]
    fn settings_go_beside_a_tall_overlay() {
        let tall = Rect::from_min_size(pos2(100.0, 300.0), vec2(680.0, 600.0));
        assert_eq!(settings_position(tall, SETTINGS, MONITOR), pos2(tall.right() + 10.0, 300.0));

        let right = Rect::from_min_size(pos2(1200.0, 300.0), vec2(680.0, 600.0));
        assert_eq!(settings_position(right, SETTINGS, MONITOR), pos2(1200.0 - 10.0 - SETTINGS.x, 300.0));
    }

    #[test]
    fn settings_stay_on_the_monitor() {
        let left = panel_at(-500.0, 846.0);
        assert_eq!(settings_position(left, SETTINGS, MONITOR).x, 8.0);
    }
}
