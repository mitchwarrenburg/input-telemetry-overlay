//! Windows integration: tray icon, global hotkey, file dialog, window styles and
//! screen capture.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui::{self, Color32, ColorImage};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::HWND;

/// What the tray, the hotkey or the file dialog asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopEvent {
    OpenSettings,
    /// The tray's "Lock overlay" item, already flipped.
    SetLocked(bool),
    /// The lock/unlock shortcut.
    ToggleLock,
    ResetPosition,
    Quit,
    /// The file dialog closed; `None` when cancelled.
    Picked(Option<PathBuf>),
}

/// Raw events from OS callbacks and worker threads.
enum Raw {
    Menu(MenuId),
    TrayClick,
    Hotkey(u32),
    Picked(Option<PathBuf>),
}

struct Tray {
    _icon: TrayIcon,
    settings: MenuId,
    lock: CheckMenuItem,
    reset: MenuId,
    quit: MenuId,
}

/// The tray icon and the lock/unlock hotkey. Their events wake egui and are read with
/// [`Desktop::poll`] (from `App::logic`, which runs even while the window is hidden).
pub struct Desktop {
    ctx: egui::Context,
    tx: Sender<Raw>,
    rx: Receiver<Raw>,
    tray: Option<Tray>,
    hotkeys: Option<GlobalHotKeyManager>,
    hotkey: Option<HotKey>,
}

impl Desktop {
    /// Creates the tray icon (from RGBA pixels) and the hotkey manager. Call on the main
    /// thread once the event loop runs, i.e. from `App::new`. Failures are logged; the
    /// overlay works without either.
    pub fn new(ctx: &egui::Context, icon: Option<(Vec<u8>, u32, u32)>, locked: bool) -> Self {
        let (tx, rx) = channel();
        forward_events(ctx, &tx);
        let tray = icon
            .and_then(|(rgba, w, h)| Icon::from_rgba(rgba, w, h).map_err(|e| log::warn!("Tray icon image: {e}")).ok())
            .and_then(|icon| build_tray(icon, locked).map_err(|e| log::warn!("No tray icon: {e}")).ok());
        let hotkeys = GlobalHotKeyManager::new().map_err(|e| log::warn!("No global hotkeys: {e}")).ok();
        Self { ctx: ctx.clone(), tx, rx, tray, hotkeys, hotkey: None }
    }

    /// Registers the lock/unlock shortcut (e.g. `Ctrl+Shift+O`), replacing the previous
    /// one. The error is written for the settings window.
    pub fn set_hotkey(&mut self, spec: &str) -> Result<(), String> {
        let result = self.register_hotkey(spec);
        match &result {
            Ok(()) => log::info!("Lock/unlock shortcut: {spec}"),
            Err(e) => log::warn!("{e}"),
        }
        result
    }

    fn register_hotkey(&mut self, spec: &str) -> Result<(), String> {
        let Some(manager) = &self.hotkeys else {
            return Err("Global shortcuts aren't available. Unlock the overlay from the tray icon.".into());
        };
        if let Some(old) = self.hotkey.take() {
            let _ = manager.unregister(old);
        }
        let hotkey: HotKey = spec.parse().map_err(|_| format!("“{spec}” isn't a shortcut the overlay understands."))?;
        manager.register(hotkey).map_err(|e| match e {
            global_hotkey::Error::AlreadyRegistered(_) => {
                format!("Another app already uses {spec}, so it can't lock the overlay. Use the tray icon instead.")
            }
            e => format!("Couldn't set up {spec} ({e}). Use the tray icon to lock the overlay."),
        })?;
        self.hotkey = Some(hotkey);
        Ok(())
    }

    /// Syncs the tray's "Lock overlay" check mark.
    pub fn set_locked(&self, locked: bool) {
        if let Some(tray) = &self.tray {
            tray.lock.set_checked(locked);
        }
    }

    /// There's a way to unlock a click-through overlay (tray or hotkey).
    pub fn can_unlock(&self) -> bool {
        self.tray.is_some() || self.hotkey.is_some()
    }

    /// Events since the last call, oldest first.
    pub fn poll(&self) -> Vec<DesktopEvent> {
        self.rx.try_iter().filter_map(|raw| self.translate(raw)).collect()
    }

    fn translate(&self, raw: Raw) -> Option<DesktopEvent> {
        match raw {
            Raw::Menu(id) => {
                let tray = self.tray.as_ref()?;
                if id == tray.settings {
                    Some(DesktopEvent::OpenSettings)
                } else if id == *tray.lock.id() {
                    // Windows flips the check mark before the event arrives.
                    Some(DesktopEvent::SetLocked(tray.lock.is_checked()))
                } else if id == tray.reset {
                    Some(DesktopEvent::ResetPosition)
                } else if id == tray.quit {
                    Some(DesktopEvent::Quit)
                } else {
                    None
                }
            }
            Raw::TrayClick => Some(DesktopEvent::OpenSettings),
            Raw::Hotkey(id) => (self.hotkey.map(|h| h.id()) == Some(id)).then_some(DesktopEvent::ToggleLock),
            Raw::Picked(path) => Some(DesktopEvent::Picked(path)),
        }
    }

