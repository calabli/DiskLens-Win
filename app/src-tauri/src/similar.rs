//! 相似图片：后台计算感知哈希并分组，用 Channel 推送进度；另提供预览用的缩略图。
//!
//! 指纹缓存放在 `%LOCALAPPDATA%\DiskLens\cache\phash.dlhash`，第一次查找时读入，每次查找后保存。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use dl_analyze::similar::{self, PhashCache};
use dl_tree::{NodeId, NodeKind};
use serde::Serialize;
use tauri::State;
use tauri::ipc::{Channel, Response};

use crate::commands::check;
use crate::scan::AppState;

const PROGRESS_INTERVAL_MS: u64 = 100;
/// 最多返回多少组。
const MAX_GROUPS: usize = 1_000;
/// 缩略图长边的上限（像素）。
const MAX_THUMB: u32 = 512;

static PHASHES: OnceLock<PhashCache> = OnceLock::new();
static CANCEL: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

fn phash_path() -> Option<PathBuf> {
    crate::cache::cache_dir().map(|d| d.join("phash.dlhash"))
}

fn phashes() -> &'static PhashCache {
    PHASHES.get_or_init(|| {
        phash_path()
            .map(|p| PhashCache::load(&p))
            .unwrap_or_default()
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarProgress {
    done: u64,
    total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarImage {
    id: NodeId,
    path: String,
    size: u64,
    modified_ms: u64,
    width: u32,
    height: u32,
    /// 与组里第一张（画质最好的）相差的位数。
    distance: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarGroup {
    files: Vec<SimilarImage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarResult {
    groups: Vec<SimilarGroup>,
    more_groups: usize,
    /// 全部组只保留第一张时能释放的空间。
    wasted: u64,
    images: usize,
    skipped: u64,
    decoded: u64,
    elapsed_ms: u64,
}

/// 在目录 `id` 下查找相似图片：不小于 `min_size` 字节，指纹相差不超过 `max_distance` 位。
#[tauri::command]
pub async fn find_similar(
    id: NodeId,
    min_size: u64,
    max_distance: u32,
    on_event: Channel<SimilarProgress>,
    state: State<'_, AppState>,
) -> Result<SimilarResult, String> {
    let scan = state.current()?;
    let candidates = {
        let tree = scan.read();
        check(&tree, id)?;
        similar::candidates(&tree, id, min_size)
    };

    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(old) = CANCEL
        .lock()
        .expect("cancel poisoned")
        .replace(Arc::clone(&cancel))
    {
        old.store(true, Ordering::Relaxed);
    }

    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let last_sent = AtomicU64::new(0);
        let progress = |done: u64, total: u64| -> bool {
            let now = started.elapsed().as_millis() as u64;
            let last = last_sent.load(Ordering::Relaxed);
            if (done == total || now >= last + PROGRESS_INTERVAL_MS)
                && last_sent
                    .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
            {
                let _ = on_event.send(SimilarProgress { done, total });
            }
            !cancel.load(Ordering::Relaxed)
        };
        let cache = phashes();
        let report = similar::find_similar(
            &candidates,
            cache,
            max_distance.min(64),
            dl_analyze::dupes::default_threads(),
            &progress,
        );
        // 取消时也保存：已经算出的指纹下次可以直接用。
        if let Some(path) = phash_path() {
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
            .map(|g| {
                let first = g.files[0].hash;
                SimilarGroup {
                    files: g
                        .files
                        .into_iter()
                        .map(|f| SimilarImage {
                            id: f.id,
                            path: f.path.to_string_lossy().into_owned(),
                            size: f.size,
                            modified_ms: f.modified_ms,
                            width: f.width,
                            height: f.height,
                            distance: similar::distance(first, f.hash),
                        })
                        .collect(),
                }
            })
            .collect();
        Ok(SimilarResult {
            groups,
            more_groups,
            wasted,
            images: candidates.len(),
            skipped: report.skipped,
            decoded: report.decoded,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_similar() {
    if let Some(flag) = CANCEL.lock().expect("cancel poisoned").as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

/// 图片 `id` 的缩略图（PNG），长边不超过 `max` 像素。
#[tauri::command]
pub async fn thumbnail(
    id: NodeId,
    max: u32,
    state: State<'_, AppState>,
) -> Result<Response, String> {
    let scan = state.current()?;
    let path = {
        let tree = scan.read();
        check(&tree, id)?;
        if tree.kind(id) != NodeKind::File {
            return Err("不是文件".into());
        }
        PathBuf::from(tree.path(id))
    };
    let max = max.clamp(16, MAX_THUMB);
    tauri::async_runtime::spawn_blocking(move || similar::thumbnail_png(&path, max))
        .await
        .map_err(|e| e.to_string())?
        .map(Response::new)
        .map_err(|e| format!("无法生成预览：{e}"))
}
