//! The strip window: geometry, click-through, Ctrl-to-drag, capture exclusion.

use std::cell::RefCell;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, PoisonError};
use std::thread;

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK,
    RIDEV_REMOVE, RID_INPUT, RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, GetWindowLongPtrW, PostMessageW,
    PostQuitMessage, RegisterClassW, SetWindowDisplayAffinity, SetWindowLongPtrW, GWL_EXSTYLE, HWND_MESSAGE, MSG,
    RI_KEY_BREAK, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_INPUT,
    WNDCLASSW, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use crate::settings::{Settings, Store};

// ---- Geometry (pure) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

/// How much of the strip must stay on a monitor, in physical pixels.
const MIN_VISIBLE: i32 = 80;

/// Window height in physical pixels for `font_px` CSS pixels: prev + current + next at two visual
/// lines each, the plate padding and the toast row, with the line heights and paddings of overlay.css.
pub fn strip_height(font_px: u32, scale: f64) -> i32 {
    // Rows: current 2 lines of 1.3em plus padding, prev and next at 0.7em, plate padding .6em, toast 1.12em.
    ((8.2 * f64::from(font_px) + 6.0) * scale).ceil() as i32
}

/// Default top-left and size of the strip from the monitor work area (physical px) and scale factor.
pub fn default_rect(work: Rect, scale: f64, font_px: u32) -> Rect {
    let w = (f64::from(work.w) * 0.6).round() as i32;
    let h = strip_height(font_px, scale);
    let bottom = work.bottom() - (8.0 * scale).round() as i32;
    Rect::new(work.x + (work.w - w) / 2, bottom - h, w, h)
}

/// Clamps a saved position so at least 80 px of the strip stays on some monitor.
pub fn clamp_to_monitors(pos: (i32, i32), size: (i32, i32), monitors: &[Rect]) -> (i32, i32) {
    let strip = Rect::new(pos.0, pos.1, size.0, size.1);
    let overlap = |a: i32, a_len: i32, b: i32, b_len: i32| (a + a_len).min(b + b_len) - a.max(b);
    let visible = |m: &Rect| {
        overlap(strip.x, strip.w, m.x, m.w) >= MIN_VISIBLE.min(strip.w)
            && overlap(strip.y, strip.h, m.y, m.h) >= MIN_VISIBLE.min(strip.h)
    };
    if monitors.iter().any(visible) {
        return pos;
    }
    // Not reachable on any monitor: the whole strip goes onto the nearest one.
    let centre = |r: &Rect| (i64::from(r.x) * 2 + i64::from(r.w), i64::from(r.y) * 2 + i64::from(r.h));
    let (sx, sy) = centre(&strip);
    let Some(m) = monitors.iter().min_by_key(|m| {
        let (mx, my) = centre(m);
        (mx - sx).pow(2) + (my - sy).pow(2)
    }) else {
        return pos;
    };
    (pos.0.clamp(m.x, (m.right() - strip.w).max(m.x)), pos.1.clamp(m.y, (m.bottom() - strip.h).max(m.y)))
}

// ---- The window ----

static PLACED: AtomicBool = AtomicBool::new(false);
/// Whether the strip is to be left out of captures; kept so it can be applied once the window is shown.
static EXCLUDE: AtomicBool = AtomicBool::new(true);
static SET_CTRL: Mutex<()> = Mutex::new(());
static CTRL: AtomicBool = AtomicBool::new(false);
/// Where the strip was when Ctrl went down, to tell whether it was dragged.
static DRAG_FROM: Mutex<Option<(i32, i32)>> = Mutex::new(None);

fn overlay_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("overlay")
}

fn store(app: &AppHandle) -> Option<Store> {
    app.path().app_local_data_dir().ok().map(Store::new)
}

fn rect_of(position: &PhysicalPosition<i32>, size: &PhysicalSize<u32>) -> Rect {
    Rect::new(position.x, position.y, size.width as i32, size.height as i32)
}

