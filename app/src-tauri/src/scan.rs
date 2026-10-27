//! 扫描：在后台线程里运行，用 Channel 推送进度，结果留在 [`AppState`] 里供后续查询。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::time::Instant;

use dl_analyze::{Category, ChildBreakdown};
use dl_ntfs::usn::{JournalPos, NodeIndex};
use dl_tree::{FileTree, NodeId};
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

/// 两次进度推送之间至少间隔这么多毫秒，避免把前端淹没。
const PROGRESS_INTERVAL_MS: u64 = 100;
/// 边扫边看：两次推送顶层项目快照之间至少间隔这么多毫秒。
const LIVE_INTERVAL_MS: u64 = 400;
/// 快照里最多列出几个顶层项目（最大的）。
const LIVE_ITEMS: usize = 40;

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
    /// 普通遍历中，各顶层项目目前累计的大小（最大的若干项，从大到小）。
    Live {
        items: Vec<LiveEntry>,
        /// 目前累计的总大小（含没有列出的项目）。
        total_size: u64,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub files: u64,
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
    /// 读不了的文件夹（最多记 [`dl_walk::MAX_UNREADABLE`] 个）。
    pub unreadable: Vec<UnreadableDir>,
    pub elapsed_ms: u64,
    /// 扫描完成的时间（Unix 毫秒）。
    pub scanned_at: u64,
    /// 是否是从缓存读回的上次结果。
    pub from_cache: bool,
}

/// 一个读不了的文件夹和原因，界面上单独列出。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadableDir {
    pub path: String,
    pub reason: String,
}

impl UnreadableDir {
    fn new(u: &dl_walk::Unreadable) -> Self {
        Self {
            path: u.path.to_string_lossy().into_owned(),
            reason: reason(u.kind, u.code),
        }
    }
}

/// 把读目录失败的错误说成一句话。
fn reason(kind: std::io::ErrorKind, code: Option<i32>) -> String {
    use std::io::ErrorKind;
    match (code, kind) {
        (Some(5), _) | (None, ErrorKind::PermissionDenied) => "没有访问权限".into(),
        (Some(32), _) => "正被其他程序占用".into(),
        (Some(2 | 3), _) | (None, ErrorKind::NotFound) => "扫描时已经不存在".into(),
        (Some(21), _) => "设备未就绪".into(),
        (Some(1920), _) => "系统无法访问".into(),
        (Some(c), _) => format!("系统错误 {c}"),
        (None, k) => k.to_string(),
    }
}

/// 一次完成的扫描。删除文件后树会被修改，所以放在读写锁里。
pub struct Scanned {
    pub tree: RwLock<FileTree>,
    /// 扫描完成时的汇总；删除之后的最新数字以树为准。
    pub summary: ScanSummary,
    /// 每个目录的类型统计，计算一次后缓存；树变化时清空。
    pub breakdowns: Mutex<HashMap<NodeId, Arc<ChildBreakdown>>>,
    /// 旭日图里各目录的主要类型，按旭日图的中心目录缓存；树变化时清空。
    pub dominants: Mutex<HashMap<NodeId, Arc<HashMap<NodeId, Category>>>>,
    /// 树对应的 USN 日志位置；没有日志（或不是 MFT 扫描）时为 `None`。
    pub journal: Mutex<Option<JournalPos>>,
    /// 记录号到节点的索引，第一次增量刷新时建立。同时用作刷新的互斥锁。
    pub index: Mutex<Option<NodeIndex>>,
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
            dominants: Mutex::new(HashMap::new()),
            journal: Mutex::new(None),
            index: Mutex::new(None),
        }
    }

    pub fn journal(&self) -> Option<JournalPos> {
        *self.journal.lock().expect("journal poisoned")
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

    /// 以 `id` 为中心、`depth` 圈旭日图里各目录的主要类型。缓存方式同 [`breakdown`](Self::breakdown)。
    pub fn dominants(
        &self,
        tree: &FileTree,
        id: NodeId,
        depth: u32,
    ) -> Arc<HashMap<NodeId, Category>> {
        if let Some(d) = self.dominants.lock().expect("cache poisoned").get(&id) {
            return Arc::clone(d);
        }
        let d = Arc::new(dl_analyze::dominant_dirs(tree, id, depth));
        self.dominants
            .lock()
            .expect("cache poisoned")
            .insert(id, Arc::clone(&d));
        d
    }

    /// 树被修改后清空按树计算的缓存。
    pub fn invalidate(&self) {
        self.breakdowns.lock().expect("cache poisoned").clear();
        self.dominants.lock().expect("cache poisoned").clear();
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
    let live = dl_walk::Live::default();
    let live_sent = AtomicU64::new(0);
    // 节流后推送进度和顶层项目快照；返回 false 表示要取消。
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
        let last = live_sent.load(Ordering::Relaxed);
        if now >= last + LIVE_INTERVAL_MS
            && live_sent
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            send_live(events, &live);
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
        // 先记下日志位置再扫描：扫描期间的变化在下次刷新时重放，结果不受影响。
        let journal = dl_ntfs::open_volume(letter)
            .ok()
            .and_then(|v| dl_ntfs::usn::query_journal(&v).ok())
            .map(|(pos, _)| pos);
        match dl_ntfs::scan_volume(letter, report) {
            Ok(scan) => {
                let scanned = finish(path, ScanMode::Mft, None, scan.tree, 0, Vec::new(), started);
                *scanned.journal.lock().expect("journal poisoned") = journal;
                return Ok(scanned);
            }
            Err(dl_ntfs::Error::Cancelled) => return Err("已取消".into()),
            Err(e) => fallback_reason = Some(e.to_string()),
        }
    }

    let _ = events.send(ScanEvent::Started {
        mode: ScanMode::Walk,
        fallback_reason: fallback_reason.clone(),
    });
    let result = dl_walk::walk_live(&PathBuf::from(&path), &report, &live).map_err(|e| {
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
        result.unreadable.iter().map(UnreadableDir::new).collect(),
        started,
    ))
}

/// 推送一次顶层项目快照；还没有任何项目时不推送（例如 MFT 直读）。
fn send_live(events: &Channel<ScanEvent>, live: &dl_walk::Live) {
    let mut items = live.snapshot();
    if items.is_empty() {
        return;
    }
    let total_size = items.iter().map(|i| i.size).sum();
    items.sort_by_key(|i| std::cmp::Reverse(i.size));
    let items = items
        .into_iter()
        .take(LIVE_ITEMS)
        .map(|i| LiveEntry {
            name: i.name,
            is_dir: i.is_dir,
            size: i.size,
            files: i.files,
        })
        .collect();
    let _ = events.send(ScanEvent::Live { items, total_size });
}

fn finish(
    root: String,
    mode: ScanMode,
    fallback_reason: Option<String>,
    tree: FileTree,
    skipped: u64,
    unreadable: Vec<UnreadableDir>,
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
        unreadable,
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
