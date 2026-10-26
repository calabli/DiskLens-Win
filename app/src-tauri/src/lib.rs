use std::path::PathBuf;
use std::time::Instant;

use dl_tree::{FileTree, NodeId, NodeKind};
use serde::Serialize;

/// 列表里最多返回多少个子项。
const MAX_CHILDREN: usize = 200;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    name: String,
    size: u64,
    files: u32,
    is_dir: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanSummary {
    root: String,
    total_size: u64,
    file_count: u32,
    node_count: usize,
    skipped: u64,
    elapsed_ms: u128,
    children: Vec<Entry>,
}

fn entry(tree: &FileTree, id: NodeId) -> Entry {
    Entry {
        name: tree.name(id).to_owned(),
        size: tree.size(id),
        files: tree.file_count(id),
        is_dir: tree.kind(id) == NodeKind::Dir,
    }
}

/// 用普通遍历扫描一个文件夹，返回汇总和按大小排序的第一层子项。
/// M1 起会换成 MFT 直读，并把整棵树留在 Rust 侧供后续查询。
#[tauri::command]
async fn scan_folder(path: String) -> Result<ScanSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let result = dl_walk::walk(&PathBuf::from(&path)).map_err(|e| e.to_string())?;
        let tree = &result.tree;
        let root = tree.root();
        let children = tree
            .children_by_size(root)
            .into_iter()
            .take(MAX_CHILDREN)
            .map(|id| entry(tree, id))
            .collect();
        Ok(ScanSummary {
            root: path,
            total_size: tree.size(root),
            file_count: tree.file_count(root),
            node_count: tree.len(),
            skipped: result.skipped,
            elapsed_ms: started.elapsed().as_millis(),
            children,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![scan_folder])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
