//! 增量刷新：按 USN 日志把上次扫描之后的变化应用到当前结果上，不必重新扫描。
//!
//! 只有以 MFT 直读方式扫描整个磁盘、且磁盘启用了 USN 日志时才能增量刷新；
//! 否则返回 [`Refresh::NeedsFullScan`]，由前端决定是否完整重扫。

use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use tauri::State;

use crate::scan::{AppState, ScanMode, Scanned, volume_root};

#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Refresh {
    /// 已刷新。没有变化时各项都为 0。
    Updated {
        added: u64,
        removed: u64,
        updated: u64,
        elapsed_ms: u64,
    },
    /// 不能增量刷新，需要完整重扫。
    NeedsFullScan { reason: String },
}

fn needs_full(reason: impl Into<String>) -> Refresh {
    Refresh::NeedsFullScan {
        reason: reason.into(),
    }
}

#[tauri::command]
pub async fn refresh_scan(state: State<'_, AppState>) -> Result<Refresh, String> {
    let scan = state.current()?;
    let Some(letter) = volume_root(&scan.summary.root) else {
        return Ok(needs_full("只有整个磁盘的扫描结果才能增量刷新"));
    };
    if scan.summary.mode != ScanMode::Mft {
        return Ok(needs_full("普通遍历的结果不能增量刷新"));
    }
    if scan.journal().is_none() {
        return Ok(needs_full("这个磁盘没有启用 USN 日志"));
    }

    tauri::async_runtime::spawn_blocking(move || run(scan, letter))
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn run(_: Arc<Scanned>, _: char) -> Refresh {
    needs_full("只在 Windows 上支持")
}

#[cfg(windows)]
fn run(scan: Arc<Scanned>, letter: char) -> Refresh {
    use dl_ntfs::usn::{self, FileSizer, NodeIndex};

    let started = Instant::now();
    // 同一时间只做一次刷新；索引锁同时充当刷新锁。
    let mut index = scan.index.lock().unwrap_or_else(|e| e.into_inner());
    let Some(pos) = scan.journal() else {
        return needs_full("这个磁盘没有启用 USN 日志");
    };
    let volume = match dl_ntfs::open_volume(letter) {
        Ok(v) => v,
        Err(e) => return needs_full(e.to_string()),
    };
    let (records, next) = match usn::read_journal(&volume, pos) {
        Ok(r) => r,
        Err(e) => return needs_full(e.to_string()),
    };
    let changes = usn::fold(&records);

    // 先查好文件大小，再拿写锁，尽量缩短界面被阻塞的时间。
    let sizes: Vec<Option<(u64, u64)>> = match FileSizer::open(letter) {
        Ok(sizer) => changes
            .iter()
            .map(|c| (!c.deleted && !c.is_dir()).then(|| sizer.size(c)).flatten())
            .collect(),
        Err(_) => vec![None; changes.len()],
    };
    let size_at: std::collections::HashMap<u64, (u64, u64)> = changes
        .iter()
        .zip(&sizes)
        .filter_map(|(c, s)| s.map(|s| (c.frn, s)))
        .collect();

    let stats = {
        let mut tree = scan.tree.write().expect("tree poisoned");
        let index = index.get_or_insert_with(|| NodeIndex::build(&tree));
        usn::apply(&mut tree, index, &changes, |c| size_at.get(&c.frn).copied())
    };
    *scan.journal.lock().expect("journal poisoned") = Some(next);

    let changed = stats.added + stats.removed + stats.updated > 0;
    if changed {
        scan.breakdowns.lock().expect("cache poisoned").clear();
    }
    drop(index);
    if changed || next != pos {
        crate::cache::save_in_background(Arc::clone(&scan));
    }
    Refresh::Updated {
        added: stats.added,
        removed: stats.removed,
        updated: stats.updated,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}
