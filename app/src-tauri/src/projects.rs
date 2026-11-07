//! 开发项目页面：扫描范围里的开发项目和它们可以删掉的构建产物，见 [`dl_analyze::projects`]。

use dl_tree::NodeId;
use serde::Serialize;
use tauri::State;

use crate::commands::check;
use crate::scan::AppState;

/// 最多列出的项目数。
const MAX_PROJECTS: usize = 1000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactInfo {
    id: NodeId,
    name: String,
    size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    id: NodeId,
    path: String,
    kinds: Vec<&'static str>,
    size: u64,
    reclaimable: u64,
    /// 构建产物以外最近一次修改的时间（Unix 秒，0 表示不知道）。
    modified: u32,
    artifacts: Vec<ArtifactInfo>,
}

/// `id` 子树里的开发项目，按能释放的大小从大到小。
#[tauri::command]
pub async fn dev_projects(
    id: NodeId,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectInfo>, String> {
    let scan = state.current()?;
    tauri::async_runtime::spawn_blocking(move || {
        let t = scan.read();
        check(&t, id)?;
        Ok(dl_analyze::projects::projects(&t, id)
            .into_iter()
            .take(MAX_PROJECTS)
            .map(|p| ProjectInfo {
                reclaimable: p.reclaimable(),
                path: t.path(p.id),
                id: p.id,
                kinds: p.kinds,
                size: p.size,
                modified: p.modified,
                artifacts: p
                    .artifacts
                    .into_iter()
                    .map(|a| ArtifactInfo {
                        id: a.id,
                        name: a.name,
                        size: a.size,
                    })
                    .collect(),
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}