/// Size, position and capture exclusion. The first call also places the strip (the saved position,
/// kept on a monitor, or the default); later calls keep it where it is and its bottom edge fixed
/// when the height changes. Safe to call again whenever settings or the display change.
pub fn apply(app: &AppHandle, settings: &Settings) {
    let Some(window) = overlay_window(app) else { return };
    let first = !PLACED.swap(true, Ordering::SeqCst);
    let current = match (window.outer_position(), window.outer_size()) {
        (Ok(p), Ok(s)) => Some(rect_of(&p, &s)),
        _ => None,
    };
    let monitors = app.available_monitors().unwrap_or_default();
    let screens: Vec<Rect> = monitors.iter().map(|m| rect_of(m.position(), m.size())).collect();

    // The monitor that holds the strip (on the first call: where it was saved), else the primary one.
    let probe = if first {
        settings.position.map(|(x, y)| (x + 40, y + 40))
    } else {
        current.map(|r| (r.x + r.w / 2, r.y + r.h / 2))
    };
    let holds = |m: &&tauri::Monitor| {
        probe.is_some_and(|(x, y)| {
            let r = rect_of(m.position(), m.size());
            x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
        })
    };
    let primary = app.primary_monitor().ok().flatten();
    let Some(monitor) = monitors.iter().find(holds).or(primary.as_ref()).or(monitors.first()) else { return };

    let work = rect_of(&monitor.work_area().position, &monitor.work_area().size);
    let want = default_rect(work, monitor.scale_factor(), settings.font_px);
    let size = (want.w, want.h);
    let pos = match (first, current, settings.position) {
        (true, _, Some(saved)) => clamp_to_monitors(saved, size, &screens),
        (true, _, None) | (false, None, _) => (want.x, want.y),
        // Same left edge and same bottom edge, whatever the new height.
        (false, Some(c), _) => clamp_to_monitors((c.x, c.bottom() - want.h), size, &screens),
    };

    EXCLUDE.store(settings.hide_from_capture, Ordering::SeqCst);
    set_affinity(&window);
    if first {
        click_through(&window, true);
    }
    if current.map(|c| (c.w, c.h)) != Some(size) {
        let _ = window.set_size(PhysicalSize::new(size.0 as u32, size.1 as u32));
    }
    if current.map(|c| (c.x, c.y)) != Some(pos) {
        let _ = window.set_position(PhysicalPosition::new(pos.0, pos.1));
        // A saved position follows the bottom edge when the height changes.
        if !first && settings.position.is_some() {
            save_position(app, pos);
        }
    }

    if !first {
        let _ = app.emit_to("overlay", "style", serde_json::json!({ "font_px": settings.font_px }));
    }
}

/// Runs `f` on the main thread, after anything already queued for it. The window library applies
/// `show`, `hide` and `set_ignore_cursor_events` from another thread through a queue, and rebuilds
/// the extended styles when it does; work that must see the result queues up behind them here.
fn on_main(window: &WebviewWindow, f: impl FnOnce(&WebviewWindow) + Send + 'static) {
    let w = window.clone();
    let _ = window.run_on_main_thread(move || f(&w));
}

/// Keeps the strip out of screen sharing and recordings (spec B8), or lets it in. Windows refuses to
/// change this on a layered window, which the window library makes for click-through, so a failed
/// call is retried with click-through briefly off. Still failing (Windows before 10 2004): ignored,
/// the strip is then simply captured.
fn set_affinity(window: &WebviewWindow) {
    on_main(window, |window| {
        let Ok(hwnd) = window.hwnd() else { return };
        let affinity = if EXCLUDE.load(Ordering::SeqCst) { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
        if unsafe { SetWindowDisplayAffinity(hwnd, affinity) }.is_err() {
            // On the main thread these run at once, so the order is the one written here.
            let _ = window.set_ignore_cursor_events(false);
            let _ = unsafe { SetWindowDisplayAffinity(hwnd, affinity) };
            let _ = window.set_ignore_cursor_events(!CTRL.load(Ordering::SeqCst));
            drop_app_window_style(window);
        }
    });
}

/// Click-through on or off. Not taking focus comes from `focusable: false` in tauri.conf.json.
fn click_through(window: &WebviewWindow, on: bool) {
    let _ = window.set_ignore_cursor_events(on);
    keep_out_of_alt_tab(window);
}

/// A tool window without `WS_EX_APPWINDOW` is not listed in Alt+Tab. The window library rebuilds all
/// extended styles whenever one of its flags changes (shown, hidden, click-through), so this is
/// repeated after each of those calls, queued behind them.
fn keep_out_of_alt_tab(window: &WebviewWindow) {
    on_main(window, drop_app_window_style);
}

fn drop_app_window_style(window: &WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let wanted = (ex | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
            if wanted != ex {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted);
            }
        }
    }
}

/// Back to the bottom centre, forgetting the saved position.
pub fn reset_position(app: &AppHandle, store: &Store) {
    let settings = store.update(|s| s.position = None);
    PLACED.store(false, Ordering::SeqCst);
    apply(app, &settings);
}

