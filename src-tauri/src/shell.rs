//! The parts of the app a user touches outside the strip: tray, global shortcuts, the settings
//! window and the commands it calls.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, PoisonError};
use std::thread;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::engine::Msg;
use crate::i18n::{self, t, Lang};
use crate::settings::{Settings, Store, FONT_PX_RANGE};

const OFFSET_RANGE_MS: std::ops::RangeInclusive<i32> = -2000..=2000;
const MAX_APPS: usize = 50;
const MAX_APP_ID_LEN: usize = 200;
/// Tray text sizes (spec B16): small, medium, large.
const SIZES: [(u32, &str); 3] = [(18, "tray.size.small"), (22, "tray.size.medium"), (28, "tray.size.large")];

#[derive(Clone, Copy)]
enum Action {
    Toggle,
    Earlier,
    Later,
    Settings,
}

/// Win+Alt+`code`: how it reads, what it does, and the text that describes it in the settings window.
const SHORTCUTS: [(Code, &str, Action, &str); 4] = [
    (Code::KeyL, "Win+Alt+L", Action::Toggle, "settings.shortcut.toggle"),
    // Spec B11: `]` shows the lyrics earlier, so the offset grows.
    (Code::BracketRight, "Win+Alt+]", Action::Earlier, "settings.shortcut.earlier"),
    (Code::BracketLeft, "Win+Alt+[", Action::Later, "settings.shortcut.later"),
    (Code::KeyO, "Win+Alt+O", Action::Settings, "settings.shortcut.settings"),
];

fn shortcut(code: Code) -> Shortcut {
    Shortcut::new(Some(Modifiers::SUPER | Modifiers::ALT), code)
}

/// What the tray, the shortcuts and the settings window share with the rest of the app.
pub struct Shared {
    pub lang: Lang,
    /// Shortcuts (by their description key) that Windows refused to register: another app holds them.
    unavailable: Mutex<BTreeSet<&'static str>>,
    /// Player ids seen this session, for the offset list. Never more than `MAX_APPS`.
    seen: Mutex<BTreeSet<String>>,
}

impl Shared {
    pub fn new(lang: Lang) -> Self {
        Self { lang, unavailable: Mutex::default(), seen: Mutex::default() }
    }

    pub fn saw_app(&self, app: &str) {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        if !app.is_empty() && app.chars().count() <= MAX_APP_ID_LEN && seen.len() < MAX_APPS {
            seen.insert(app.to_string());
        }
    }
}

struct TrayItems {
    toggle: MenuItem<Wry>,
    sizes: Vec<CheckMenuItem<Wry>>,
    autostart: CheckMenuItem<Wry>,
}

fn store(app: &AppHandle) -> Option<Store> {
    app.path().app_local_data_dir().ok().map(Store::new)
}

fn lang(app: &AppHandle) -> Lang {
    app.state::<Shared>().lang
}

/// Saves a change on top of the file (so a change made elsewhere in the meantime is kept) and lets the
/// engine reload it. The engine applies it to the strip (size, capture, the `style` event) and calls `refresh`.
fn update(app: &AppHandle, change: impl FnOnce(&mut Settings)) {
    let Some(store) = store(app) else { return };
    store.update(change);
    let _ = app.state::<Sender<Msg>>().send(Msg::SettingsChanged);
}

/// The global shortcuts and the tray. The app does not depend on the tray: if it cannot be built (at logon
/// this can run before Explorer has created the notification area) the error is logged, the shortcuts and the
/// strip keep working, and the tray is tried once more after 3 seconds.
pub fn setup(app: &AppHandle, tx: Sender<Msg>) -> tauri::Result<()> {
    let lang = lang(app);
    for (code, _, _, key) in SHORTCUTS {
        if app.global_shortcut().register(shortcut(code)).is_err() {
            eprintln!("encore: the shortcut for \"{}\" is taken by another app", t(lang, key));
            app.state::<Shared>().unavailable.lock().unwrap_or_else(PoisonError::into_inner).insert(key);
        }
    }

    if let Err(e) = build_tray(app, &tx) {
        eprintln!("encore: no tray icon, trying again in 3 seconds: {e}");
        let app = app.clone();
        let spawned = thread::Builder::new().name("tray-retry".into()).spawn(move || {
            thread::sleep(std::time::Duration::from_secs(3));
            let on_main = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Err(e) = build_tray(&on_main, &tx) {
                    eprintln!("encore: still no tray icon, running without it: {e}");
                }
            });
        });
        if spawned.is_err() {
            eprintln!("encore: cannot retry the tray icon");
        }
    }
    Ok(())
}

