//! 针对当前扫描结果的查询命令。整棵树留在 Rust 侧，前端只取要显示的部分。

use dl_analyze::category::{classify, is_dev_dir};
use dl_analyze::{Category, ChildBreakdown, Item, Query};
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
/// 旭日图最多画几圈。
const MAX_SUNBURST_DEPTH: u32 = 8;
/// 嵌套树图最多几层。
const MAX_TREEMAP_DEPTH: u32 = 4;
/// 嵌套树图一共最多画多少块。
const MAX_NESTED_RECTS: usize = 8_000;
/// 嵌套树图里目录块顶部留给名称的高度，以及四周的边距（像素）。
const NEST_HEADER: f32 = 18.0;
const NEST_PAD: f32 = 3.0;
/// 小于这个宽或高的目录块不再往里画。
const NEST_MIN: f32 = 48.0;
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
    /// 修改时间、访问时间（Unix 秒，0 表示未知）。
    modified: u32,
    accessed: u32,
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
    modified: u32,
    accessed: u32,
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

/// 目录的直接子项；`filter` 非空时按搜索语法筛选。
/// `sort` 为 `name`、`size`（默认）、`allocated`、`files`、`modified`、`accessed`，
/// `desc` 默认按名称升序、其他降序。
#[tauri::command]
pub fn list_children(
    id: NodeId,
    filter: Option<String>,
    sort: Option<String>,
    desc: Option<bool>,
    state: State<'_, AppState>,
) -> Result<ChildList, String> {
    let scan = state.current()?;
    let guard = scan.read();
    let t = &*guard;
    check(t, id)?;
    let query = Query::parse(filter.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
    let breakdown = scan.breakdown(t, id);
    let in_dev = in_dev_dir(t, id);

    let mut matched: Vec<NodeId> = t
        .children_by_size(id)
        .into_iter()
        .filter(|&c| {
            query.matches(&Item {
                name: t.name(c),
                is_dir: t.kind(c) == NodeKind::Dir,
                size: t.size(c),
                modified: t.modified(c),
                accessed: t.accessed(c),
                // 当前层的子项：目录按其中占用最多的类别，与着色一致。
                category: Some(category_of(t, &breakdown, in_dev, c)),
            })
        })
        .collect();
    sort_children(t, &mut matched, sort.as_deref().unwrap_or("size"), desc);
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
            modified: t.modified(c),
            accessed: t.accessed(c),
        })
        .collect();
    Ok(ChildList {
        total: matched.len(),
        entries,
    })
}

/// 按列排序。`ids` 已按大小从大到小排好，排序是稳定的，所以同值时仍按大小排列。
fn sort_children(t: &FileTree, ids: &mut [NodeId], key: &str, desc: Option<bool>) {
    let desc = desc.unwrap_or(key != "name");
    match key {
        "name" => ids.sort_by_cached_key(|&c| t.name(c).to_lowercase()),
        "allocated" => ids.sort_by_key(|&c| t.allocated(c)),
        "files" => ids.sort_by_key(|&c| t.file_count(c)),
        "modified" => ids.sort_by_key(|&c| t.modified(c)),
        "accessed" => ids.sort_by_key(|&c| t.accessed(c)),
        _ => {
            // 已经是按大小降序。
            if !desc {
                ids.reverse();
            }
            return;
        }
    }
    if desc {
        ids.reverse();
    }
}

