use tauri::Manager;

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
