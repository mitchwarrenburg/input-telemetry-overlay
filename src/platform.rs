//! Windows integration: tray icon, global hotkey, file dialog, window styles, screen
//! capture, and one overlay per data folder.

use std::collections::HashSet;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, ColorImage};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{HANDLE, HWND};

/// What the tray, the hotkey, the file dialog or another launch asked for.
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
    /// Another launch of the program ran while this one does: the laps it was given (a
    /// CSV dropped on the exe, "Open with"), or none when it only asked for the settings.
    HandOver(HandOver),
}

/// One launch's hand-over. Its inbox file stays until [`HandOver::done`], so laps
/// handed over while this overlay quits are taken at its next start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandOver {
    pub paths: Vec<PathBuf>,
    receipt: PathBuf,
}

impl HandOver {
    /// The paths were dealt with: forget the hand-over.
    pub fn done(self) {
        if let Err(e) = std::fs::remove_file(&self.receipt) {
            log::warn!("Couldn't remove {}: {e}", self.receipt.display());
        }
    }
}

/// Raw events from OS callbacks and worker threads.
enum Raw {
    Menu(MenuId),
    TrayClick,
    Hotkey(u32),
    Picked(Option<PathBuf>),
    HandOver(HandOver),
}

struct Tray {
    _icon: TrayIcon,
    settings: MenuId,
    lock: CheckMenuItem,
    reset: MenuId,
    quit: MenuId,
}

/// The tray icon, the lock/unlock hotkey and hand-overs from other launches. Their
/// events wake egui and are read with [`Desktop::poll`] (from `App::logic`, which runs
/// even while the window is hidden).
pub struct Desktop {
    ctx: egui::Context,
    tx: Sender<Raw>,
    rx: Receiver<Raw>,
    tray: Option<Tray>,
    hotkeys: Option<GlobalHotKeyManager>,
    hotkey: Option<HotKey>,
    /// [`Instance`]'s mutex, held while the overlay runs.
    _instance: Option<OwnedHandle>,
}

impl Desktop {
    /// Creates the tray icon (from RGBA pixels) and the hotkey manager, and listens for
    /// other launches handing over to `instance`. Call on the main thread once the event
    /// loop runs, i.e. from `App::new`. Failures are logged; the overlay works without
    /// any of them.
    pub fn new(
        ctx: &egui::Context,
        icon: Option<(Vec<u8>, u32, u32)>,
        locked: bool,
        instance: Option<Instance>,
    ) -> Self {
        let (tx, rx) = channel();
        forward_events(ctx, &tx);
        let instance = instance.map(|instance| instance.listen(ctx, &tx));
        let tray = icon
            .and_then(|(rgba, w, h)| Icon::from_rgba(rgba, w, h).map_err(|e| log::warn!("Tray icon image: {e}")).ok())
            .and_then(|icon| build_tray(icon, locked).map_err(|e| log::warn!("No tray icon: {e}")).ok());
        let hotkeys = GlobalHotKeyManager::new().map_err(|e| log::warn!("No global hotkeys: {e}")).ok();
        Self { ctx: ctx.clone(), tx, rx, tray, hotkeys, hotkey: None, _instance: instance }
    }

    /// Registers the lock/unlock shortcut (e.g. `Ctrl+Alt+Shift+O`), replacing the previous
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
                format!("Another app already uses {spec}. Pick another shortcut, or use the tray icon.")
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
            Raw::HandOver(h) => Some(DesktopEvent::HandOver(h)),
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

// ---- One overlay per data folder ----

/// Hand-overs wait in `<data dir>\inbox`, one file per launch, one path per line.
const INBOX: &str = "inbox";

/// This process as the overlay for a data folder: a named mutex held while it runs,
/// and the event other launches of the program signal after leaving their CSVs in the
/// data folder's inbox. (Two overlays on one folder would each save their own copy of
/// the lap library over the other's.)
pub struct Instance {
    mutex: OwnedHandle,
    wake: OwnedHandle,
    inbox: PathBuf,
}

impl Instance {
    /// Makes this process the overlay for `data_dir`. `Ok(None)`: one already runs
    /// there; give it this launch's files with [`hand_over`].
    pub fn claim(data_dir: &Path) -> io::Result<Option<Self>> {
        use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        std::fs::create_dir_all(data_dir)?;
        let name = wide(&instance_name(data_dir));
        // SAFETY: a NUL-terminated name and default security; the error is read before
        // anything else can change it.
        let (mutex, error) = unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            (mutex, GetLastError())
        };
        // Access denied: it exists, made by an overlay running as administrator.
        if mutex.is_null() && error == ERROR_ACCESS_DENIED {
            return Ok(None);
        }
        let mutex = owned(mutex)?;
        if error == ERROR_ALREADY_EXISTS {
            return Ok(None);
        }
        Ok(Some(Self { mutex, wake: wake_event(data_dir)?, inbox: data_dir.join(INBOX) }))
    }

