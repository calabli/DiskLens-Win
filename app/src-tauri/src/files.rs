//! 文件管理：把文件夹移到其他磁盘（原位置留连接点），以及在资源管理器中显示。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use dl_clean::{LogEntry, Outcome};
use dl_tree::{NodeId, NodeKind};
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use crate::cleanup::protector;
use crate::commands::check;
use crate::scan::AppState;

const PROGRESS_INTERVAL_MS: u64 = 100;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveProgress {
    done: u64,
    total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveResult {
    /// 原位置现在是连接点，在树上换成了一个新的空节点。
    new_id: NodeId,
    dest: String,
    bytes: u64,
    files: u64,
    /// 旧文件夹没能完全删除时的位置。
    leftover: Option<String>,
    log_error: Option<String>,
}

/// 把文件夹 `id` 移到 `dest_dir` 下（保留原名），原位置留下连接点。
#[tauri::command]
pub async fn move_folder(
    id: NodeId,
    dest_dir: String,
    on_event: Channel<MoveProgress>,
    state: State<'_, AppState>,
) -> Result<MoveResult, String> {
    let scan = state.current()?;
    let (src, name) = {
        let t = scan.read();
        check(&t, id)?;
        if t.kind(id) != NodeKind::Dir || t.parent(id).is_none() {
            return Err("只能移动扫描范围内的文件夹".into());
        }
        (PathBuf::from(t.path(id)), t.name(id).to_owned())
    };
    if let Some(reason) = protector().check(&src) {
        return Err(reason);
    }
    let dest = PathBuf::from(dest_dir.trim()).join(&name);

    let (result, log_error) = {
        let (src, dest) = (src.clone(), dest.clone());
        tauri::async_runtime::spawn_blocking(move || {
            let started = Instant::now();
            let last = AtomicU64::new(0);
            let mut progress = |done: u64, total: u64| {
                let now = started.elapsed().as_millis() as u64;
                if done == total || now >= last.load(Ordering::Relaxed) + PROGRESS_INTERVAL_MS {
                    last.store(now, Ordering::Relaxed);
                    let _ = on_event.send(MoveProgress { done, total });
                }
                true
            };
            let result = dl_clean::relocate::move_with_junction(&src, &dest, &mut progress);
            let (size, outcome) = match &result {
                Ok(m) => (m.bytes, Outcome::Moved),
                Err(e) => (0, Outcome::Failed(e.clone())),
            };
            let log = dl_clean::OpLog::new(dl_clean::OpLog::default_dir());
            let log_error = log
                .append(&LogEntry::moved(&src, &dest, size, &outcome))
                .err()
                .map(|e| format!("操作日志写入失败：{e}"));
            (result, log_error)
        })
        .await
        .map_err(|e| e.to_string())?
    };
    let moved = result?;

    // 原位置现在是连接点：在树上换成一个同名的空目录，大小不再计入这个磁盘。
    let new_id = {
        let mut t = scan.tree.write().expect("tree poisoned");
        let parent = t.parent(id).expect("checked above");
        t.detach(id);
        t.insert(parent, &name, NodeKind::Dir, 0, 0)
    };
    scan.invalidate();
    crate::cache::save_in_background(scan);
    Ok(MoveResult {
        new_id,
        dest: dest.to_string_lossy().into_owned(),
        bytes: moved.bytes,
        files: moved.files,
        leftover: moved.leftover.map(|p| p.to_string_lossy().into_owned()),
        log_error,
    })
}

/// 打开资源管理器并选中这一项。
#[tauri::command]
pub fn reveal_in_explorer(id: NodeId, state: State<'_, AppState>) -> Result<(), String> {
    let scan = state.current()?;
    let path = {
        let t = scan.read();
        check(&t, id)?;
        t.path(id)
    };
    open_explorer(&path).map_err(|e| format!("无法打开资源管理器：{e}"))
}

#[cfg(windows)]
fn open_explorer(path: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    // explorer 自己解析参数，路径要整个放在引号里。
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{path}\""))
        .spawn()
        .map(|_| ())
}

#[cfg(not(windows))]
fn open_explorer(_: &str) -> std::io::Result<()> {
    Err(std::io::Error::other("只在 Windows 上支持"))
}
