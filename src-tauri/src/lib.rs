use std::collections::HashSet;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};

pub mod cache;
pub mod clock;
pub mod engine;
pub mod lrc;
pub mod lrclib;
pub mod lyrics;
pub mod matcher;
pub mod normalize;
pub mod schedule;
pub mod settings;
pub mod smtc;
pub mod timing_log;

use engine::{Engine, Host, Msg, Ui};

fn running(fetching: &Mutex<HashSet<String>>) -> MutexGuard<'_, HashSet<String>> {
    fetching.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The engine's view of the machine: the overlay window, SMTC, LRCLIB and the clocks.
struct TauriHost {
    app: AppHandle,
    tx: Sender<Msg>,
    /// None when SMTC could not be started: the app then runs without lyrics.
    watcher: Option<smtc::Watcher>,
    start: Instant,
    /// Keys with a lookup running, so one song is never looked up twice at the same time.
    fetching: Arc<Mutex<HashSet<String>>>,
}

impl Host for TauriHost {
    fn emit(&self, ui: &Ui) {
        let _ = self.app.emit_to("overlay", "ui", ui);
    }

    fn set_overlay_visible(&self, visible: bool) {
        if let Some(overlay) = self.app.get_webview_window("overlay") {
            // The window is built with focus off, so showing it does not take focus from the player.
            let _ = if visible { overlay.show() } else { overlay.hide() };
        }
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
        self.watcher.as_mut()?.read()
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

pub fn run() {
    let (tx, rx) = mpsc::channel();
    let engine_tx = tx.clone();
    tauri::Builder::default()
        .manage(tx)
        .invoke_handler(tauri::generate_handler![shown])
        .setup(move |app| {
            let timing = timing_log::TimingLog::from_env();

            let config = app.config().app.windows.iter().find(|w| w.label == "overlay").cloned();
            let mut overlay = WebviewWindowBuilder::from_config(
                app.handle(),
                &config.ok_or("the overlay window is missing from tauri.conf.json")?,
            )?;
            if timing.is_some() {
                // Asks the overlay to answer every line with `shown`.
                overlay = overlay.initialization_script("window.__LARIK_TIMING__ = true;");
            }
            overlay.build()?;

            let dir = app.path().app_local_data_dir()?;
            let store = settings::Store::new(dir.clone());
            let engine = Engine::new(store.load(), store, cache::Cache::new(dir.join("cache")))
                .with_timing_log(timing);
            let app = app.handle().clone();
            thread::Builder::new().name("engine".into()).spawn(move || {
                // Created here: the watcher joins the MTA of the thread that reads it.
                let notify = engine_tx.clone();
                let watcher = smtc::Watcher::start(move || {
                    let _ = notify.send(Msg::SmtcChanged);
                })
                .map_err(|e| eprintln!("larik: cannot watch media sessions, running without lyrics: {e}"))
                .ok();
                let _ = engine_tx.send(Msg::SmtcChanged);
                let host = TauriHost {
                    app,
                    tx: engine_tx,
                    watcher,
                    start: Instant::now(),
                    fetching: Arc::default(),
                };
                engine::run(rx, engine, host);
            })?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Larik");
}
