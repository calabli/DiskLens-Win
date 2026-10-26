mod commands;
mod scan;
mod volumes;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(scan::AppState::default())
        .invoke_handler(tauri::generate_handler![
            volumes::list_volumes,
            scan::start_scan,
            scan::cancel_scan,
            commands::node_info,
            commands::list_children,
            commands::treemap,
            commands::type_stats,
            commands::search,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