    /// Sends hand-overs to `tx` (waking egui) from a thread that lives as long as the
    /// process: those already waiting, then each one signalled. Returns the mutex, to
    /// hold while the overlay runs.
    fn listen(self, ctx: &egui::Context, tx: &Sender<Raw>) -> OwnedHandle {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};
        let Self { mutex, wake, inbox } = self;
        let (ctx, tx) = (ctx.clone(), tx.clone());
        let spawned = std::thread::Builder::new().name("hand-over".into()).spawn(move || {
            let mut seen = HashSet::new();
            loop {
                for h in read_hand_overs(&inbox, &mut seen) {
                    log::info!("Another launch handed over {:?}", h.paths);
                    let _ = tx.send(Raw::HandOver(h));
                    ctx.request_repaint();
                }
                // SAFETY: waits on an event handle this thread owns.
                if unsafe { WaitForSingleObject(wake.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
                    log::warn!("Stopped listening for other launches: {}", io::Error::last_os_error());
                    return;
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("Can't take files from other launches: {e}");
        }
        mutex
    }
}

/// Gives the overlay running for `data_dir` this launch's CSVs, or with none, asks it
/// to show its settings.
pub fn hand_over(data_dir: &Path, paths: &[PathBuf]) -> io::Result<()> {
    use windows_sys::Win32::System::Threading::SetEvent;
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    // Relative paths mean this launch's folder, which the running overlay doesn't share.
    let paths: Vec<PathBuf> = paths.iter().map(std::path::absolute).collect::<io::Result<_>>()?;
    write_hand_over(&data_dir.join(INBOX), &paths)?;
    let wake = wake_event(data_dir)?;
    // SAFETY: plain calls; the event handle is owned here.
    unsafe {
        // It opens its settings window to show the result: let that come to the front.
        AllowSetForegroundWindow(ASFW_ANY);
        if SetEvent(wake.as_raw_handle()) == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Writes one hand-over (UTF-8, a path per line) whole, then renames it into place.
/// Named by the time, so they're taken in order.
fn write_hand_over(inbox: &Path, paths: &[PathBuf]) -> io::Result<()> {
    let text: String = paths.iter().map(|p| format!("{}\n", p.to_string_lossy())).collect();
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let file = inbox.join(format!("{nanos:020}-{}.txt", std::process::id()));
    crate::settings::write_atomic(&file, text.as_bytes())
}

/// The hand-overs waiting in `inbox` that aren't in `seen` yet, oldest first; each is
/// added to `seen`.
fn read_hand_overs(inbox: &Path, seen: &mut HashSet<PathBuf>) -> Vec<HandOver> {
    let Ok(entries) = std::fs::read_dir(inbox) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "txt") && !seen.contains(p))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|file| {
            seen.insert(file.clone());
            let text = std::fs::read_to_string(&file).map_err(|e| log::warn!("Couldn't read {}: {e}", file.display()));
            let paths = text.ok()?.lines().filter(|line| !line.trim().is_empty()).map(PathBuf::from).collect();
            Some(HandOver { paths, receipt: file })
        })
        .collect()
}

/// `Local\input-telemetry-overlay-<hash of the folder>`: however the folder is spelled
/// (relative, another case), the same folder gets the same name.
fn instance_name(data_dir: &Path) -> String {
    let dir = std::fs::canonicalize(data_dir)
        .or_else(|_| std::path::absolute(data_dir))
        .unwrap_or_else(|_| data_dir.to_path_buf());
    let key = dir.to_string_lossy().to_lowercase();
    // FNV-1a, 64-bit.
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("Local\\input-telemetry-overlay-{hash:016x}")
}

/// The auto-reset event that wakes the overlay for `data_dir` (created by whichever
/// side gets there first).
fn wake_event(data_dir: &Path) -> io::Result<OwnedHandle> {
    use windows_sys::Win32::System::Threading::CreateEventW;
    let name = wide(&format!("{}-wake", instance_name(data_dir)));
    // SAFETY: a NUL-terminated name and default security.
    owned(unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) })
}

