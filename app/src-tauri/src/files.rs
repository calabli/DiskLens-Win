//! 文件管理：把文件夹移到其他磁盘（原位置留连接点）、在资源管理器中显示、查询所有者。

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

/// 各项目的所有者（“域\用户名”或用户名）；查不到的为 `None`。逐个读安全描述符，
/// 只给界面上看得见的几十行用。
#[tauri::command]
pub async fn owners(
    ids: Vec<NodeId>,
    state: State<'_, AppState>,
) -> Result<Vec<Option<String>>, String> {
    let scan = state.current()?;
    let paths: Vec<Option<String>> = {
        let t = scan.read();
        ids.iter()
            .map(|&id| check(&t, id).ok().map(|()| t.path(id)))
            .collect()
    };
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .iter()
            .map(|p| p.as_deref().and_then(owner_of))
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn owner_of(path: &str) -> Option<String> {
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        LookupAccountSidW, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    };

    let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
    let mut owner: PSID = std::ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `wide` 以 0 结尾；输出指针都指向有效的局部变量；成功时 `sd` 需要用 LocalFree 释放。
    let err = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    let (mut name, mut domain) = ([0u16; 256], [0u16; 256]);
    let (mut name_len, mut domain_len, mut kind) = (256u32, 256u32, 0);
    // SAFETY: `owner` 指向 `sd` 里的 SID，在释放 `sd` 之前有效；缓冲区长度如实传入。
    let ok = unsafe {
        LookupAccountSidW(
            std::ptr::null(),
            owner,
            name.as_mut_ptr(),
            &mut name_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut kind,
        )
    };
    // SAFETY: `sd` 由 GetNamedSecurityInfoW 分配。
    unsafe { LocalFree(sd) };
    if ok == 0 {
        return None;
    }
    let name = String::from_utf16_lossy(&name[..name_len as usize]);
    let domain = String::from_utf16_lossy(&domain[..domain_len as usize]);
    Some(if domain.is_empty() {
        name
    } else {
        format!(r"{domain}\{name}")
    })
}

#[cfg(not(windows))]
fn owner_of(_: &str) -> Option<String> {
    None
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
