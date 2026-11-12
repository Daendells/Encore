use tauri::Manager;

pub mod cache;
pub mod lrc;
pub mod lrclib;
pub mod lyrics;
pub mod matcher;
pub mod normalize;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if let Some(overlay) = app.get_webview_window("overlay") {
                overlay.show()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Larik");
}
