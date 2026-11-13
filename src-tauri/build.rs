fn main() {
    // The app's commands are listed so that a capability can name each one (`allow-shown`, `allow-get-settings`, ...)
    // instead of using `core:default`: the overlay may call only `shown` and `ctrl_released`, the settings window only the other three.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["shown", "ctrl_released", "get_settings", "set_settings", "strings"])),
    )
    .expect("failed to run tauri-build");
}