/// 目录在 `width × height` 区域内的树图，二进制格式，每块 [`RECT_BYTES`] 字节（小端）：
/// `id: u32, flags: u32, x: f32, y: f32, w: f32, h: f32`。
/// flags 低 8 位是 [`Category::ALL`] 中的下标，8–15 位是层级（直接子项为 1），第 16 位表示目录。
/// `depth` 大于 1 时，足够大的目录块里继续画它的子项，顶部留出名称栏；父块排在子块前面。
#[tauri::command]
pub fn treemap(
    id: NodeId,
    width: f32,
    height: f32,
    depth: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Response, String> {
    let scan = state.current()?;
    let guard = scan.read();
    let t = &*guard;
    check(t, id)?;
    let depth = depth.unwrap_or(1).clamp(1, MAX_TREEMAP_DEPTH);
    let breakdown = scan.breakdown(t, id);
    let dominants = (depth > 1).then(|| scan.dominants(t, id, MAX_SUNBURST_DEPTH));
    let in_dev = in_dev_dir(t, id);
    let area = Rect {
        x: 0.0,
        y: 0.0,
        w: width.max(0.0),
        h: height.max(0.0),
    };
    let mut rects = Vec::new();
    nested_treemap(t, id, area, 1, depth, &mut rects);
    let mut buf = Vec::with_capacity(rects.len() * RECT_BYTES);
    for (n, r, level) in rects {
        let cat = if level == 1 {
            category_of(t, &breakdown, in_dev, n)
        } else if t.kind(n) == NodeKind::Dir {
            dominants
                .as_ref()
                .and_then(|d| d.get(&n).copied())
                .unwrap_or(Category::Other)
        } else {
            let in_dev = t.parent(n).is_some_and(|p| in_dev_dir(t, p));
            classify(t.name(n), in_dev)
        };
        let is_dir = u32::from(t.kind(n) == NodeKind::Dir);
        let flags = cat.index() as u32 | (level << 8) | (is_dir << 16);
        buf.extend_from_slice(&n.to_le_bytes());
        buf.extend_from_slice(&flags.to_le_bytes());
        for v in [r.x, r.y, r.w, r.h] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    Ok(Response::new(buf))
}

/// 递归布局：先放 `id` 的子项，再在足够大的目录块里（除去名称栏和边距）放它的子项。
fn nested_treemap(
    t: &FileTree,
    id: NodeId,
    area: Rect,
    level: u32,
    depth: u32,
    out: &mut Vec<(NodeId, Rect, u32)>,
) {
    let max = if level == 1 { MAX_RECTS } else { MAX_RECTS / 4 };
    for (n, r) in t.treemap(id, area, max, MIN_RECT_AREA) {
        if out.len() >= MAX_NESTED_RECTS {
            return;
        }
        out.push((n, r, level));
        if level < depth && t.kind(n) == NodeKind::Dir && r.w >= NEST_MIN && r.h >= NEST_MIN {
            let inner = Rect {
                x: r.x + NEST_PAD,
                y: r.y + NEST_HEADER,
                w: r.w - 2.0 * NEST_PAD,
                h: r.h - NEST_HEADER - NEST_PAD,
            };
            nested_treemap(t, n, inner, level + 1, depth, out);
        }
    }
}

/// 以目录为中心的旭日图，二进制格式，每段 [`SEGMENT_BYTES`] 字节（小端）：
/// `id: u32, flags: u32, depth: u32, start: f32, end: f32`。
/// flags 低 8 位是 [`Category::ALL`] 中的下标，第 8 位表示目录；
/// depth 从 1 开始；角度是圆周的比例（0..1，从 12 点方向顺时针）。
#[tauri::command]
pub fn sunburst(
    id: NodeId,
    depth: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Response, String> {
    let scan = state.current()?;
    let guard = scan.read();
    let t = &*guard;
    check(t, id)?;
    let depth = depth.unwrap_or(5).clamp(2, MAX_SUNBURST_DEPTH);
    let segments = t.sunburst(id, depth, MIN_SWEEP, MAX_SEGMENTS);
    // 缓存按最大圈数计算，换圈数时不必重算。
    let dominants = scan.dominants(t, id, MAX_SUNBURST_DEPTH);
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketStat {
    label: &'static str,
    /// 搜索语法，点按后列出这一组的文件。时间未知的一组没有。
    query: Option<&'static str>,
    size: u64,
    files: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketStats {
    age: Vec<BucketStat>,
    size: Vec<BucketStat>,
}

/// 目录整棵子树中的文件按修改时间、按大小的分布。
#[tauri::command]
pub fn bucket_stats(id: NodeId, state: State<'_, AppState>) -> Result<BucketStats, String> {
    const AGE: [(&str, Option<&str>); 5] = [
        ("一周内修改", Some("file: dm:>1w")),
        ("一个月内", Some("file: dm:<=1w dm:>30d")),
        ("一年内", Some("file: dm:<=30d dm:>1y")),
        ("一年以前", Some("file: dm:<=1y")),
        ("时间未知", None),
    ];
    const SIZE: [(&str, Option<&str>); 4] = [
        ("1 GB 以上", Some("file: size:>=1gb")),
        ("100 MB – 1 GB", Some("file: size:>=100mb size:<1gb")),
        ("1 – 100 MB", Some("file: size:>=1mb size:<100mb")),
        ("1 MB 以下", Some("file: size:<1mb")),
    ];
    let scan = state.current()?;
    let guard = scan.read();
    check(&guard, id)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let b = dl_analyze::buckets(&guard, id, now);
    let rows = |defs: &[(&'static str, Option<&'static str>)], vals: &[dl_analyze::Bucket]| {
        defs.iter()
            .zip(vals)
            .filter(|(_, v)| v.files > 0)
            .map(|(&(label, query), v)| BucketStat {
                label,
                query,
                size: v.size,
                files: v.files,
            })
            .collect()
    };
    Ok(BucketStats {
        age: rows(&AGE, &b.age),
        size: rows(&SIZE, &b.size),
    })
}

/// 在目录的整棵子树里搜索，返回最大的 `limit` 项。
/// 按完整路径找到扫描结果里的节点；不在扫描范围内或已不存在时返回 `None`。
#[tauri::command]
pub fn locate(path: String, state: State<'_, AppState>) -> Result<Option<NodeId>, String> {
    let scan = state.current()?;
    let t = scan.read();
    let root = t.root();
    let base = t.path(root);
    let rest = if path.len() >= base.len() && path[..base.len()].eq_ignore_ascii_case(&base) {
        &path[base.len()..]
    } else {
        return Ok(None);
    };
    let mut id = root;
    for part in rest.split('\\').filter(|p| !p.is_empty()) {
        match t.child_by_name(id, part) {
            Some(c) => id = c,
            None => return Ok(None),
        }
    }
    Ok(Some(id))
}

/// 图表上标出搜索结果：`ids` 是画出来的方块，返回每一项的标记，见 [`dl_analyze::search_marks`]。
#[tauri::command]
pub fn search_marks(
    id: NodeId,
    query: String,
    ids: Vec<NodeId>,
    state: State<'_, AppState>,
) -> Result<Vec<u8>, String> {
    let scan = state.current()?;
    let guard = scan.read();
    let t = &*guard;
    check(t, id)?;
    let query = Query::parse(&query).map_err(|e| e.to_string())?;
    if query.is_empty() {
        return Ok(vec![0; ids.len()]);
    }
    Ok(dl_analyze::search_marks(t, id, &query, &ids))
}

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
                modified: t.modified(n),
                accessed: t.accessed(n),
            }
        })
        .collect())
}
