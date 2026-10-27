//! 查重：在当前结果的某个目录下找内容相同的文件。耗时较长，在后台线程里运行，用 Channel 推送进度。
//!
//! 哈希缓存放在 `%LOCALAPPDATA%\DiskLens\cache\hashes.dlhash`，第一次查重时读入，每次查重后保存。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use dl_analyze::dupes::{self, HashCache, Phase};
use dl_tree::NodeId;
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use crate::commands::check;
use crate::scan::AppState;

/// 两次进度推送之间至少间隔这么多毫秒。
const PROGRESS_INTERVAL_MS: u64 = 100;
/// 最多返回多少组（按可释放空间从大到小）。
const MAX_GROUPS: usize = 2_000;

static HASHES: OnceLock<HashCache> = OnceLock::new();
/// 正在进行的查重的取消标志。
static CANCEL: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

fn hash_path() -> Option<PathBuf> {
    crate::cache::cache_dir().map(|d| d.join("hashes.dlhash"))
}

fn hashes() -> &'static HashCache {
    HASHES.get_or_init(|| hash_path().map(|p| HashCache::load(&p)).unwrap_or_default())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupProgress {
    /// `checking`：核对文件、计算部分哈希；`hashing`：计算全文哈希。
    phase: &'static str,
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupFile {
    id: NodeId,
    path: String,
    modified_ms: u64,
    /// 硬链接数；大于 1 时删除这个名称不释放空间。
    links: u32,
    /// 所在卷的序列号，只有同一个卷上的文件才能替换为硬链接。
    volume: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupGroup {
    size: u64,
    files: Vec<DupFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupResult {
    groups: Vec<DupGroup>,
    /// 还有多少组因数量限制没有返回。
    more_groups: usize,
    /// 全部组只保留一份时能释放的空间。
    wasted: u64,
    /// 大小与别的文件相同、需要进一步核对的文件数。
    candidates: usize,
    skipped: u64,
    same_file: u64,
    bytes_read: u64,
    elapsed_ms: u64,
}

/// 在目录 `id` 的子树里查找不小于 `min_size` 字节的重复文件。被取消时返回错误“已取消”。
#[tauri::command]
pub async fn find_duplicates(
    id: NodeId,
    min_size: u64,
    on_event: Channel<DupProgress>,
    state: State<'_, AppState>,
) -> Result<DupResult, String> {
    let scan = state.current()?;
    let candidates = {
        let tree = scan.read();
        check(&tree, id)?;
        dupes::candidates(&tree, id, min_size)
    };

    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(old) = CANCEL
        .lock()
        .expect("cancel poisoned")
        .replace(Arc::clone(&cancel))
    {
        old.store(true, Ordering::Relaxed);
    }

    let flag = Arc::clone(&cancel);
    tauri::async_runtime::spawn_blocking(move || run(&candidates, &on_event, &flag))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_duplicates() {
    if let Some(flag) = CANCEL.lock().expect("cancel poisoned").as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

fn run(
    candidates: &[dupes::Candidate],
    events: &Channel<DupProgress>,
    cancel: &AtomicBool,
) -> Result<DupResult, String> {
    let started = Instant::now();
    let last_sent = AtomicU64::new(0);
    let progress = |p: &dupes::Progress| -> bool {
        let now = started.elapsed().as_millis() as u64;
        let last = last_sent.load(Ordering::Relaxed);
        let finished = p.files_done == p.files_total;
        if (finished || now >= last + PROGRESS_INTERVAL_MS)
            && last_sent
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            let _ = events.send(DupProgress {
                phase: match p.phase {
                    Phase::Checking => "checking",
                    Phase::Hashing => "hashing",
                },
                files_done: p.files_done,
                files_total: p.files_total,
                bytes_done: p.bytes_done,
                bytes_total: p.bytes_total,
            });
        }
        !cancel.load(Ordering::Relaxed)
    };

    let cache = hashes();
    let report = dupes::find_duplicates(candidates, cache, dupes::default_threads(), &progress);
    // 取消时也保存：已经算出的哈希下次可以直接用。
    if let Some(path) = hash_path() {
        let _ = cache.save(&path);
    }
    let report = report.map_err(|e| {
        if e.kind() == std::io::ErrorKind::Interrupted {
            "已取消".to_string()
        } else {
            e.to_string()
        }
    })?;

    let wasted = report.groups.iter().map(|g| g.wasted()).sum();
    let more_groups = report.groups.len().saturating_sub(MAX_GROUPS);
    let groups = report
        .groups
        .into_iter()
        .take(MAX_GROUPS)
        .map(|g| DupGroup {
            size: g.size,
            files: g
                .files
                .into_iter()
                .map(|f| DupFile {
                    id: f.id,
                    path: f.path.to_string_lossy().into_owned(),
                    modified_ms: f.modified_ms,
                    links: f.links,
                    volume: f.volume,
                })
                .collect(),
        })
        .collect();
    Ok(DupResult {
        groups,
        more_groups,
        wasted,
        candidates: candidates.len(),
        skipped: report.skipped,
        same_file: report.same_file,
        bytes_read: report.bytes_read,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}