fn save_position(app: &AppHandle, pos: (i32, i32)) {
    if let Some(store) = store(app) {
        // Saved on top of the file, so a change made by the settings window in the meantime is kept.
        store.update(|s| s.position = Some(pos));
    }
}

/// A restart after a WebView2 failure is skipped when the last one was less than this many seconds ago,
/// so a failure that repeats at start cannot make the app restart in a loop.
const RESTART_GAP_SECS: u64 = 30;
/// File in the app data folder holding the time (Unix seconds) of the last restart after a failure.
const RESTART_MARKER: &str = "restarted-after-failure";
/// Set once the app is on its way out (tray Quit, exit request): WebView2 dying then is not a failure.
static EXITING: AtomicBool = AtomicBool::new(false);

pub fn mark_exiting() {
    EXITING.store(true, Ordering::SeqCst);
}

/// A marker from the future (the clock went back) does not block a restart: it must not turn recovery off for good.
fn restart_allowed(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|t| now < t || now - t >= RESTART_GAP_SECS)
}

/// WebView2 runs in a single process (see `WEBVIEW_ARGS`), so a failure leaves the strip blank for good.
/// When the browser or the renderer process exits, the app logs it and restarts itself, unless it is
/// quitting, or it already restarted less than 30 s ago, or it cannot record the restart: then it only logs.
/// The restart is recorded before it happens, so a failure that repeats can never restart in a loop.
pub fn restart_on_webview_failure(app: &AppHandle, window: &WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
    };
    let app = app.clone();
    let _ = window.with_webview(move |webview| {
        let Ok(core) = (unsafe { webview.controller().CoreWebView2() }) else {
            eprintln!("larik: cannot watch WebView2 for failures, it will not be restarted");
            return;
        };
        let handler = webview2_com::ProcessFailedEventHandler::create(Box::new(move |_, args| {
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            if let Some(args) = args {
                let _ = unsafe { args.ProcessFailedKind(&mut kind) };
            }
            let fatal = kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
                || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED;
            if !fatal {
                eprintln!("larik: WebView2 process failed (kind {}), not restarting for this kind", kind.0);
                return Ok(());
            }
            if EXITING.load(Ordering::SeqCst) {
                return Ok(());
            }
            let Some(marker) = app.path().app_local_data_dir().ok().map(|dir| dir.join(RESTART_MARKER)) else {
                eprintln!("larik: WebView2 process failed (kind {}), no data folder to record a restart", kind.0);
                return Ok(());
            };
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let last = std::fs::read_to_string(&marker).ok().and_then(|text| text.trim().parse::<u64>().ok());
            if !restart_allowed(last, now) {
                eprintln!("larik: WebView2 process failed (kind {}), not restarting again so soon", kind.0);
            } else if std::fs::write(&marker, now.to_string()).is_err() {
                eprintln!("larik: WebView2 process failed (kind {}), cannot record a restart, not restarting", kind.0);
            } else {
                eprintln!("larik: WebView2 process failed (kind {}), restarting", kind.0);
                app.restart();
            }
            Ok(())
        }));
        let mut token = 0;
        if let Err(e) = unsafe { core.add_ProcessFailed(&handler, &mut token) } {
            eprintln!("larik: cannot watch WebView2 for failures, it will not be restarted: {e}");
        }
    });
}

