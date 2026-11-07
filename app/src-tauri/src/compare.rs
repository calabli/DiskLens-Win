//! 变化对比：当前结果和对比基准（上一次完整扫描，或用户手动设定的时刻）相比，
//! 哪些文件夹和文件变大了、变小了。

use std::cmp::Reverse;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use dl_analyze::diff;
use dl_tree::{FileTree, NodeId};
use serde::Serialize;
use tauri::State;

use crate::cache;
use crate::scan::AppState;

/// 变大、变小各最多列出的项数。
const MAX_LIST: usize = 50;
/// 变化至少这么大才列出；扫描范围很大时还要不小于总大小的千分之一。
const MIN_DELTA: u64 = 16 << 20;

/// 读进内存的对比基准。基准文件没变时重复使用，不用每次都从磁盘读。
struct Baseline {
    root: String,
    modified: SystemTime,
    scanned_at: u64,
    tree: FileTree,
}

static BASELINE: Mutex<Option<Arc<Baseline>>> = Mutex::new(None);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeItem {
    path: String,
    /// 现在扫描结果里的节点，可以定位；已删除的项没有。
    id: Option<NodeId>,
    before: u64,
    after: u64,
    is_dir: bool,
    /// `new`、`removed`、`changed`。
    status: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// 基准的扫描时间（Unix 毫秒）；还没有基准时为空，其余字段也都是空的。
    base_scanned_at: Option<u64>,
    before_size: u64,
    after_size: u64,
    before_files: u32,
    after_files: u32,
    grown: Vec<ChangeItem>,
    shrunk: Vec<ChangeItem>,
}

/// 当前结果和对比基准相比的变化。
#[tauri::command]
pub async fn compare_scan(state: State<'_, AppState>) -> Result<Comparison, String> {
    let scan = state.current()?;
    tauri::async_runtime::spawn_blocking(move || {
        let empty = Comparison {
            base_scanned_at: None,
            before_size: 0,
            after_size: 0,
            before_files: 0,
            after_files: 0,
            grown: Vec::new(),
            shrunk: Vec::new(),
        };
        let Some(base) = baseline(&scan.summary.root)? else {
            return Ok(empty);
        };
        let now = scan.read();
        let before = &base.tree;
        let (now_root, before_root) = (now.root(), before.root());
        let total = now.size(now_root).max(before.size(before_root));
        let mut list = diff::changes(&now, before, (total / 1000).max(MIN_DELTA));
        list.sort_by_key(|c| Reverse(c.delta()));
        let item = |c: &diff::Change| ChangeItem {
            path: match (c.now, c.before) {
                (Some(n), _) => now.path(n),
                (None, Some(b)) => before.path(b),
                (None, None) => String::new(),
            },
            id: c.now,
            before: c.size_before,
            after: c.size_after,
            is_dir: c.is_dir,
            status: match (c.now, c.before) {
                (Some(_), None) => "new",
                (None, Some(_)) => "removed",
                _ => "changed",
            },
        };
        Ok(Comparison {
            base_scanned_at: Some(base.scanned_at),
            before_size: before.size(before_root),
            after_size: now.size(now_root),
            before_files: before.file_count(before_root),
            after_files: now.file_count(now_root),
            grown: list
                .iter()
                .filter(|c| c.delta() > 0)
                .take(MAX_LIST)
                .map(item)
                .collect(),
            shrunk: list
                .iter()
                .rev()
                .filter(|c| c.delta() < 0)
                .take(MAX_LIST)
                .map(item)
                .collect(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 把当前结果设为对比基准，返回基准时间（Unix 毫秒）。
#[tauri::command]
pub async fn set_baseline(state: State<'_, AppState>) -> Result<u64, String> {
    let scan = state.current()?;
    let at = tauri::async_runtime::spawn_blocking(move || cache::save_baseline(&scan))
        .await
        .map_err(|e| e.to_string())??;
    *BASELINE.lock().expect("baseline poisoned") = None;
    Ok(at)
}

/// `root` 的对比基准；内存里的还是最新的就直接用。
fn baseline(root: &str) -> Result<Option<Arc<Baseline>>, String> {
    let modified = cache::baseline_modified(root);
    let mut held = BASELINE.lock().expect("baseline poisoned");
    if let Some(b) = held.as_ref()
        && b.root.eq_ignore_ascii_case(root)
        && Some(b.modified) == modified
    {
        return Ok(Some(Arc::clone(b)));
    }
    *held = cache::load_baseline(root)?.map(|(scanned_at, modified, tree)| {
        Arc::new(Baseline {
            root: root.to_owned(),
            modified,
            scanned_at,
            tree,
        })
    });
    Ok(held.clone())
}