    /// Opens the "choose a Garage 61 lap" dialog on a worker thread (the overlay keeps
    /// drawing); the result arrives as [`DesktopEvent::Picked`].
    pub fn pick_csv<W: HasWindowHandle + HasDisplayHandle>(&self, parent: &W) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Choose a Garage 61 lap")
            .add_filter("Garage 61 lap (CSV)", &["csv"])
            .set_parent(parent);
        if let Some(downloads) = dirs::download_dir() {
            dialog = dialog.set_directory(downloads);
        }
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Raw::Picked(dialog.pick_file()));
            ctx.request_repaint();
        });
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
        GlobalHotKeyEvent::set_event_handler(None::<fn(GlobalHotKeyEvent)>);
    }
}

/// Routes the crates' global event handlers into our channel and wakes egui. (With a
/// handler set, the crates' own receivers get nothing.)
fn forward_events(ctx: &egui::Context, tx: &Sender<Raw>) {
    let wake = |tx: &Sender<Raw>| {
        let (tx, ctx) = (tx.clone(), ctx.clone());
        move |raw: Raw| {
            let _ = tx.send(raw);
            ctx.request_repaint();
        }
    };
    let send = wake(tx);
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| send(Raw::Menu(e.id))));
    let send = wake(tx);
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        // Move/Enter/Leave arrive on every mouse move over the icon.
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            send(Raw::TrayClick);
        }
    }));
    let send = wake(tx);
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        // Windows reports the release too.
        if e.state() == HotKeyState::Pressed {
            send(Raw::Hotkey(e.id()));
        }
    }));
}

fn build_tray(icon: Icon, locked: bool) -> Result<Tray, Box<dyn std::error::Error>> {
    let settings = MenuItem::new("Settings…", true, None);
    let lock = CheckMenuItem::new("Lock overlay", true, locked, None);
    let reset = MenuItem::new("Reset position", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append_items(&[&settings, &lock, &reset, &PredefinedMenuItem::separator(), &quit])?;
    let icon = TrayIconBuilder::new()
        .with_tooltip("Input Telemetry Overlay")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?;
    Ok(Tray { _icon: icon, settings: settings.id().clone(), lock, reset: reset.id().clone(), quit: quit.id().clone() })
}

/// The Win32 handle behind a window.
fn hwnd(window: &impl HasWindowHandle) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as HWND),
        _ => None,
    }
}

/// Keeps the overlay from ever taking focus from the sim, and out of Alt-Tab (a tool
/// window, not an app window). winit rewrites the extended style whenever
/// click-through or visibility changes, so call this every frame; it only writes when
/// the style is off.
pub fn keep_no_activate(window: &impl HasWindowHandle) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    let Some(hwnd) = hwnd(window) else { return };
    // SAFETY: style reads and writes on a live window handle.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let wanted = (style | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize) & !(WS_EX_APPWINDOW as isize);
        if style != wanted {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted);
        }
    }
}

/// Re-enables DWM blur-behind with an empty region, which keeps transparent OpenGL
/// windows transparent on AMD drivers (egui#4451).
pub fn fix_transparency(window: &impl HasWindowHandle) {
    use windows_sys::Win32::Graphics::Dwm::{DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DwmEnableBlurBehindWindow};
    use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject};
    let Some(hwnd) = hwnd(window) else { return };
    // SAFETY: the region is created, handed to DWM (which copies it) and freed here.
    unsafe {
        let region = CreateRectRgn(0, 0, -1, -1);
        let blur = DWM_BLURBEHIND { dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION, fEnable: 1, hRgnBlur: region, fTransitionOnMaximized: 0 };
        let hr = DwmEnableBlurBehindWindow(hwnd, &blur);
        if hr < 0 {
            log::warn!("DwmEnableBlurBehindWindow failed: {hr:#x}");
        }
        DeleteObject(region);
    }
}

/// Copies a screen rectangle (physical pixels) as the desktop composes it. Used for
/// the settings window screenshot: eframe's glow backend doesn't take screenshots of
/// immediate viewports.
pub fn capture_screen(x: i32, y: i32, w: i32, h: i32) -> Option<ColorImage> {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    };
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    // SAFETY: standard GDI capture; every handle is released before returning and the
    // buffer holds exactly w × h 32-bit pixels.
    let lines = unsafe {
        let screen = GetDC(std::ptr::null_mut());
        let mem = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bitmap);
        let copied = BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT) != 0;
        SelectObject(mem, old);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let lines =
            if copied { GetDIBits(mem, bitmap, 0, h as u32, bgra.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS) } else { 0 };
        DeleteObject(bitmap);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);
        lines
    };
    if lines != h {
        return None;
    }
    let pixels = bgra.as_chunks::<4>().0.iter().map(|&[b, g, r, _]| Color32::from_rgb(r, g, b)).collect();
    Some(ColorImage::new([w as usize, h as usize], pixels))
}
