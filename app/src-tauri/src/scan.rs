//! 扫描：在后台线程里运行，用 Channel 推送进度，结果留在 [`AppState`] 里供后续查询。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::time::Instant;

use dl_analyze::ChildBreakdown;
use dl_tree::{FileTree, NodeId};
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

/// 两次进度推送之间至少间隔这么多毫秒，避免把前端淹没。
const PROGRESS_INTERVAL_MS: u64 = 100;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ScanMode {
    /// 直读 NTFS MFT。
    Mft,
    /// 逐个目录遍历。
    Walk,
}

/// 推送给前端的扫描进度。`rename_all` 只改变体名，字段名要靠 `rename_all_fields`。
#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum ScanEvent {
    /// 开始以某种方式扫描（MFT 失败转普通遍历时会再发一次）。
    Started {
        mode: ScanMode,
        fallback_reason: Option<String>,
    },
    /// MFT：已解析的记录数；普通遍历：已处理的条目数。
    Progress { processed: u64, elapsed_ms: u64 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub root: String,
    pub root_id: NodeId,
    pub mode: ScanMode,
    pub fallback_reason: Option<String>,
    pub total_size: u64,
    pub total_allocated: u64,
    pub file_count: u32,
    pub node_count: usize,
    pub skipped: u64,
    pub elapsed_ms: u64,
    /// 扫描完成的时间（Unix 毫秒）。
    pub scanned_at: u64,
    /// 是否是从缓存读回的上次结果。
    pub from_cache: bool,
}

/// 一次完成的扫描。删除文件后树会被修改，所以放在读写锁里。
pub struct Scanned {
    pub tree: RwLock<FileTree>,
    /// 扫描完成时的汇总；删除之后的最新数字以树为准。
    pub summary: ScanSummary,
    /// 每个目录的类型统计，计算一次后缓存；树变化时清空。
    pub breakdowns: Mutex<HashMap<NodeId, Arc<ChildBreakdown>>>,
}

impl Scanned {
    /// 用扫描或缓存得到的树建立结果。`summary` 里的总大小、文件数和节点数以树为准。
    pub fn new(tree: FileTree, mut summary: ScanSummary) -> Self {
        let top = tree.root();
        summary.root_id = top;
        summary.total_size = tree.size(top);
        summary.total_allocated = tree.allocated(top);
        summary.file_count = tree.file_count(top);
        summary.node_count = tree.len();
        Self {
            tree: RwLock::new(tree),
            summary,
            breakdowns: Mutex::new(HashMap::new()),
        }
    }

    pub fn read(&self) -> RwLockReadGuard<'_, FileTree> {
        self.tree.read().expect("tree poisoned")
    }

    /// `id` 的类型统计。`tree` 是调用方已经持有的读锁，避免重复加锁。
    pub fn breakdown(&self, tree: &FileTree, id: NodeId) -> Arc<ChildBreakdown> {
        if let Some(b) = self.breakdowns.lock().expect("cache poisoned").get(&id) {
            return Arc::clone(b);
        }
        // 计算可能较慢，不持缓存锁计算；并发时最多重复算一次。
        let b = Arc::new(dl_analyze::breakdown(tree, id));
        self.breakdowns
            .lock()
            .expect("cache poisoned")
            .insert(id, Arc::clone(&b));
        b
    }
}

#[derive(Default)]
pub struct AppState {
    pub scan: Mutex<Option<Arc<Scanned>>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

impl AppState {
    pub fn current(&self) -> Result<Arc<Scanned>, String> {
        self.scan
            .lock()
            .expect("state poisoned")
            .clone()
            .ok_or_else(|| "还没有扫描结果".to_string())
    }
}

/// `D:`、`D:\` 这样的卷根目录返回盘符，其他路径返回 `None`。
pub fn volume_root(path: &str) -> Option<char> {
    let mut chars = path.trim().chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    match (chars.next(), chars.next(), chars.next()) {
        (Some(':'), None | Some('\\' | '/'), None) => Some(letter),
        _ => None,
    }
}

/// 开始扫描。之前未完成的扫描会被取消。完成后返回汇总；被取消时返回错误“已取消”。
#[tauri::command]
pub async fn start_scan(
    path: String,
    on_event: Channel<ScanEvent>,
    state: State<'_, AppState>,
) -> Result<ScanSummary, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(old) = state
        .cancel
        .lock()
        .expect("state poisoned")
        .replace(Arc::clone(&cancel))
    {
        old.store(true, Ordering::Relaxed);
    }

