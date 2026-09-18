fn main() {
    let attrs = tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&["reopen_main"]),
    );
    tauri_build::try_build(attrs).expect("tauri build");
}
