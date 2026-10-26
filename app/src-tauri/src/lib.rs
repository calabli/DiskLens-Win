mod cache;
mod cleanup;
mod commands;
mod refresh;
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
            cache::cached_scans,
            cache::open_cached,
            refresh::refresh_scan,
            commands::node_info,
            commands::list_children,
            commands::treemap,
            commands::type_stats,
            commands::search,
            cleanup::plan_delete,
            cleanup::delete_items,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