/// Takes ownership of a handle a Win32 call returned (null: it failed).
fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a fresh handle nothing else owns or closes.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// NUL-terminated UTF-16.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
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
    if let Some(hwnd) = hwnd(window) {
        no_activate(hwnd);
    }
}

fn no_activate(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
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
    if let Some(hwnd) = hwnd(window) {
        blur_behind(hwnd);
    }
}

fn blur_behind(hwnd: HWND) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DwmEnableBlurBehindWindow,
    };
    use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject};
    // SAFETY: the region is created, handed to DWM (which copies it) and freed here.
    unsafe {
        let region = CreateRectRgn(0, 0, -1, -1);
        let blur = DWM_BLURBEHIND {
            dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
            fEnable: 1,
            hRgnBlur: region,
            fTransitionOnMaximized: 0,
        };
        let hr = DwmEnableBlurBehindWindow(hwnd, &blur);
        if hr < 0 {
            log::warn!("DwmEnableBlurBehindWindow failed: {hr:#x}");
        }
        DeleteObject(region);
    }
}

/// A window of this program that eframe made for a secondary viewport, which egui gives
/// no handle to: found by its title among this thread's windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeWindow(HWND);

impl NativeWindow {
    /// This thread's window titled `title`.
    pub fn find(title: &str) -> Option<Self> {
        use windows_sys::Win32::Foundation::LPARAM;
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, GetWindowTextW};
        struct Search {
            title: Vec<u16>,
            found: HWND,
        }
        unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> i32 {
            // SAFETY: `lparam` is the `Search` below, alive for the whole enumeration.
            let search = unsafe { &mut *(lparam as *mut Search) };
            let mut buf = [0u16; 128];
            // SAFETY: the buffer's length is passed.
            let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
            if buf[..n.max(0) as usize] == search.title[..] {
                search.found = hwnd;
                return 0;
            }
            1
        }
        let mut search = Search { title: title.encode_utf16().collect(), found: std::ptr::null_mut() };
        // SAFETY: the callback only reads window titles and writes to `search`.
        unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(visit), &raw mut search as LPARAM) };
        (!search.found.is_null()).then_some(Self(search.found))
    }

    /// The window still exists (egui destroys a viewport's window when it isn't shown).
    pub fn alive(self) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
        // SAFETY: IsWindow accepts any value.
        unsafe { IsWindow(self.0) != 0 }
    }

    /// As [`keep_no_activate`]: call every frame.
    pub fn keep_no_activate(self) {
        no_activate(self.0);
    }

    /// As [`fix_transparency`].
    pub fn fix_transparency(self) {
        blur_behind(self.0);
    }

    /// Takes away the frame winit gives an undecorated window for a drop shadow, which
    /// egui asks for and has no way to turn off for a secondary window: a 1 pt strip of
    /// non-client area, around which Windows 11 draws a border and a shadow. The window's
    /// procedure is wrapped to answer `WM_NCCALCSIZE` with the whole window as client
    /// area, which is what winit does without the shadow. Once per window; later calls do
    /// nothing.
    pub fn remove_frame(self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWLP_WNDPROC, GetPropW, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SWP_NOZORDER, SetPropW, SetWindowLongPtrW, SetWindowPos,
        };
        // SAFETY: `frameless_proc` lives as long as the program, and passes everything but
        // the one message on to the procedure it replaced, which it keeps in a property of
        // the window (set before it takes over) until the window is destroyed.
        unsafe {
            if !GetPropW(self.0, ORIGINAL_PROC).is_null() {
                return;
            }
            let original = GetWindowLongPtrW(self.0, GWLP_WNDPROC);
            if original == 0 || SetPropW(self.0, ORIGINAL_PROC, original as HANDLE) == 0 {
                log::warn!("Couldn't take the frame off a window: {}", io::Error::last_os_error());
                return;
            }
            SetWindowLongPtrW(self.0, GWLP_WNDPROC, frameless_proc as *const () as isize);
            // Have the frame worked out again, with the new procedure answering.
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
            SetWindowPos(self.0, std::ptr::null_mut(), 0, 0, 0, 0, flags);
        }
    }

    /// Where the window is, physical pixels.
    pub fn outer_px(self) -> Option<egui::Rect> {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: writes into `r`.
        let ok = unsafe { GetWindowRect(self.0, &mut r) } != 0;
        ok.then(|| {
            egui::Rect::from_min_max(
                egui::pos2(r.left as f32, r.top as f32),
                egui::pos2(r.right as f32, r.bottom as f32),
            )
        })
    }

    /// Moves the window's top-left corner to `x`, `y` physical pixels, keeping its size.
    pub fn move_to_px(self, x: i32, y: i32) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
        };
        let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE;
        // SAFETY: a plain call on a window handle.
        unsafe { SetWindowPos(self.0, std::ptr::null_mut(), x, y, 0, 0, flags) };
    }

    /// Starts moving the window with the mouse, like a press on a title bar. (egui's
    /// `StartDrag` wants focus, which the overlay's windows never take.)
    pub fn start_move(self) {
        self.start_sizing_loop(windows_sys::Win32::UI::WindowsAndMessaging::HTCAPTION);
    }

    /// Starts resizing the window from an edge or corner with the mouse.
    pub fn start_resize(self, dir: egui::ResizeDirection) {
        use egui::ResizeDirection::*;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
        };
        self.start_sizing_loop(match dir {
            North => HTTOP,
            South => HTBOTTOM,
            East => HTRIGHT,
            West => HTLEFT,
            NorthEast => HTTOPRIGHT,
            NorthWest => HTTOPLEFT,
            SouthEast => HTBOTTOMRIGHT,
            SouthWest => HTBOTTOMLEFT,
        });
    }

    /// What winit's `drag_window` does: a non-client press at the cursor on `hit`.
    fn start_sizing_loop(self, hit: u32) {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, PostMessageW, WM_NCLBUTTONDOWN};
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: plain calls; `p` outlives them.
        unsafe {
            GetCursorPos(&mut p);
            ReleaseCapture();
            let points = ((p.y as u32 & 0xffff) << 16) | (p.x as u32 & 0xffff);
            PostMessageW(self.0, WM_NCLBUTTONDOWN, hit as usize, points as isize);
        }
    }
}