/// Asks WebView2 to keep its memory use low while the strip is hidden and normal while it is shown.
fn memory_level(window: &WebviewWindow, low: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use windows::core::Interface;
    let level =
        if low { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
    let _ = window.with_webview(move |webview| {
        if let Ok(core) = unsafe { webview.controller().CoreWebView2() } {
            if let Ok(core) = core.cast::<ICoreWebView2_19>() {
                let _ = unsafe { core.SetMemoryUsageTargetLevel(level) };
            }
        }
    });
}

/// Shows or hides the strip. The Ctrl watch only exists while it is shown.
pub fn set_visible(app: &AppHandle, visible: bool) {
    let Some(window) = overlay_window(app) else { return };
    memory_level(&window, !visible);
    if visible {
        let _ = window.show();
        keep_out_of_alt_tab(&window);
        set_affinity(&window);
        start_ctrl_watch(app);
        // A key-up missed while the watch was off must not leave the strip clickable.
        set_ctrl(app, unsafe { GetAsyncKeyState(i32::from(VK_CONTROL.0)) } < 0);
    } else {
        stop_ctrl_watch();
        set_ctrl(app, false);
        let _ = window.hide();
        keep_out_of_alt_tab(&window);
    }
}

/// Ctrl down makes the strip clickable and draggable; Ctrl up makes it click-through again and,
/// if it was dragged, remembers where it is.
pub(crate) fn set_ctrl(app: &AppHandle, down: bool) {
    // The raw-input thread and the engine thread both come here: one at a time, start to end.
    let _one_at_a_time = SET_CTRL.lock().unwrap_or_else(PoisonError::into_inner);
    if CTRL.swap(down, Ordering::SeqCst) == down {
        return;
    }
    let Some(window) = overlay_window(app) else { return };
    let at = window.outer_position().ok().map(|p| (p.x, p.y));
    let from = {
        let mut guard = DRAG_FROM.lock().unwrap_or_else(PoisonError::into_inner);
        if down {
            *guard = at;
            None
        } else {
            guard.take()
        }
    };
    click_through(&window, !down);
    let _ = app.emit_to("overlay", "drag", serde_json::json!({ "on": down }));
    if let (Some(from), Some(now)) = (from, at) {
        if from != now {
            save_position(app, now);
        }
    }
}

// ---- Ctrl watch (spec B7) ----

const CLASS: PCWSTR = w!("LarikCtrlWatch");
/// The message-only window of the running watch, as an integer so it can sit in a static.
static WATCH: Mutex<Option<(isize, thread::JoinHandle<()>)>> = Mutex::new(None);

thread_local! {
    static WATCH_APP: RefCell<Option<AppHandle>> = const { RefCell::new(None) };
}

/// Starts watching the Ctrl key: a message-only window on its own thread receives keyboard Raw
/// Input (`RIDEV_INPUTSINK`, so also while another window has focus). No hook and no polling: the
/// thread sleeps in `GetMessageW` until Windows delivers a key event.
///
/// Only the key code is inspected, and only to tell whether it is Ctrl and whether it went down
/// or up. Nothing else in the event is read, stored or logged.
pub fn start_ctrl_watch(app: &AppHandle) {
    let mut watch = WATCH.lock().unwrap_or_else(PoisonError::into_inner);
    if watch.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    let app = app.clone();
    let spawned = thread::Builder::new().name("ctrl-watch".into()).spawn(move || {
        WATCH_APP.with(|a| *a.borrow_mut() = Some(app));
        let Some(hwnd) = (unsafe { create_watch_window() }) else {
            let _ = tx.send(None);
            return;
        };
        let _ = tx.send(Some(hwnd.0 as isize));
        let mut msg = MSG::default();
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                DispatchMessageW(&msg);
            }
            // Ends the registration (it must be removed explicitly, a destroyed window is not enough).
            let remove =
                RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_REMOVE, hwndTarget: HWND::default() };
            let _ = RegisterRawInputDevices(&[remove], size_of::<RAWINPUTDEVICE>() as u32);
        }
    });
    if let Ok(thread) = spawned {
        match rx.recv().ok().flatten() {
            Some(hwnd) => *watch = Some((hwnd, thread)),
            None => {
                let _ = thread.join();
            }
        }
    }
}

/// Removes the registration and ends the thread. Waits for it, with the lock held, so that a
/// start right after cannot have its registration removed by the thread that is leaving.
pub fn stop_ctrl_watch() {
    let mut watch = WATCH.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((hwnd, thread)) = watch.take() {
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        let _ = thread.join();
    }
}

unsafe fn create_watch_window() -> Option<HWND> {
    let instance = GetModuleHandleW(None).ok()?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(watch_proc),
        hInstance: instance.into(),
        lpszClassName: CLASS,
        ..Default::default()
    };
    RegisterClassW(&class); // already registered on a second start: fine
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        CLASS,
        w!("Larik Ctrl watch"),
        WINDOW_STYLE::default(),
        0,
        0,
        0,
        0,
        Some(HWND_MESSAGE),
        None,
        Some(instance.into()),
        None,
    )
    .ok()?;
    let keyboard = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
    if RegisterRawInputDevices(&[keyboard], size_of::<RAWINPUTDEVICE>() as u32).is_err() {
        let _ = DestroyWindow(hwnd);
        return None;
    }
    Some(hwnd)
}

