fn main() {
    // `shown` is listed so that a capability can name it (`allow-shown`) instead of using `core:default`.
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&["shown"])),
    )
    .expect("failed to run tauri-build");
}