/// The window property holding the procedure [`NativeWindow::remove_frame`] replaced.
const ORIGINAL_PROC: windows_sys::core::PCWSTR = windows_sys::core::w!("ito-original-wndproc");

/// The procedure of a window whose frame [`NativeWindow::remove_frame`] took away.
unsafe extern "system" fn frameless_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, GWLP_WNDPROC, GetPropW, RemovePropW, SetWindowLongPtrW, WM_NCCALCSIZE,
        WM_NCDESTROY, WNDPROC,
    };
    // SAFETY: the property holds the procedure this one replaced, a valid WNDPROC (it was
    // set before this one took over); it's put back as the window goes.
    unsafe {
        // The whole window is client area: no frame to draw a border and shadow around.
        if msg == WM_NCCALCSIZE && wparam != 0 {
            return 0;
        }
        let original = GetPropW(hwnd, ORIGINAL_PROC) as isize;
        if original == 0 {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        if msg == WM_NCDESTROY {
            RemovePropW(hwnd, ORIGINAL_PROC);
            SetWindowLongPtrW(hwnd, GWLP_WNDPROC, original);
        }
        let original = std::mem::transmute::<isize, WNDPROC>(original);
        CallWindowProcW(original, hwnd, msg, wparam, lparam)
    }
}

/// A beep through the speakers, on a worker thread (the call blocks while it plays).
pub fn beep(hz: u32, ms: u32) {
    use windows_sys::Win32::System::Diagnostics::Debug::Beep;
    // SAFETY: plain call.
    let spawned = std::thread::Builder::new().name("beep".into()).spawn(move || unsafe { Beep(hz, ms) });
    if let Err(e) = spawned {
        log::warn!("Couldn't beep: {e}");
    }
}

