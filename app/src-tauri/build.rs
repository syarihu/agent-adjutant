fn main() {
    // The app's own commands go through the permission system like a plugin's, so that the board
    // origin (a remote page to Tauri) can be granted exactly the ones it needs and no others.
    let commands = &[
        "get_status",
        "retry",
        "notify",
        "notification_permission",
        "request_notification_permission",
        "focus_window",
        "zoom",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(commands)),
    )
    .expect("failed to run tauri-build");
}
