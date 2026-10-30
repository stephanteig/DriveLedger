//! DriveLedger desktop app. The Rust side is a thin layer of Tauri commands
//! over `driveledger-core`; it never shells out to the CLI.

mod commands;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::list_volumes,
            commands::list_saved_indexes,
            commands::start_scan,
            commands::cancel_scan,
            commands::compare,
            commands::export_compare,
            commands::verify,
            commands::search_files,
            commands::is_available,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DriveLedger");
}
