//! 清理记录：把操作日志读回来，按批次分组；删除到回收站的项目标出现在是否还在回收站，
//! 还在的可以放回原处。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dl_clean::log::{self, Record};
use dl_clean::recycle::{self, BinItem};
use dl_clean::{LogEntry, OpLog, Outcome};
use serde::{Deserialize, Serialize};

/// 最多读回这么多条记录。
const MAX_RECORDS: usize = 3000;
/// 相邻两条记录动作相同、相隔不超过这么多秒，算同一批操作。
const BATCH_GAP: u64 = 10;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    time: String,
    path: String,
    size: u64,
    /// 日志里的结果：`deleted`、`linked`、`moved`、`restored`、`blocked`、`failed`。
    result: String,
    reason: Option<String>,
    link_to: Option<String>,
    /// 现在的状态，见 [`status`]。
    status: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryGroup {
    /// 这批操作开始的时间。
    time: String,
    /// `recycle`、`delete`、`hardlink`、`move`、`restore`。
    action: String,
    /// 成功处理的项的大小合计。
    size: u64,
    items: Vec<HistoryItem>,
}

/// 最近的清理记录，从新到旧。
#[tauri::command]
pub async fn cleanup_history() -> Result<Vec<HistoryGroup>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let records = OpLog::new(OpLog::default_dir()).read(MAX_RECORDS);
        let mut bins = Bins::default();
        let restored = restored_at(&records);
        group(records)
            .into_iter()
            .map(|(action, records)| {
                let items: Vec<HistoryItem> = records
                    .into_iter()
                    .map(|r| HistoryItem {
                        status: status(&r, &mut bins, &restored),
                        time: r.time,
                        path: r.path,
                        size: r.size,
                        result: r.result,
                        reason: r.reason,
                        link_to: r.link_to,
                    })
                    .collect();
                HistoryGroup {
                    time: items.first().map(|i| i.time.clone()).unwrap_or_default(),
                    size: items
                        .iter()
                        .filter(|i| !matches!(i.result.as_str(), "blocked" | "failed"))
                        .map(|i| i.size)
                        .sum(),
                    action,
                    items,
                }
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// 要放回的一项：日志里的时间和路径。
#[derive(Deserialize)]
pub struct RestoreKey {
    time: String,
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreFailure {
    path: String,
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    restored: Vec<String>,
    failed: Vec<RestoreFailure>,
    log_error: Option<String>,
}

/// 把回收站里的这些项目放回原处，每一项都写进日志。
#[tauri::command]
pub async fn restore_items(items: Vec<RestoreKey>) -> Result<RestoreResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let log = OpLog::new(OpLog::default_dir());
        let mut bins = Bins::default();
        let mut result = RestoreResult {
            restored: Vec::new(),
            failed: Vec::new(),
            log_error: None,
        };
        for key in items {
            let found = log::parse_time(&key.time).and_then(|t| bins.find(&key.path, t).cloned());
            let (size, outcome) = match found {
                None => (
                    0,
                    Outcome::Failed(
                        dl_i18n::tr("回收站里已经找不到它，可能回收站已被清空，或者已经放回")
                            .into(),
                    ),
                ),
                Some(item) => match recycle::restore(&item) {
                    Ok(()) => (item.size, Outcome::Restored),
                    Err(e) => (item.size, Outcome::Failed(e)),
                },
            };
            if let Err(e) = log.append(&LogEntry::restored(Path::new(&key.path), size, &outcome)) {
                result.log_error = Some(dl_i18n::trf!("写入操作日志失败：{e}", e = e));
            }
            match outcome {
                Outcome::Restored => result.restored.push(key.path),
                Outcome::Failed(reason) => result.failed.push(RestoreFailure {
                    path: key.path,
                    reason,
                }),
                _ => {}
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())
}

/// 把从新到旧的记录按批次分组：动作相同、与上一条相隔不超过 [`BATCH_GAP`] 秒的归为一批。
/// 每批里的记录按原来的先后顺序排列。
fn group(records: Vec<Record>) -> Vec<(String, Vec<Record>)> {
    let mut out: Vec<(String, Vec<Record>)> = Vec::new();
    let mut last_time: Option<u64> = None;
    for r in records {
        let t = log::parse_time(&r.time);
        let same = match (out.last(), last_time, t) {
            (Some((action, _)), Some(prev), Some(t)) => {
                *action == r.action && prev.saturating_sub(t) <= BATCH_GAP
            }
            _ => false,
        };
        last_time = t;
        if same {
            out.last_mut().expect("checked above").1.push(r);
        } else {
            out.push((r.action.clone(), vec![r]));
        }
    }
    for (_, items) in &mut out {
        items.reverse();
    }
    out
}

/// 每个路径（小写）最近一次成功放回原处的时间。
fn restored_at(records: &[Record]) -> HashMap<String, u64> {
    let mut out: HashMap<String, u64> = HashMap::new();
    for r in records {
        if r.action == "restore"
            && r.result == "restored"
            && let Some(t) = log::parse_time(&r.time)
        {
            let e = out.entry(r.path.to_lowercase()).or_default();
            *e = (*e).max(t);
        }
    }
    out
}

/// 一条记录现在的状态：
/// - `inBin`：删除到回收站，现在还在回收站里，可以放回
/// - `restored`：删除到回收站，之后已经放回原处（看日志里后来的放回记录）
/// - `occupied`：删除到回收站，回收站里没有了，原位置又有了同名项目
/// - `gone`：删除到回收站，回收站里和原位置都没有了（回收站已清空）
/// - `deleted`：彻底删除
/// - `notDone`：被拦截或失败，没有删除
/// - `linked`、`moved`：硬链接替换、移动；放回原处这条记录本身也是 `restored`
fn status(r: &Record, bins: &mut Bins, restored: &HashMap<String, u64>) -> &'static str {
    match (r.action.as_str(), r.result.as_str()) {
        (_, "blocked" | "failed") => "notDone",
        ("recycle", "deleted") => {
            let found = log::parse_time(&r.time).is_some_and(|t| bins.find(&r.path, t).is_some());
            let later_restore = log::parse_time(&r.time)
                .zip(restored.get(&r.path.to_lowercase()))
                .is_some_and(|(t, &back)| back >= t);
            if found {
                "inBin"
            } else if later_restore {
                "restored"
            } else if Path::new(&r.path).symlink_metadata().is_ok() {
                "occupied"
            } else {
                "gone"
            }
        }
        ("hardlink", _) => "linked",
        ("move", _) => "moved",
        ("restore", _) => "restored",
        _ => "deleted",
    }
}

/// 各磁盘回收站的内容，每个磁盘第一次用到时才读取。
#[derive(Default)]
struct Bins {
    by_volume: HashMap<String, Vec<BinItem>>,
}

impl Bins {
    fn find(&mut self, path: &str, time: u64) -> Option<&BinItem> {
        let volume = volume_root(path)?;
        let items = self
            .by_volume
            .entry(volume.clone())
            .or_insert_with(|| recycle::list(&PathBuf::from(&volume)));
        recycle::find(items, path, time)
    }
}

/// `D:\foo` → `D:\`。
fn volume_root(path: &str) -> Option<String> {
    let b = path.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        .then(|| format!("{}:\\", char::from(b[0]).to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(time: &str, action: &str, path: &str) -> Record {
        Record {
            time: time.into(),
            action: action.into(),
            path: path.into(),
            size: 1,
            result: "deleted".into(),
            reason: None,
            link_to: None,
        }
    }

    #[test]
    fn groups_nearby_records_with_the_same_action() {
        // 从新到旧。
        let groups = group(vec![
            rec("2026-11-05T10:00:30Z", "restore", "d"),
            rec("2026-11-05T10:00:08Z", "recycle", "c"),
            rec("2026-11-05T10:00:02Z", "recycle", "b"),
            rec("2026-11-05T09:00:00Z", "recycle", "a"),
        ]);
        let shape: Vec<(String, Vec<String>)> = groups
            .into_iter()
            .map(|(a, rs)| (a, rs.into_iter().map(|r| r.path).collect()))
            .collect();
        assert_eq!(
            shape,
            [
                ("restore".to_string(), vec!["d".to_string()]),
                (
                    "recycle".to_string(),
                    vec!["b".to_string(), "c".to_string()]
                ),
                ("recycle".to_string(), vec!["a".to_string()]),
            ]
        );
    }
}
