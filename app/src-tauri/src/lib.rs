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
    allocated: u64,
    files: u32,
    is_dir: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum ScanMode {
    /// 直读 NTFS MFT。
    Mft,
    /// 逐个目录遍历。
    Walk,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanSummary {
    root: String,
    mode: ScanMode,
    /// 想用 MFT 直读但没成功时的原因。
    fallback_reason: Option<String>,
    total_size: u64,
    total_allocated: u64,
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
        allocated: tree.allocated(id),
        files: tree.file_count(id),
        is_dir: tree.kind(id) == NodeKind::Dir,
    }
}

/// `D:`、`D:\` 这样的卷根目录返回盘符，其他路径返回 `None`。
fn volume_root(path: &str) -> Option<char> {
    let mut chars = path.trim().chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    match (chars.next(), chars.next(), chars.next()) {
        (Some(':'), None | Some('\\' | '/'), None) => Some(letter),
        _ => None,
    }
}

/// 扫描卷根目录时先尝试 MFT 直读（需要管理员权限），失败则降级为普通遍历；
/// 其他路径直接用普通遍历。返回汇总和按大小排序的第一层子项。
#[tauri::command]
async fn scan_folder(path: String) -> Result<ScanSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let mut fallback_reason = None;

        #[cfg(windows)]
        if let Some(letter) = volume_root(&path) {
            match dl_ntfs::scan_volume(letter, |_| {}) {
                Ok(scan) => {
                    return Ok(summarize(path, ScanMode::Mft, None, &scan.tree, 0, started));
                }
                Err(e) => fallback_reason = Some(e.to_string()),
            }
        }

        let result = dl_walk::walk(&PathBuf::from(&path)).map_err(|e| e.to_string())?;
        Ok(summarize(
            path,
            ScanMode::Walk,
            fallback_reason,
            &result.tree,
            result.skipped,
            started,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn summarize(
    root: String,
    mode: ScanMode,
    fallback_reason: Option<String>,
    tree: &FileTree,
    skipped: u64,
    started: Instant,
) -> ScanSummary {
    let top = tree.root();
    ScanSummary {
        root,
        mode,
        fallback_reason,
        total_size: tree.size(top),
        total_allocated: tree.allocated(top),
        file_count: tree.file_count(top),
        node_count: tree.len(),
        skipped,
        elapsed_ms: started.elapsed().as_millis(),
        children: tree
            .children_by_size(top)
            .into_iter()
            .take(MAX_CHILDREN)
            .map(|id| entry(tree, id))
            .collect(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![scan_folder])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::volume_root;

    #[test]
    fn recognizes_volume_roots() {
        assert_eq!(volume_root("D:"), Some('D'));
        assert_eq!(volume_root(r"d:\"), Some('d'));
        assert_eq!(volume_root(" D:/ "), Some('D'));
        assert_eq!(volume_root(r"D:\ai"), None);
        assert_eq!(volume_root(r"\\server\share"), None);
        assert_eq!(volume_root(""), None);
    }
}