    let flag = Arc::clone(&cancel);
    let scanned = tauri::async_runtime::spawn_blocking(move || run_scan(path, &on_event, &flag))
        .await
        .map_err(|e| e.to_string())??;

    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".into());
    }
    let summary = scanned.summary.clone();
    let scanned = Arc::new(scanned);
    *state.scan.lock().expect("state poisoned") = Some(Arc::clone(&scanned));
    crate::cache::save_in_background(scanned);
    Ok(summary)
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>) {
    if let Some(flag) = state.cancel.lock().expect("state poisoned").as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

fn run_scan(
    path: String,
    events: &Channel<ScanEvent>,
    cancel: &AtomicBool,
) -> Result<Scanned, String> {
    let started = Instant::now();
    let last_sent = AtomicU64::new(0);
    // 节流后推送进度；返回 false 表示要取消。
    let report = |processed: u64| -> bool {
        let now = started.elapsed().as_millis() as u64;
        let last = last_sent.load(Ordering::Relaxed);
        if now >= last + PROGRESS_INTERVAL_MS
            && last_sent
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            let _ = events.send(ScanEvent::Progress {
                processed,
                elapsed_ms: now,
            });
        }
        !cancel.load(Ordering::Relaxed)
    };

    let mut fallback_reason = None;

    #[cfg(windows)]
    if let Some(letter) = volume_root(&path) {
        let _ = events.send(ScanEvent::Started {
            mode: ScanMode::Mft,
            fallback_reason: None,
        });
        match dl_ntfs::scan_volume(letter, report) {
            Ok(scan) => {
                return Ok(finish(path, ScanMode::Mft, None, scan.tree, 0, started));
            }
            Err(dl_ntfs::Error::Cancelled) => return Err("已取消".into()),
            Err(e) => fallback_reason = Some(e.to_string()),
        }
    }

    let _ = events.send(ScanEvent::Started {
        mode: ScanMode::Walk,
        fallback_reason: fallback_reason.clone(),
    });
    let result = dl_walk::walk_with_progress(&PathBuf::from(&path), &report).map_err(|e| {
        if e.kind() == std::io::ErrorKind::Interrupted {
            "已取消".to_string()
        } else {
            format!("无法扫描 {path}：{e}")
        }
    })?;
    Ok(finish(
        path,
        ScanMode::Walk,
        fallback_reason,
        result.tree,
        result.skipped,
        started,
    ))
}

fn finish(
    root: String,
    mode: ScanMode,
    fallback_reason: Option<String>,
    tree: FileTree,
    skipped: u64,
    started: Instant,
) -> Scanned {
    let summary = ScanSummary {
        root,
        root_id: 0,
        mode,
        fallback_reason,
        total_size: 0,
        total_allocated: 0,
        file_count: 0,
        node_count: 0,
        skipped,
        elapsed_ms: started.elapsed().as_millis() as u64,
        scanned_at: crate::cache::now_ms(),
        from_cache: false,
    };
    Scanned::new(tree, summary)
}

#[cfg(test)]
mod tests {
    use super::{ScanEvent, ScanMode, volume_root};

    #[test]
    fn events_use_camel_case_fields() {
        let json = serde_json::to_string(&ScanEvent::Progress {
            processed: 3,
            elapsed_ms: 40,
        })
        .unwrap();
        assert_eq!(json, r#"{"kind":"progress","processed":3,"elapsedMs":40}"#);
        let json = serde_json::to_string(&ScanEvent::Started {
            mode: ScanMode::Walk,
            fallback_reason: Some("x".into()),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"kind":"started","mode":"walk","fallbackReason":"x"}"#
        );
    }

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
