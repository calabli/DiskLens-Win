//! 空文件夹、空文件、失效链接和快捷方式：在后台核实，用 Channel 推送进度。
//! 受保护路径下的结果直接略去，反正也删不了。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use dl_analyze::findings::{self, Kind};
use dl_tree::NodeId;
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use crate::commands::check;
use crate::scan::AppState;

const PROGRESS_INTERVAL_MS: u64 = 100;
/// 每一类最多返回这么多项。
const MAX_PER_KIND: usize = 5_000;

static CANCEL: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindProgress {
    done: u64,
    total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingOut {
    id: NodeId,
    path: String,
    /// `emptyDir`、`emptyFile`、`brokenLink`、`brokenShortcut`。
    kind: &'static str,
    target: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindResult {
    items: Vec<FindingOut>,
    /// 因数量限制没有返回的项数。
    more: usize,
    checked: usize,
    elapsed_ms: u64,
}

#[tauri::command]
pub async fn find_issues(
    id: NodeId,
    on_event: Channel<FindProgress>,
    state: State<'_, AppState>,
) -> Result<FindResult, String> {
    let scan = state.current()?;
    let candidates = {
        let tree = scan.read();
        check(&tree, id)?;
        findings::candidates(&tree, id)
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
                let _ = on_event.send(FindProgress { done, total });
            }
            !cancel.load(Ordering::Relaxed)
        };
        let found =
            findings::verify_all(&candidates, dl_analyze::dupes::default_threads(), &progress)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        "已取消".to_string()
                    } else {
                        e.to_string()
                    }
                })?;

        let protector = crate::cleanup::protector();
        let mut per_kind = [0usize; 4];
        let mut more = 0;
        let mut items = Vec::new();
        for f in found {
            if protector.check(&f.path).is_some() {
                continue;
            }
            let (slot, kind) = match f.kind {
                Kind::EmptyDir => (0, "emptyDir"),
                Kind::EmptyFile => (1, "emptyFile"),
                Kind::BrokenLink => (2, "brokenLink"),
                Kind::BrokenShortcut => (3, "brokenShortcut"),
            };
            per_kind[slot] += 1;
            if per_kind[slot] > MAX_PER_KIND {
                more += 1;
                continue;
            }
            items.push(FindingOut {
                id: f.id,
                path: f.path.to_string_lossy().into_owned(),
                kind,
                target: f.target,
            });
        }
        Ok(FindResult {
            items,
            more,
            checked: candidates.len(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_issues() {
    if let Some(flag) = CANCEL.lock().expect("cancel poisoned").as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}
