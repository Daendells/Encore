use std::collections::HashSet;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewWindowBuilder, WindowEvent};

pub mod cache;
pub mod clock;
pub mod engine;
pub mod i18n;
pub mod lrc;
pub mod lrclib;
pub mod lyrics;
pub mod matcher;
pub mod network;
pub mod normalize;
pub mod overlay;
pub mod schedule;
pub mod settings;
pub mod shell;
pub mod smtc;
pub mod timing_log;

use engine::{Engine, Host, Msg, Ui};

fn running(fetching: &Mutex<HashSet<String>>) -> MutexGuard<'_, HashSet<String>> {
    fetching.lock().unwrap_or_else(PoisonError::into_inner)
}

/// WebView2 flags for every window: Tauri's own defaults plus what keeps the app's only network traffic
/// LRCLIB and its memory small. All windows must use the same string, because they share one WebView2
/// profile and WebView2 refuses to open a window whose flags differ.
/// - `msOneAuthWAM,msForceOneAuthWAM`: with these on, WebView2 signs the Windows account in to Microsoft
/// - `DnsOverHttpsUpgrade`: without it, when the system DNS server is a known one (here Google), Chromium
///   switches to DNS over HTTPS and connects to 8.8.8.8 and 8.8.4.4 on port 443 a few seconds after start.
///   services at start (a connection to substrate.office.com). Both must be off, either one alone is not enough.
/// - `--disable-gpu`: no GPU, so the strip is drawn in software. This removes about 60 MB of GPU process
///   and the plate stays transparent. The strip is three lines of text, so nothing is lost.
/// - `--single-process`: the page, network and storage run inside the WebView2 browser process instead of
///   four more processes (about 40 MB less). Microsoft does not document this flag. A crash of the page
///   then takes the whole WebView2 down; `overlay::restart_on_webview_failure` restarts the app when that happens.
pub const WEBVIEW_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,OptimizationHints,msOneAuthWAM,msForceOneAuthWAM,DnsOverHttpsUpgrade --disable-background-networking --disable-component-update --disable-gpu --single-process";

/// The engine's view of the machine: the overlay window, SMTC, LRCLIB and the clocks.
struct TauriHost {
    app: AppHandle,
    tx: Sender<Msg>,
    /// None when SMTC could not be started: the app then runs without lyrics.
    watcher: Option<smtc::Watcher>,
    /// Held so that the subscription lasts as long as the engine thread.
    _network: Option<network::NetworkWatcher>,
    start: Instant,
    /// Keys with a lookup running, so one song is never looked up twice at the same time.
    fetching: Arc<Mutex<HashSet<String>>>,
}

impl Host for TauriHost {
    fn emit(&self, ui: &Ui) {
        let _ = self.app.emit_to("overlay", "ui", ui);
    }

    fn set_overlay_visible(&self, visible: bool) {
        // The window is built with focus off, so showing it does not take focus from the player.
        overlay::set_visible(&self.app, visible);
    }

    fn settings_changed(&self, settings: &settings::Settings) {
        overlay::apply(&self.app, settings);
        shell::refresh(&self.app, settings);
    }

    fn fetch_lyrics(&self, key: String, track: matcher::Track) {
        if !running(&self.fetching).insert(key.clone()) {
            return;
        }
        let (tx, fetching, worker_key) = (self.tx.clone(), self.fetching.clone(), key.clone());
        let spawned = thread::Builder::new().name("lyrics".into()).spawn(move || {
            let result = lyrics::resolve(&track, &lrclib::LrcLib::new())
                .map(|(lyrics, _)| lyrics)
                .map_err(|e| format!("{e:?}"));
            running(&fetching).remove(&worker_key);
            let _ = tx.send(Msg::Lyrics { key: worker_key, result });
        });
        if spawned.is_err() {
            running(&self.fetching).remove(&key);
        }
    }

    fn read_smtc(&mut self) -> Option<smtc::Snapshot> {
        let snapshot = self.watcher.as_mut()?.read();
        if let Some(s) = &snapshot {
            self.app.state::<shell::Shared>().saw_app(&s.app);
        }
        snapshot
    }

    fn now_mono_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn now_wall_ms(&self) -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
    }
}

