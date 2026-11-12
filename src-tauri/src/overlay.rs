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

/// Keeps the strip out of screen sharing and recordings (spec B8), or lets it in. Windows refuses to
/// change this on a layered window, which the window library makes for click-through, so a failed
/// call is retried with click-through briefly off. Still failing (Windows before 10 2004): ignored,
/// the strip is then simply captured.
fn set_affinity(window: &WebviewWindow) {
    let Ok(hwnd) = window.hwnd() else { return };
    let affinity = if EXCLUDE.load(Ordering::SeqCst) { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    if unsafe { SetWindowDisplayAffinity(hwnd, affinity) }.is_err() {
        click_through(window, false);
        let _ = unsafe { SetWindowDisplayAffinity(hwnd, affinity) };
        click_through(window, !CTRL.load(Ordering::SeqCst));
    }
}

/// Click-through on or off. Not taking focus comes from `focusable: false` in tauri.conf.json.
fn click_through(window: &WebviewWindow, on: bool) {
    let _ = window.set_ignore_cursor_events(on);
    keep_out_of_alt_tab(window);
}

/// A tool window without `WS_EX_APPWINDOW` is not listed in Alt+Tab. The window library rebuilds all
/// extended styles whenever one of its flags changes (shown, hidden, click-through), so this is
/// repeated after each of those calls.
fn keep_out_of_alt_tab(window: &WebviewWindow) {
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
    let mut settings = store.load();
    settings.position = None;
    store.save(&settings);
    PLACED.store(false, Ordering::SeqCst);
    apply(app, &settings);
}

fn save_position(app: &AppHandle, pos: (i32, i32)) {
    if let Some(store) = store(app) {
        // Saved on top of the file, so a change made by the settings window in the meantime is kept.
        let mut settings = store.load();
        settings.position = Some(pos);
        store.save(&settings);
    }
}

/// Shows or hides the strip. The Ctrl watch only exists while it is shown.
pub fn set_visible(app: &AppHandle, visible: bool) {
    let Some(window) = overlay_window(app) else { return };
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
fn set_ctrl(app: &AppHandle, down: bool) {
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
static WATCH: Mutex<Option<isize>> = Mutex::new(None);

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
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
            // Ends the registration (it must be removed explicitly, a destroyed window is not enough).
            let remove =
                RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_REMOVE, hwndTarget: HWND::default() };
            let _ = RegisterRawInputDevices(&[remove], size_of::<RAWINPUTDEVICE>() as u32);
        }
    });
    if spawned.is_ok() {
        *watch = rx.recv().ok().flatten();
    }
}

/// Removes the registration and ends the thread.
pub fn stop_ctrl_watch() {
    if let Some(hwnd) = WATCH.lock().unwrap_or_else(PoisonError::into_inner).take() {
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
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
        WM_INPUT => {
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
            DefWindowProcW(hwnd, msg, wparam, lparam) // required for input received in the background
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