/// Makes `println!` reach whoever started the program, although a GUI-subsystem build
/// has no console of its own: its output as redirected (to a file or a pipe), or else
/// the console it was started from. False when there's neither.
pub fn attach_parent_console() -> bool {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    // SAFETY: no pointers; both fail harmlessly.
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        // Attaching would replace a redirection with the console.
        if !out.is_null() && out != INVALID_HANDLE_VALUE {
            return true;
        }
        AttachConsole(ATTACH_PARENT_PROCESS) != 0
    }
}

/// A modal message box with an OK button.
pub fn message_box(title: &str, text: &str, error: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW};
    let (title, text) = (wide(title), wide(text));
    let icon = if error { MB_ICONERROR } else { MB_ICONINFORMATION };
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | icon) };
}

/// Copies a screen rectangle (physical pixels) as the desktop composes it. Used for
/// the settings window screenshot: eframe's glow backend doesn't take screenshots of
/// immediate viewports.
pub fn capture_screen(x: i32, y: i32, w: i32, h: i32) -> Option<ColorImage> {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
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
        let lines = if copied {
            GetDIBits(mem, bitmap, 0, h as u32, bgra.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS)
        } else {
            0
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_has_one_instance_name_however_it_is_spelled() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let name = instance_name(dir.path());
        assert!(name.starts_with("Local\\input-telemetry-overlay-") && name.len() == 30 + 16, "{name}");
        let upper = PathBuf::from(dir.path().to_string_lossy().to_uppercase());
        assert_eq!(instance_name(&upper), name);
        assert_eq!(instance_name(&dir.path().join("sub").join("..")), name);
        assert_ne!(instance_name(&dir.path().join("sub")), name);
    }

    #[test]
    fn hand_overs_are_taken_in_order_once_and_kept_until_done() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join(INBOX);
        let paths = |hs: &[HandOver]| hs.iter().map(|h| h.paths.clone()).collect::<Vec<_>>();
        let mut seen = HashSet::new();
        assert!(read_hand_overs(&inbox, &mut seen).is_empty(), "no inbox yet");
        let laps = [PathBuf::from("C:/laps/Spa 2.17.4.csv"), PathBuf::from("D:/b.csv")];
        write_hand_over(&inbox, &laps).unwrap();
        write_hand_over(&inbox, &[]).unwrap();
        let taken = read_hand_overs(&inbox, &mut seen);
        assert_eq!(paths(&taken), vec![laps.to_vec(), Vec::new()]);
        assert!(read_hand_overs(&inbox, &mut seen).is_empty(), "each is sent once");

        // Until done, they're still there for the next start (a fresh `seen`).
        assert_eq!(read_hand_overs(&inbox, &mut HashSet::new()).len(), 2);
        taken.into_iter().for_each(HandOver::done);
        assert!(read_hand_overs(&inbox, &mut HashSet::new()).is_empty());
    }

    #[test]
    fn a_second_launch_finds_the_first_and_wakes_it() {
        use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        let dir = tempfile::tempdir().unwrap();
        let first = Instance::claim(dir.path()).unwrap().expect("nothing runs there yet");
        assert!(Instance::claim(dir.path()).unwrap().is_none(), "second launch");
        // SAFETY: polls an event handle owned by `first`.
        let signalled = || unsafe { WaitForSingleObject(first.wake.as_raw_handle(), 0) };
        assert_eq!(signalled(), WAIT_TIMEOUT);

        hand_over(dir.path(), &[PathBuf::from("lap.csv")]).unwrap();
        assert_eq!(signalled(), WAIT_OBJECT_0);
        assert_eq!(signalled(), WAIT_TIMEOUT, "auto-reset");
        let expected = std::env::current_dir().unwrap().join("lap.csv");
        let taken = read_hand_overs(&first.inbox, &mut HashSet::new());
        assert_eq!(taken.iter().map(|h| h.paths.clone()).collect::<Vec<_>>(), vec![vec![expected]], "made absolute");

        drop(first);
        assert!(Instance::claim(dir.path()).unwrap().is_some(), "free again once it quits");
    }
}