/// The overlay reports that a line is on screen; only the timing log uses it.
#[tauri::command]
fn shown(seq: u64, tx: tauri::State<Sender<Msg>>) {
    let _ = tx.send(Msg::Shown { seq });
}

/// The overlay saw a mouse event without Ctrl while it is clickable: the key-up never reached the Raw Input window.
#[tauri::command]
fn ctrl_released(app: AppHandle) {
    overlay::set_ctrl(&app, false);
}

pub fn run() {
    let (tx, rx) = mpsc::channel();
    let engine_tx = tx.clone();
    let second_start = tx.clone();
    let shortcut_tx = tx.clone();
    let shell_tx = tx.clone();
    tauri::Builder::default()
        // First, so that a second start ends before anything else is set up. It shows the strip of the first.
        .plugin(tauri_plugin_single_instance::init(move |_, _, _| {
            let _ = second_start.send(Msg::SetVisible(true));
        }))
        .plugin(tauri_plugin_autostart::Builder::new().app_name("Encore").build())
        .plugin(shell::shortcut_plugin(shortcut_tx))
        .manage(tx)
        .manage(shell::Shared::new(i18n::detect()))
        .invoke_handler(tauri::generate_handler![shown, ctrl_released, shell::get_settings, shell::set_settings, shell::strings])
        .setup(move |app| {
            let timing = timing_log::TimingLog::from_env();
            let dir = app.path().app_local_data_dir()?;
            let store = settings::Store::new(dir.clone());
            let settings = store.load();

            let config = app.config().app.windows.iter().find(|w| w.label == "overlay").cloned();
            // The first style goes in with the page: an event sent before the page listens is lost.
            let mut script = format!("window.__ENCORE_STYLE__ = {};", overlay::style(&settings));
            if timing.is_some() {
                // Asks the overlay to answer every line with `shown`.
                script.push_str("window.__ENCORE_TIMING__ = true;");
            }
            let overlay = WebviewWindowBuilder::from_config(
                app.handle(),
                &config.ok_or("the overlay window is missing from tauri.conf.json")?,
            )?
            .additional_browser_args(WEBVIEW_ARGS)
            .initialization_script(script)
            .build()?;
            overlay::restart_on_webview_failure(app.handle(), &overlay);
            overlay::apply(app.handle(), &settings);
            shell::setup(app.handle(), shell_tx)?;
            let display = app.handle().clone();
            overlay.on_window_event(move |event| {
                // Another monitor or a new scale: the size in physical pixels changes. On its own thread
                // because the window is mid-resize when this fires.
                if matches!(event, WindowEvent::ScaleFactorChanged { .. }) {
                    let app = display.clone();
                    thread::spawn(move || {
                        if let Ok(dir) = app.path().app_local_data_dir() {
                            overlay::apply(&app, &settings::Store::new(dir).load());
                        }
                    });
                }
            });

            let engine = Engine::new(settings, store, cache::Cache::new(dir.join("cache")))
                .with_timing_log(timing);
            let app = app.handle().clone();
            thread::Builder::new().name("engine".into()).spawn(move || {
                // Created here: the watcher joins the MTA of the thread that reads it.
                let notify = engine_tx.clone();
                let watcher = smtc::Watcher::start(move || {
                    let _ = notify.send(Msg::SmtcChanged);
                })
                .map_err(|e| eprintln!("encore: cannot watch media sessions, running without lyrics: {e}"))
                .ok();
                let notify = engine_tx.clone();
                let network = network::NetworkWatcher::start(move || {
                    let _ = notify.send(Msg::NetworkChanged);
                })
                .map_err(|e| eprintln!("encore: cannot watch the network, no retry on reconnect: {e}"))
                .ok();
                let _ = engine_tx.send(Msg::SmtcChanged);
                let host = TauriHost {
                    app,
                    tx: engine_tx,
                    watcher,
                    _network: network,
                    start: Instant::now(),
                    fetching: Arc::default(),
                };
                engine::run(rx, engine, host);
            })?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Encore")
        .run(|_, event| {
            // Whatever ends the app (tray Quit, an exit request, logoff), WebView2 going away after it is no failure.
            if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
                overlay::mark_exiting();
            }
        });
}