unsafe extern "system" fn watch_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // A panic must not unwind into Windows.
        WM_INPUT => {
            let _ = std::panic::catch_unwind(|| ctrl_from_input(lparam));
            DefWindowProcW(hwnd, msg, wparam, lparam) // required for input received in the background
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Reads one raw keyboard event and passes on whether Ctrl went down or up.
unsafe fn ctrl_from_input(lparam: LPARAM) {
    let mut raw = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    let read = GetRawInputData(
        HRAWINPUT(lparam.0 as *mut _),
        RID_INPUT,
        Some((&mut raw as *mut RAWINPUT).cast()),
        &mut size,
        size_of::<RAWINPUTHEADER>() as u32,
    );
    if read != u32::MAX && raw.header.dwType == RIM_TYPEKEYBOARD.0 {
        let key = raw.data.keyboard;
        if key.VKey == VK_CONTROL.0 {
            let down = u32::from(key.Flags) & RI_KEY_BREAK == 0;
            WATCH_APP.with(|a| {
                if let Some(app) = a.borrow().as_ref() {
                    set_ctrl(app, down);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_only_when_the_last_one_is_30_seconds_old() {
        assert!(restart_allowed(None, 1000));
        assert!(!restart_allowed(Some(1000), 1000));
        assert!(!restart_allowed(Some(1000), 1029));
        assert!(restart_allowed(Some(1000), 1030));
        // A marker from the future (the clock went back) must not turn recovery off.
        assert!(restart_allowed(Some(1000), 900));
    }

    const SCREEN: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };

    #[test]
    fn default_rect_is_60_percent_centred_above_taskbar() {
        let r = default_rect(Rect::new(0, 0, 1920, 1040), 1.0, 22);
        assert_eq!((r.x, r.w, r.bottom()), (384, 1152, 1032));
        assert_eq!(r.h, strip_height(22, 1.0));
    }

    #[test]
    fn default_rect_scales() {
        let r = default_rect(Rect::new(0, 0, 2880, 1560), 1.5, 22);
        assert_eq!((r.x, r.w, r.bottom()), (576, 1728, 1548));
        assert_eq!(r.h, 280);
    }

    #[test]
    fn default_rect_follows_work_area_origin() {
        let r = default_rect(Rect::new(1920, 100, 1000, 900), 1.0, 22);
        assert_eq!((r.x, r.w, r.bottom()), (1920 + 200, 600, 992));
    }

    #[test]
    fn height_grows_with_font() {
        assert!(strip_height(28, 1.0) > strip_height(22, 1.0));
        assert!(strip_height(18, 1.0) < strip_height(22, 1.0));
    }

    #[test]
    fn clamp_keeps_visible_position() {
        assert_eq!(clamp_to_monitors((300, 800), (1152, 190), &[SCREEN]), (300, 800));
        // 100 px of the strip are still on screen.
        assert_eq!(clamp_to_monitors((1820, 800), (1152, 190), &[SCREEN]), (1820, 800));
    }

    #[test]
    fn clamp_pulls_back_offscreen_position() {
        assert_eq!(clamp_to_monitors((5000, 800), (1152, 190), &[SCREEN]), (768, 800));
        assert_eq!(clamp_to_monitors((-2000, -500), (1152, 190), &[SCREEN]), (0, 0));
        // Only 40 px visible: not enough.
        assert_eq!(clamp_to_monitors((1880, 800), (1152, 190), &[SCREEN]), (768, 800));
        // Below the bottom edge.
        assert_eq!(clamp_to_monitors((300, 1200), (1152, 190), &[SCREEN]), (300, 890));
    }

    #[test]
    fn clamp_with_two_monitors() {
        let right = Rect::new(1920, 0, 1920, 1080);
        let both = [SCREEN, right];
        // Straddling the seam is fine.
        assert_eq!(clamp_to_monitors((1500, 800), (1152, 190), &both), (1500, 800));
        // Fully on the second monitor is fine.
        assert_eq!(clamp_to_monitors((2500, 800), (1152, 190), &both), (2500, 800));
        // Off the right end: pulled to the nearest monitor, the second one.
        assert_eq!(clamp_to_monitors((4000, 800), (1152, 190), &both), (2688, 800));
        // Off the left end: pulled to the first one.
        assert_eq!(clamp_to_monitors((-3000, 800), (1152, 190), &both), (0, 800));
        // Second monitor unplugged: back onto the one left.
        assert_eq!(clamp_to_monitors((2500, 800), (1152, 190), &[SCREEN]), (768, 800));
    }

    #[test]
    fn clamp_no_monitors_returns_input() {
        assert_eq!(clamp_to_monitors((-9000, 9000), (1152, 190), &[]), (-9000, 9000));
    }
}