fn build_tray(app: &AppHandle, tx: &Sender<Msg>) -> tauri::Result<()> {
    let tx = tx.clone();
    let lang = lang(app);
    let settings = store(app).map(|s| s.load()).unwrap_or_default();

    let toggle_text = t(lang, if settings.visible { "tray.hide" } else { "tray.show" });
    let toggle = MenuItem::with_id(app, "toggle", toggle_text, true, None::<&str>)?;
    let sizes = SIZES
        .iter()
        .map(|(px, key)| {
            let text = format!("{} ({px} {})", t(lang, key), t(lang, "unit.px"));
            CheckMenuItem::with_id(app, format!("size.{px}"), text, true, settings.font_px == *px, None::<&str>)
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let size_items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = sizes.iter().map(|i| i as _).collect();
    let size_menu = Submenu::with_items(app, t(lang, "tray.size"), true, &size_items)?;
    let reset = MenuItem::with_id(app, "reset", t(lang, "tray.reset_position"), true, None::<&str>)?;
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(app, "autostart", t(lang, "tray.autostart"), true, autostart_on, None::<&str>)?;
    let open = MenuItem::with_id(app, "settings", t(lang, "tray.settings"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t(lang, "tray.quit"), true, None::<&str>)?;
    let line = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &size_menu, &reset, &autostart, &line, &open, &quit])?;

    let click_tx = tx.clone();
    let mut tray = TrayIconBuilder::with_id("encore")
        .tooltip("Encore")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "toggle" => _ = tx.send(Msg::ToggleVisible),
            "reset" => {
                let app = app.clone();
                thread::spawn(move || {
                    if let Some(store) = store(&app) {
                        crate::overlay::reset_position(&app, &store);
                    }
                });
            }
            "autostart" => {
                let manager = app.autolaunch();
                let _ = if manager.is_enabled().unwrap_or(false) { manager.disable() } else { manager.enable() };
                refresh_autostart(app);
            }
            "settings" => open_settings(app),
            "quit" => {
                crate::overlay::mark_exiting();
                let _ = tx.send(Msg::Quit);
                app.exit(0);
            }
            id => {
                if let Some(px) = id.strip_prefix("size.").and_then(|px| px.parse().ok()) {
                    update(app, |s| s.font_px = px);
                }
            }
        })
        .on_tray_icon_event(move |_, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                let _ = click_tx.send(Msg::ToggleVisible);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.manage(TrayItems { toggle, sizes, autostart });
    Ok(())
}

/// The tray follows the saved settings: the toggle's wording, the checked text size and the Run key.
pub fn refresh(app: &AppHandle, settings: &Settings) {
    let Some(items) = app.try_state::<TrayItems>() else { return };
    let _ = items.toggle.set_text(t(lang(app), if settings.visible { "tray.hide" } else { "tray.show" }));
    for (item, (px, _)) in items.sizes.iter().zip(SIZES) {
        let _ = item.set_checked(settings.font_px == px);
    }
    refresh_autostart(app);
}

/// The check mark is the real state of the Run key, not what was last clicked.
fn refresh_autostart(app: &AppHandle) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.autostart.set_checked(app.autolaunch().is_enabled().unwrap_or(false));
    }
}

/// The global shortcut plugin, with its handler.
pub fn shortcut_plugin(tx: Sender<Msg>) -> tauri::plugin::TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(move |app, pressed, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let Some((_, _, action, _)) = SHORTCUTS.iter().find(|(code, ..)| shortcut(*code) == *pressed) else { return };
            match action {
                Action::Toggle => _ = tx.send(Msg::ToggleVisible),
                Action::Earlier => _ = tx.send(Msg::Nudge(250)),
                Action::Later => _ = tx.send(Msg::Nudge(-250)),
                Action::Settings => open_settings(app),
            }
        })
        .build()
}

/// Shows the settings window, creating it if it is not open. Built on its own thread because a window
/// cannot be created from inside the event handler that asked for it.
pub fn open_settings(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        if let Some(window) = app.get_webview_window("settings") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
            return;
        }
        let built = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
            .title(t(lang(&app), "settings.title"))
            .inner_size(420.0, 520.0)
            .center()
            .additional_browser_args(crate::WEBVIEW_ARGS)
            .build();
        if let Ok(window) = built {
            // The tray's check mark must show what the settings window changed in the Run key.
            let handle = app.clone();
            window.on_window_event(move |event| {
                if matches!(event, WindowEvent::Focused(false) | WindowEvent::Destroyed) {
                    refresh_autostart(&handle);
                }
            });
        }
    });
}

// ---- Commands of the settings window ----

#[derive(serde::Serialize)]
pub struct SettingsView {
    font_px: u32,
    hide_from_capture: bool,
    /// Every app with an offset, plus the players seen this session.
    apps: Vec<AppOffset>,
    shortcuts: Vec<ShortcutView>,
    version: &'static str,
}

#[derive(serde::Serialize)]
struct AppOffset {
    id: String,
    offset_ms: i32,
}

#[derive(serde::Serialize)]
struct ShortcutView {
    keys: &'static str,
    label: &'static str,
    available: bool,
}

#[derive(serde::Deserialize)]
pub struct SettingsInput {
    font_px: u32,
    hide_from_capture: bool,
    app_offsets_ms: BTreeMap<String, i32>,
}

