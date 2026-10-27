//! 清理建议：按内置规则在当前结果里找出可以清理的位置，确认后清理。
//!
//! 规则匹配的位置可能在受保护的文件夹里（例如 `C:\Windows\Temp`），清理时只放行这些位置。

use std::sync::OnceLock;

use dl_clean::Target;
use dl_clean::rules::{self, Action, Matched, Risk, Rule};
use dl_tree::{FileTree, NodeId};
use serde::Serialize;
use tauri::State;

use crate::cleanup::{DeletePlan, DeleteResult, execute, make_plan};
use crate::scan::AppState;

/// 每条建议最多列出的位置数。
const MAX_LOCATIONS: usize = 30;

fn rule_set() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(rules::builtin)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    id: NodeId,
    path: String,
    size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    id: String,
    name: String,
    category: String,
    /// `low`、`medium`、`high`。
    risk: &'static str,
    /// 默认勾选。
    default: bool,
    description: String,
    effect: String,
    /// `contents`、`self`、`guide`。
    action: &'static str,
    guide: Option<String>,
    size: u64,
    /// 匹配到的位置数（`locations` 最多列出一部分）。
    count: usize,
    locations: Vec<Location>,
}

/// 当前结果中所有可以清理（或需要指引）的项目，按大小从大到小。
#[tauri::command]
pub fn clean_suggestions(state: State<'_, AppState>) -> Result<Vec<Suggestion>, String> {
    let scan = state.current()?;
    let t = scan.read();
    let rules = rule_set();
    let mut out: Vec<Suggestion> = rules::match_rules(rules, &t, &env)
        .into_iter()
        .map(|m| {
            let r = &rules[m.rule];
            let mut nodes = m.nodes.clone();
            nodes.sort_by_key(|&n| std::cmp::Reverse(t.size(n)));
            Suggestion {
                id: r.id.clone(),
                name: r.name.clone(),
                category: r.category.clone(),
                risk: match r.risk {
                    Risk::Low => "low",
                    Risk::Medium => "medium",
                    Risk::High => "high",
                },
                default: r.default,
                description: r.description.clone(),
                effect: r.effect.clone(),
                action: match r.action {
                    Action::Contents => "contents",
                    Action::Remove => "self",
                    Action::Guide => "guide",
                },
                guide: r.guide.clone(),
                size: m.size,
                count: m.nodes.len(),
                locations: nodes
                    .into_iter()
                    .take(MAX_LOCATIONS)
                    .map(|n| Location {
                        id: n,
                        path: t.path(n),
                        size: t.size(n),
                    })
                    .collect(),
            }
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.size));
    Ok(out)
}

/// 选中的规则要删除的项，以及它们放行的位置。只给指引的规则不删除任何东西。
fn selected(t: &FileTree, rule_ids: &[String]) -> (Vec<NodeId>, Vec<String>) {
    let rules = rule_set();
    let matches: Vec<Matched> = rules::match_rules(rules, t, &env);
    let mut ids = Vec::new();
    let mut allowed = Vec::new();
    for m in matches {
        let r = &rules[m.rule];
        if r.action == Action::Guide || !rule_ids.contains(&r.id) {
            continue;
        }
        allowed.extend(m.nodes.iter().map(|&n| t.path(n)));
        ids.extend(rules::targets(r, t, &m.nodes));
    }
    ids.sort_unstable();
    ids.dedup();
    (ids, allowed)
}

/// 按规则清理前的汇总，格式同删除。
#[tauri::command]
pub fn plan_clean(rule_ids: Vec<String>, state: State<'_, AppState>) -> Result<DeletePlan, String> {
    let scan = state.current()?;
    let t = scan.read();
    let (ids, allowed) = selected(&t, &rule_ids);
    Ok(make_plan(&t, &ids, &allowed))
}

/// 按规则清理。`permanent` 为 false 时移到回收站。
#[tauri::command]
pub async fn clean_rules(
    rule_ids: Vec<String>,
    permanent: bool,
    state: State<'_, AppState>,
) -> Result<DeleteResult, String> {
    let scan = state.current()?;
    let (ids, targets, allowed) = {
        let t = scan.read();
        let (ids, allowed) = selected(&t, &rule_ids);
        let targets = ids
            .iter()
            .map(|&id| Target {
                path: t.path(id).into(),
                size: t.size(id),
            })
            .collect();
        (ids, targets, allowed)
    };
    execute(scan, ids, targets, permanent, allowed).await
}
