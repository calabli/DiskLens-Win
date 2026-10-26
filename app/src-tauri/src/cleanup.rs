//! 删除：先给出汇总和拦截原因供确认，确认后执行，并把删掉的项从树上摘除。

use std::sync::OnceLock;

use dl_clean::{Mode, OpLog, Outcome, Protector, Target};
use dl_tree::{NodeId, NodeKind};
use serde::Serialize;
use tauri::State;

use crate::commands::check;
use crate::scan::AppState;

fn protector() -> &'static Protector {
    static P: OnceLock<Protector> = OnceLock::new();
    P.get_or_init(Protector::from_env)
}

fn op_log() -> OpLog {
    OpLog::new(OpLog::default_dir())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    id: NodeId,
    name: String,
    path: String,
    size: u64,
    files: u32,
    is_dir: bool,
    /// 不能删除时的原因。
    blocked: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlan {
    items: Vec<PlanItem>,
    /// 可以删除的项合计。
    total_size: u64,
    total_files: u64,
    log_dir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    path: String,
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    deleted: Vec<NodeId>,
    failed: Vec<Failure>,
    /// 已删除项的逻辑大小合计。
    freed: u64,
    log_error: Option<String>,
}

/// 删除前的汇总：每项的路径、大小，以及会不会被拦截。
#[tauri::command]
pub fn plan_delete(ids: Vec<NodeId>, state: State<'_, AppState>) -> Result<DeletePlan, String> {
    let scan = state.current()?;
    let t = scan.read();
    let mut items = Vec::new();
    for id in dedup(ids) {
        check(&t, id)?;
        let path = t.path(id);
        items.push(PlanItem {
            id,
            name: t.name(id).to_owned(),
            blocked: protector().check(path.as_ref()),
            path,
            size: t.size(id),
            files: t.file_count(id),
            is_dir: t.kind(id) == NodeKind::Dir,
        });
    }
    let ok = items.iter().filter(|i| i.blocked.is_none());
    Ok(DeletePlan {
        total_size: ok.clone().map(|i| i.size).sum(),
        total_files: ok
            .map(|i| u64::from(if i.is_dir { i.files } else { 1 }))
            .sum(),
        items,
        log_dir: op_log().dir().to_string_lossy().into_owned(),
    })
}

/// 执行删除。`permanent` 为 false 时移到回收站。
#[tauri::command]
pub async fn delete_items(
    ids: Vec<NodeId>,
    permanent: bool,
    state: State<'_, AppState>,
) -> Result<DeleteResult, String> {
    let scan = state.current()?;
    let (ids, targets): (Vec<NodeId>, Vec<Target>) = {
        let t = scan.read();
        let mut ids_out = Vec::new();
        let mut targets = Vec::new();
        for id in dedup(ids) {
            check(&t, id)?;
            ids_out.push(id);
            targets.push(Target {
                path: t.path(id).into(),
                size: t.size(id),
            });
        }
        (ids_out, targets)
    };
    let mode = if permanent {
        Mode::Permanent
    } else {
        Mode::Recycle
    };

    let (outcomes, log_error, targets) = tauri::async_runtime::spawn_blocking(move || {
        let (outcomes, log_error) = dl_clean::run(&targets, mode, protector(), &op_log());
        (outcomes, log_error, targets)
    })
    .await
    .map_err(|e| e.to_string())?;

    let mut result = DeleteResult {
        deleted: Vec::new(),
        failed: Vec::new(),
        freed: 0,
        log_error: log_error.map(|e| format!("操作日志写入失败：{e}")),
    };
    {
        let mut t = scan.tree.write().expect("tree poisoned");
        for ((id, outcome), target) in ids.into_iter().zip(outcomes).zip(targets) {
            match outcome {
                Outcome::Deleted => {
                    result.freed += target.size;
                    t.detach(id);
                    result.deleted.push(id);
                }
                Outcome::Blocked(reason) | Outcome::Failed(reason) => {
                    result.failed.push(Failure {
                        path: target.path.to_string_lossy().into_owned(),
                        reason,
                    });
                }
            }
        }
    }
    if !result.deleted.is_empty() {
        scan.breakdowns.lock().expect("cache poisoned").clear();
    }
    Ok(result)
}

/// 去重并保持顺序。
fn dedup(ids: Vec<NodeId>) -> Vec<NodeId> {
    let mut seen = std::collections::HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}