impl SettingsInput {
    /// The same limits as the page, enforced here because the page is not trusted.
    fn validate(&self) -> Result<(), &'static str> {
        if !FONT_PX_RANGE.contains(&self.font_px) {
            return Err("text size out of range");
        }
        if self.app_offsets_ms.len() > MAX_APPS {
            return Err("too many apps");
        }
        for (id, ms) in &self.app_offsets_ms {
            if id.is_empty() || id.chars().count() > MAX_APP_ID_LEN {
                return Err("app id length");
            }
            if !OFFSET_RANGE_MS.contains(ms) {
                return Err("offset out of range");
            }
        }
        Ok(())
    }

    /// Validates, then writes the fields into `settings`. An offset of 0 is no offset. On error `settings` is untouched.
    fn apply_to(self, settings: &mut Settings) -> Result<(), &'static str> {
        self.validate()?;
        settings.font_px = self.font_px;
        settings.hide_from_capture = self.hide_from_capture;
        settings.app_offsets_ms = self.app_offsets_ms.into_iter().filter(|(_, ms)| *ms != 0).collect();
        Ok(())
    }
}

#[tauri::command]
pub fn get_settings(app: AppHandle, shared: State<Shared>) -> SettingsView {
    let settings = store(&app).map(|s| s.load()).unwrap_or_default();
    let mut ids: BTreeSet<String> = settings.app_offsets_ms.keys().cloned().collect();
    ids.extend(shared.seen.lock().unwrap_or_else(PoisonError::into_inner).iter().cloned());
    let unavailable = shared.unavailable.lock().unwrap_or_else(PoisonError::into_inner).clone();
    SettingsView {
        font_px: settings.font_px,
        hide_from_capture: settings.hide_from_capture,
        apps: ids
            .into_iter()
            .take(MAX_APPS)
            .map(|id| AppOffset { offset_ms: settings.app_offsets_ms.get(&id).copied().unwrap_or(0), id })
            .collect(),
        shortcuts: SHORTCUTS
            .iter()
            .map(|(_, keys, _, label)| ShortcutView { keys, label, available: !unavailable.contains(label) })
            .collect(),
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// Saves the settings and lets the engine apply them. Invalid input changes nothing and returns an error.
#[tauri::command]
pub fn set_settings(app: AppHandle, input: SettingsInput) -> Result<(), String> {
    input.validate().map_err(str::to_string)?;
    update(&app, |s| {
        let _ = input.apply_to(s);
    });
    Ok(())
}

#[tauri::command]
pub fn strings(shared: State<Shared>) -> BTreeMap<String, String> {
    i18n::all_keys().into_iter().map(|k| (k.to_string(), t(shared.lang, k).to_string())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(font_px: u32, apps: &[(&str, i32)]) -> SettingsInput {
        SettingsInput {
            font_px,
            hide_from_capture: false,
            app_offsets_ms: apps.iter().map(|(id, ms)| (id.to_string(), *ms)).collect(),
        }
    }

    #[test]
    fn valid_input_is_applied_and_keeps_the_rest() {
        let mut s = Settings { visible: false, position: Some((1, 2)), ..Settings::default() };
        input(30, &[("Spotify.exe", 250), ("chrome.exe", 0)]).apply_to(&mut s).unwrap();
        assert_eq!((s.font_px, s.hide_from_capture, s.visible, s.position), (30, false, false, Some((1, 2))));
        assert_eq!(s.app_offsets_ms.len(), 1, "an offset of 0 is dropped");
        assert_eq!(s.app_offsets_ms["Spotify.exe"], 250);
    }

    #[test]
    fn set_settings_validation() {
        let long = "x".repeat(MAX_APP_ID_LEN + 1);
        let ids: Vec<String> = (0..=MAX_APPS).map(|i| format!("app{i}")).collect();
        let many: Vec<(&str, i32)> = ids.iter().map(|id| (id.as_str(), 100)).collect();
        let bad = [
            input(5, &[]),
            input(49, &[]),
            input(22, &[("a", 99_999)]),
            input(22, &[("a", -2001)]),
            input(22, &[(&long, 0)]),
            input(22, &[("", 100)]),
            input(22, &many),
        ];
        for b in bad {
            let before = Settings::default();
            let mut s = before.clone();
            assert!(b.apply_to(&mut s).is_err());
            assert_eq!(s, before, "invalid input changes nothing");
        }
        assert!(input(12, &many[..MAX_APPS]).validate().is_ok());
        assert!(input(48, &[("a", -2000), (&"x".repeat(MAX_APP_ID_LEN), 2000)]).validate().is_ok());
    }

    #[test]
    fn shortcut_directions_follow_the_spec() {
        // B11: Win+Alt+] shows the lyrics earlier (offset up), Win+Alt+[ later.
        let action = |keys| SHORTCUTS.iter().find(|s| s.1 == keys).map(|s| s.2);
        assert!(matches!(action("Win+Alt+]"), Some(Action::Earlier)));
        assert!(matches!(action("Win+Alt+["), Some(Action::Later)));
    }

    #[test]
    fn seen_apps_are_bounded() {
        let shared = Shared::new(Lang::En);
        for i in 0..80 {
            shared.saw_app(&format!("app{i}"));
        }
        shared.saw_app(&"x".repeat(MAX_APP_ID_LEN + 1));
        assert_eq!(shared.seen.lock().unwrap().len(), MAX_APPS);
    }
}
