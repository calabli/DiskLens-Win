mod cache;
mod cleanup;
mod commands;
mod dupes;
mod files;
mod findings;
mod refresh;
mod rules;
mod scan;
mod similar;
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
            commands::sunburst,
            commands::type_stats,
            commands::bucket_stats,
            commands::search,
            commands::search_marks,
            cleanup::plan_delete,
            cleanup::delete_items,
            cleanup::link_items,
            files::move_folder,
            files::reveal_in_explorer,
            files::open_in_explorer,
            files::owners,
            rules::clean_suggestions,
            rules::plan_clean,
            rules::clean_rules,
            dupes::find_duplicates,
            dupes::cancel_duplicates,
            findings::find_issues,
            findings::cancel_issues,
            similar::find_similar,
            similar::cancel_similar,
            similar::thumbnail,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
