//! 针对当前扫描结果的查询命令。整棵树留在 Rust 侧，前端只取要显示的部分。

use dl_analyze::category::{classify, is_dev_dir};
use dl_analyze::{Category, ChildBreakdown, Query};
use dl_tree::{FileTree, NodeId, NodeKind, Rect};
use serde::Serialize;
use tauri::State;
use tauri::ipc::Response;

use crate::scan::AppState;

/// 列表一次最多返回的子项数。
const MAX_LIST: usize = 5_000;
/// 树图最多画多少块。
const MAX_RECTS: usize = 3_000;
/// 树图里面积小于这个值（平方像素）的块不画。
const MIN_RECT_AREA: f32 = 6.0;
/// 树图二进制格式中每个矩形的字节数：id u32、类型 u32、x/y/w/h 各 f32。
const RECT_BYTES: usize = 24;
/// 旭日图画几圈。
const SUNBURST_DEPTH: u32 = 5;
/// 旭日图最多画多少段弧。
const MAX_SEGMENTS: usize = 4_000;
/// 张角小于圆周这个比例（约 0.7°）的弧不画。
const MIN_SWEEP: f32 = 0.002;
/// 旭日图二进制格式中每段弧的字节数：id u32、类型 u32、圈 u32、起止角度各 f32。
const SEGMENT_BYTES: usize = 20;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crumb {
    id: NodeId,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    id: NodeId,
    name: String,
    path: String,
    size: u64,
    allocated: u64,
    files: u32,
    is_dir: bool,
    parent: Option<NodeId>,
    /// 从根到该节点（含）的路径。
    crumbs: Vec<Crumb>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    id: NodeId,
    name: String,
    size: u64,
    allocated: u64,
    files: u32,
    is_dir: bool,
    has_children: bool,
    category: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildList {
    /// 满足筛选条件的子项总数（可能多于返回的条数）。
    total: usize,
    entries: Vec<Entry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeStat {
    key: &'static str,
    label: &'static str,
    size: u64,
    allocated: u64,
    files: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    id: NodeId,
    name: String,
    path: String,
    parent: Option<NodeId>,
    size: u64,
    is_dir: bool,
    category: &'static str,
}

/// 节点存在且仍挂在树上（没有被删除摘掉）。
pub(crate) fn check(tree: &FileTree, id: NodeId) -> Result<(), String> {
    if tree.is_attached(id) {
        Ok(())
    } else {
        Err("这个项目已不存在，可能已被删除或扫描结果已更新".into())
    }
}

/// 子项的类型：文件按名称归类，目录取其子树中占用最多的类型。
fn category_of(tree: &FileTree, breakdown: &ChildBreakdown, in_dev: bool, id: NodeId) -> Category {
    match tree.kind(id) {
        NodeKind::File => classify(tree.name(id), in_dev),
        NodeKind::Dir => breakdown
            .children
            .iter()
            .find(|(c, _)| *c == id)
            .map_or(Category::Other, |(_, t)| t.dominant()),
    }
}

fn in_dev_dir(tree: &FileTree, id: NodeId) -> bool {
    let mut cur = Some(id);
    while let Some(n) = cur {
        if tree.parent(n).is_some() && is_dev_dir(tree.name(n)) {
            return true;
        }
        cur = tree.parent(n);
    }
    false
}

#[tauri::command]
pub fn node_info(id: NodeId, state: State<'_, AppState>) -> Result<NodeInfo, String> {
    let scan = state.current()?;
    let guard = scan.read();
    let t = &*guard;
    check(t, id)?;
    let mut chain = vec![id];
    while let Some(p) = t.parent(*chain.last().expect("non-empty")) {
        chain.push(p);
    }
    chain.reverse();
    Ok(NodeInfo {
        id,
        name: t.name(id).to_owned(),
        path: t.path(id),
        size: t.size(id),
        allocated: t.allocated(id),
        files: t.file_count(id),
        is_dir: t.kind(id) == NodeKind::Dir,
        parent: t.parent(id),
        crumbs: chain
            .into_iter()
            .map(|n| Crumb {
                id: n,
                name: t.name(n).to_owned(),
            })
            .collect(),
    })
}

/// 目录的直接子项，按大小从大到小；`filter` 非空时按搜索语法筛选。
#[tauri::command]
pub fn list_children(
    id: NodeId,
    filter: Option<String>,
    state: State<'_, AppState>,
) -> Result<ChildList, String> {
    let scan = state.current()?;
    let guard = scan.read();
    let t = &*guard;
    check(t, id)?;
    let query = Query::parse(filter.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
    let breakdown = scan.breakdown(t, id);
    let in_dev = in_dev_dir(t, id);

    let matched: Vec<NodeId> = t
        .children_by_size(id)
        .into_iter()
        .filter(|&c| query.matches(t.name(c), t.kind(c) == NodeKind::Dir, t.size(c)))
        .collect();
    let entries = matched
        .iter()
        .take(MAX_LIST)
        .map(|&c| Entry {
            id: c,
            name: t.name(c).to_owned(),
            size: t.size(c),
            allocated: t.allocated(c),
            files: t.file_count(c),
            is_dir: t.kind(c) == NodeKind::Dir,
            has_children: t.children(c).next().is_some(),
            category: category_of(t, &breakdown, in_dev, c).key(),
        })
        .collect();
    Ok(ChildList {
        total: matched.len(),
        entries,
    })
}

/// 目录在 `width × height` 区域内的树图，二进制格式，每块 [`RECT_BYTES`] 字节（小端）：
/// `id: u32, category: u32, x: f32, y: f32, w: f32, h: f32`。category 为 [`Category::ALL`] 中的下标。
#[tauri::command]
pub fn treemap(
    id: NodeId,
    width: f32,
    height: f32,
    state: State<'_, AppState>,
) -> Result<Response, String> {
    let scan = state.current()?;
    let guard = scan.read();
    let t = &*guard;
    check(t, id)?;
    let breakdown = scan.breakdown(t, id);
    let in_dev = in_dev_dir(t, id);
    let area = Rect {
        x: 0.0,
        y: 0.0,
        w: width.max(0.0),
        h: height.max(0.0),
    };
    let rects = t.treemap(id, area, MAX_RECTS, MIN_RECT_AREA);
    let mut buf = Vec::with_capacity(rects.len() * RECT_BYTES);
    for (n, r) in rects {
        let cat = category_of(t, &breakdown, in_dev, n).index() as u32;
        buf.extend_from_slice(&n.to_le_bytes());
        buf.extend_from_slice(&cat.to_le_bytes());
        for v in [r.x, r.y, r.w, r.h] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    Ok(Response::new(buf))
}

/// 以目录为中心的旭日图，二进制格式，每段 [`SEGMENT_BYTES`] 字节（小端）：
/// `id: u32, flags: u32, depth: u32, start: f32, end: f32`。
/// flags 低 8 位是 [`Category::ALL`] 中的下标，第 8 位表示目录；
/// depth 从 1 开始；角度是圆周的比例（0..1，从 12 点方向顺时针）。
#[tauri::command]
pub fn sunburst(id: NodeId, state: State<'_, AppState>) -> Result<Response, String> {
    let scan = state.current()?;
    let guard = scan.read();
    let t = &*guard;
    check(t, id)?;
    let segments = t.sunburst(id, SUNBURST_DEPTH, MIN_SWEEP, MAX_SEGMENTS);
    let dominants = scan.dominants(t, id, SUNBURST_DEPTH);
    let mut buf = Vec::with_capacity(segments.len() * SEGMENT_BYTES);
    for s in segments {
        let is_dir = t.kind(s.id) == NodeKind::Dir;
        let cat = match t.kind(s.id) {
            NodeKind::Dir => dominants.get(&s.id).copied().unwrap_or(Category::Other),
            NodeKind::File => {
                let in_dev = t.parent(s.id).is_some_and(|p| in_dev_dir(t, p));
                classify(t.name(s.id), in_dev)
            }
        };
        buf.extend_from_slice(&s.id.to_le_bytes());
        let flags = cat.index() as u32 | u32::from(is_dir) << 8;
        buf.extend_from_slice(&flags.to_le_bytes());
        buf.extend_from_slice(&s.depth.to_le_bytes());
        buf.extend_from_slice(&s.start.to_le_bytes());
        buf.extend_from_slice(&s.end.to_le_bytes());
    }
    Ok(Response::new(buf))
}

/// 目录整棵子树的类型分布，从大到小，不含空类型。
#[tauri::command]
pub fn type_stats(id: NodeId, state: State<'_, AppState>) -> Result<Vec<TypeStat>, String> {
    let scan = state.current()?;
    let guard = scan.read();
    check(&guard, id)?;
    let total = scan.breakdown(&guard, id).total;
    let mut stats: Vec<TypeStat> = Category::ALL
        .into_iter()
        .filter(|c| total.files[c.index()] > 0)
        .map(|c| TypeStat {
            key: c.key(),
            label: c.label(),
            size: total.size[c.index()],
            allocated: total.allocated[c.index()],
            files: total.files[c.index()],
        })
        .collect();
    stats.sort_by_key(|s| std::cmp::Reverse(s.size));
    Ok(stats)
}

/// 在目录的整棵子树里搜索，返回最大的 `limit` 项。
#[tauri::command]
pub fn search(
    id: NodeId,
    query: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<SearchHit>, String> {
    let scan = state.current()?;
    let guard = scan.read();
    let t = &*guard;
    check(t, id)?;
    let query = Query::parse(&query).map_err(|e| e.to_string())?;
    if query.is_empty() {
        return Ok(Vec::new());
    }
    Ok(dl_analyze::search(t, id, &query, limit.min(MAX_LIST))
        .into_iter()
        .map(|n| {
            let is_dir = t.kind(n) == NodeKind::Dir;
            let category = if is_dir {
                Category::Other
            } else {
                classify(t.name(n), in_dev_dir(t, n))
            };
            SearchHit {
                id: n,
                name: t.name(n).to_owned(),
                path: t.path(n),
                parent: t.parent(n),
                size: t.size(n),
                is_dir,
                category: category.key(),
            }
        })
        .collect())
}
