fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_device_snapshot",
            "get_ingest_history",
            "scan_source_inventory",
            "get_remembered_destination",
            "remember_destination",
            "register_card_marker",
            "get_auto_ingest_profile",
            "get_format_eligibility",
            "preview_verified_ingest",
            "request_format_authorization",
            "watch_device_snapshots",
            "calibrate_reader_slot",
            "start_verified_ingest",
            "cancel_verified_ingest",
            "resume_verified_ingest",
        ]),
    ))
    .expect("failed to generate Tauri command permissions");
}
