//! 扫描结果缓存：每次扫描或删除后在后台把文件树存到磁盘，下次启动时立即显示上次的结果。
//!
//! 缓存放在 `%LOCALAPPDATA%\DiskLens\cache`，每个扫描路径一个 `.dlscan` 文件（见 dl-snapshot）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::scan::{AppState, ScanMode, ScanSummary, Scanned};

const EXT: &str = "dlscan";

/// 同一时间只写一个缓存文件，免得两次保存争用同一个临时文件。
static SAVING: Mutex<()> = Mutex::new(());

/// 缓存文件头里的描述信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheMeta {
    pub root: String,
    pub mode: ScanMode,
    pub fallback_reason: Option<String>,
    pub skipped: u64,
    pub elapsed_ms: u64,
    /// 扫描完成的时间（Unix 毫秒）。
    pub scanned_at: u64,
    pub total_size: u64,
    pub file_count: u32,
}

/// 首页列出的一条缓存。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedScan {
    pub root: String,
    pub mode: ScanMode,
    pub scanned_at: u64,
    pub total_size: u64,
    pub file_count: u32,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("DiskLens").join("cache"))
}

/// 扫描路径对应的缓存文件名：可读的前缀加上规范化路径的哈希，避免不同路径撞名。
fn file_name(root: &str) -> String {
    let key = root.trim().trim_end_matches(['\\', '/']).to_lowercase();
    let readable: String = key
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    // FNV-1a，64 位。
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{readable}-{hash:016x}.{EXT}")
}

fn cache_path(root: &str) -> Result<PathBuf, String> {
    cache_dir()
        .map(|d| d.join(file_name(root)))
        .ok_or_else(|| "找不到 LOCALAPPDATA 文件夹".to_string())
}

/// 在后台线程里把当前结果写入缓存。失败只记录到标准错误，不影响使用。
pub fn save_in_background(scan: Arc<Scanned>) {
    std::thread::spawn(move || {
        let _guard = SAVING.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = save(&scan) {
            eprintln!("保存扫描缓存失败：{e}");
        }
    });
}

fn save(scan: &Scanned) -> Result<(), String> {
    let path = cache_path(&scan.summary.root)?;
    let tree = scan.read();
    let s = &scan.summary;
    let meta = CacheMeta {
        root: s.root.clone(),
        mode: s.mode,
        fallback_reason: s.fallback_reason.clone(),
        skipped: s.skipped,
        elapsed_ms: s.elapsed_ms,
        scanned_at: s.scanned_at,
        total_size: tree.size(tree.root()),
        file_count: tree.file_count(tree.root()),
    };
    dl_snapshot::save(&path, &meta, &tree).map_err(|e| e.to_string())
}

fn list(dir: &Path) -> Vec<CachedScan> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut scans: Vec<CachedScan> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXT))
        .filter_map(|p| dl_snapshot::read_meta::<CacheMeta>(&p).ok())
        .map(|m| CachedScan {
            root: m.root,
            mode: m.mode,
            scanned_at: m.scanned_at,
            total_size: m.total_size,
            file_count: m.file_count,
        })
        .collect();
    scans.sort_by_key(|s| std::cmp::Reverse(s.scanned_at));
    scans
}

/// 所有缓存的扫描结果，最近的在前。
#[tauri::command]
pub fn cached_scans() -> Vec<CachedScan> {
    cache_dir().map(|d| list(&d)).unwrap_or_default()
}

/// 打开 `root` 的缓存结果，替换当前结果。
#[tauri::command]
pub async fn open_cached(root: String, state: State<'_, AppState>) -> Result<ScanSummary, String> {
    let path = cache_path(&root)?;
    let (meta, tree) =
        tauri::async_runtime::spawn_blocking(move || dl_snapshot::load::<CacheMeta>(&path))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("读取上次的扫描结果失败：{e}"))?;

    let summary = ScanSummary {
        root: meta.root,
        root_id: 0,
        mode: meta.mode,
        fallback_reason: meta.fallback_reason,
        total_size: 0,
        total_allocated: 0,
        file_count: 0,
        node_count: 0,
        skipped: meta.skipped,
        elapsed_ms: meta.elapsed_ms,
        scanned_at: meta.scanned_at,
        from_cache: true,
    };
    let scanned = Scanned::new(tree, summary);
    let summary = scanned.summary.clone();
    *state.scan.lock().expect("state poisoned") = Some(Arc::new(scanned));
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn file_names_are_stable_and_distinct() {
        assert_eq!(file_name(r"D:\"), file_name("d:"));
        assert_ne!(file_name(r"D:\a_b"), file_name(r"D:\a b"));
        assert!(file_name(r"D:\").starts_with("d_-"));
        assert!(file_name(&"x".repeat(300)).len() < 80);
    }
}
