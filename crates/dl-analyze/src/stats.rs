//! 子树统计与搜索。

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use dl_tree::{FileTree, NodeId, NodeKind};

use crate::category::{Category, classify, is_dev_dir};
use crate::query::{Item, Query};

const N: usize = Category::ALL.len();

/// 按类型汇总的大小、占用空间和文件数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CategoryTotals {
    pub size: [u64; N],
    pub allocated: [u64; N],
    pub files: [u64; N],
}

impl CategoryTotals {
    fn add_file(&mut self, c: Category, size: u64, allocated: u64) {
        let i = c.index();
        self.size[i] += size;
        self.allocated[i] += allocated;
        self.files[i] += 1;
    }

    fn merge(&mut self, other: &CategoryTotals) {
        for i in 0..N {
            self.size[i] += other.size[i];
            self.allocated[i] += other.allocated[i];
            self.files[i] += other.files[i];
        }
    }

    /// 占用最多的类型；全空时为 [`Category::Other`]。
    pub fn dominant(&self) -> Category {
        Category::ALL
            .into_iter()
            .max_by_key(|c| (self.size[c.index()], Reverse(c.index())))
            .filter(|c| self.size[c.index()] > 0)
            .unwrap_or(Category::Other)
    }
}

/// 一个目录的类型统计，以及每个直接子项各自的统计。
#[derive(Debug, Clone, Default)]
pub struct ChildBreakdown {
    pub total: CategoryTotals,
    pub children: Vec<(NodeId, CategoryTotals)>,
}

/// 统计 `id` 整棵子树的类型分布，并分别给出每个直接子项的分布（用于给子项着色）。
pub fn breakdown(tree: &FileTree, id: NodeId) -> ChildBreakdown {
    let in_dev = ancestors_in_dev_dir(tree, id);
    let mut out = ChildBreakdown::default();
    for child in tree.children(id) {
        let totals = subtree(tree, child, in_dev);
        out.total.merge(&totals);
        out.children.push((child, totals));
    }
    out
}

/// `id` 往下 `max_depth` 层以内每个目录的主要类型（给旭日图着色用）。整棵子树只走一遍：
/// 第 `max_depth` 层的目录直接统计整棵子树，更浅的目录由子项的统计相加得到。
pub fn dominant_dirs(tree: &FileTree, id: NodeId, max_depth: u32) -> HashMap<NodeId, Category> {
    let mut out = HashMap::new();
    if max_depth == 0 {
        return out;
    }
    let in_dev = ancestors_in_dev_dir(tree, id);
    for child in tree.children(id) {
        if tree.kind(child) == NodeKind::Dir {
            collect_dominant(tree, child, 1, max_depth, in_dev, &mut out);
        }
    }
    out
}

fn collect_dominant(
    tree: &FileTree,
    dir: NodeId,
    depth: u32,
    max_depth: u32,
    parent_in_dev: bool,
    out: &mut HashMap<NodeId, Category>,
) -> CategoryTotals {
    let totals = if depth >= max_depth {
        subtree(tree, dir, parent_in_dev)
    } else {
        let in_dev = parent_in_dev || is_dev_dir(tree.name(dir));
        let mut totals = CategoryTotals::default();
        for c in tree.children(dir) {
            match tree.kind(c) {
                NodeKind::File => {
                    totals.add_file(
                        classify(tree.name(c), in_dev),
                        tree.size(c),
                        tree.allocated(c),
                    );
                }
                NodeKind::Dir => {
                    totals.merge(&collect_dominant(
                        tree,
                        c,
                        depth + 1,
                        max_depth,
                        in_dev,
                        out,
                    ));
                }
            }
        }
        totals
    };
    out.insert(dir, totals.dominant());
    totals
}

/// 一组文件的合计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bucket {
    pub size: u64,
    pub files: u64,
}

/// 按修改时间和按大小分组的统计（只算文件）。分界与搜索语法一致，见 [`AGE_LIMITS`]、[`SIZE_LIMITS`]。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Buckets {
    /// 一周内、一个月内、一年内、更早、时间未知。
    pub age: [Bucket; 5],
    /// 1 GB 以上、100 MB–1 GB、1–100 MB、1 MB 以下。
    pub size: [Bucket; 4],
}

/// 按修改时间分组的分界：7 天、30 天、365 天（与 `dm:>7d`、`dm:>1m`、`dm:>1y` 一致）。
pub const AGE_LIMITS: [u64; 3] = [7 * 86_400, 30 * 86_400, 365 * 86_400];
/// 按大小分组的分界（从大到小）。
pub const SIZE_LIMITS: [u64; 3] = [1 << 30, 100 << 20, 1 << 20];

/// 统计 `id` 子树中的文件按修改时间和大小的分布。`now` 为当前时间（Unix 秒）。
pub fn buckets(tree: &FileTree, id: NodeId, now: u64) -> Buckets {
    let mut out = Buckets::default();
    let mut stack = vec![id];
    while let Some(n) = stack.pop() {
        if tree.kind(n) == NodeKind::Dir {
            stack.extend(tree.children(n));
            continue;
        }
        let size = tree.size(n);
        let modified = u64::from(tree.modified(n));
        let age = if modified == 0 {
            4
        } else {
            let since = now.saturating_sub(modified);
            AGE_LIMITS.iter().position(|&l| since < l).unwrap_or(3)
        };
        let by_size = SIZE_LIMITS.iter().position(|&l| size >= l).unwrap_or(3);
        for b in [&mut out.age[age], &mut out.size[by_size]] {
            b.size += size;
            b.files += 1;
        }
    }
    out
}

/// `id` 自己或它的某个祖先是否是开发缓存目录。根目录名（盘符或路径）不算。
fn ancestors_in_dev_dir(tree: &FileTree, id: NodeId) -> bool {
    let mut cur = Some(id);
    while let Some(n) = cur {
        if tree.parent(n).is_some() && is_dev_dir(tree.name(n)) {
            return true;
        }
        cur = tree.parent(n);
    }
    false
}

fn subtree(tree: &FileTree, id: NodeId, parent_in_dev: bool) -> CategoryTotals {
    let mut totals = CategoryTotals::default();
    let mut stack = vec![(id, parent_in_dev)];
    while let Some((n, in_dev)) = stack.pop() {
        match tree.kind(n) {
            NodeKind::File => {
                totals.add_file(
                    classify(tree.name(n), in_dev),
                    tree.size(n),
                    tree.allocated(n),
                );
            }
            NodeKind::Dir => {
                let dev = in_dev || is_dev_dir(tree.name(n));
                stack.extend(tree.children(n).map(|c| (c, dev)));
            }
        }
    }
    totals
}

/// 在 `id` 的子树（不含 `id` 本身）中找满足 `query` 的项，返回最大的至多 `limit` 个，从大到小。
pub fn search(tree: &FileTree, id: NodeId, query: &Query, limit: usize) -> Vec<NodeId> {
    if limit == 0 {
        return Vec::new();
    }
    // 小顶堆保留当前最大的 limit 个。
    let mut heap: BinaryHeap<Reverse<(u64, NodeId)>> = BinaryHeap::with_capacity(limit + 1);
    let root_dev = ancestors_in_dev_dir(tree, id);
    let mut stack: Vec<(NodeId, bool)> = tree.children(id).map(|c| (c, root_dev)).collect();
    while let Some((n, in_dev)) = stack.pop() {
        let is_dir = tree.kind(n) == NodeKind::Dir;
        if is_dir {
            let dev = in_dev || is_dev_dir(tree.name(n));
            stack.extend(tree.children(n).map(|c| (c, dev)));
        }
        let size = tree.size(n);
        if heap.len() == limit && heap.peek().is_some_and(|Reverse((s, _))| *s >= size) {
            continue;
        }
        let item = Item {
            name: tree.name(n),
            is_dir,
            size,
            modified: tree.modified(n),
            accessed: tree.accessed(n),
            // 文件按名称归类；目录的类别要统计整棵子树，搜索时不算。
            category: (!is_dir).then(|| classify(tree.name(n), in_dev)),
        };
        if query.matches(&item) {
            heap.push(Reverse((size, n)));
            if heap.len() > limit {
                heap.pop();
            }
        }
    }
    let mut found: Vec<(u64, NodeId)> = heap.into_iter().map(|Reverse(x)| x).collect();
    found.sort_by_key(|&(s, n)| (Reverse(s), n));
    found.into_iter().map(|(_, n)| n).collect()
}

/// 在 `id` 的子树中搜索 `query`，再看 `ids` 里的每一项和搜索结果的关系：
/// 2 表示它本身满足条件，1 表示它下面有满足条件的项，0 表示都没有。
/// 图表用它把命中的方块标出来。
pub fn search_marks(tree: &FileTree, id: NodeId, query: &Query, ids: &[NodeId]) -> Vec<u8> {
    let wanted: HashSet<NodeId> = ids.iter().copied().collect();
    let mut hit = HashSet::new();
    let mut inside = HashSet::new();
    let root_dev = ancestors_in_dev_dir(tree, id);
    let mut stack: Vec<(NodeId, bool)> = tree.children(id).map(|c| (c, root_dev)).collect();
    while let Some((n, in_dev)) = stack.pop() {
        let is_dir = tree.kind(n) == NodeKind::Dir;
        if is_dir {
            let dev = in_dev || is_dev_dir(tree.name(n));
            stack.extend(tree.children(n).map(|c| (c, dev)));
        }
        let item = Item {
            name: tree.name(n),
            is_dir,
            size: tree.size(n),
            modified: tree.modified(n),
            accessed: tree.accessed(n),
            category: (!is_dir).then(|| classify(tree.name(n), in_dev)),
        };
        if !query.matches(&item) {
            continue;
        }
        if wanted.contains(&n) {
            hit.insert(n);
        }
        // 往上标记祖先，遇到已经标过的就停。
        let mut p = tree.parent(n);
        while let Some(a) = p {
            if a == id || !inside.insert(a) {
                break;
            }
            p = tree.parent(a);
        }
    }
    ids.iter()
        .map(|n| {
            if hit.contains(n) {
                2
            } else if inside.contains(n) {
                1
            } else {
                0
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FileTree {
        let mut t = FileTree::new(r"D:\");
        let proj = t.add_dir(0, "proj");
        let nm = t.add_dir(proj, "node_modules");
        t.add_file(nm, "big.png", 5_000, 8_192); // 开发缓存目录下，算开发缓存
        t.add_file(proj, "logo.png", 300, 4_096);
        let media = t.add_dir(0, "media");
        t.add_file(media, "a.mkv", 9_000, 12_288);
        t.add_file(media, "b.MP3", 1_000, 4_096);
        t.add_file(0, "Win11.iso", 7_000, 8_192);
        t.finalize();
        t
    }

    #[test]
    fn search_marks_hits_and_their_folders() {
        let t = sample();
        let proj = t.child_by_name(0, "proj").unwrap();
        let nm = t.child_by_name(proj, "node_modules").unwrap();
        let media = t.child_by_name(0, "media").unwrap();
        let iso = t.child_by_name(0, "Win11.iso").unwrap();
        let q = Query::parse("ext:png").unwrap();
        assert_eq!(
            search_marks(&t, 0, &q, &[proj, nm, media, iso]),
            [1, 1, 0, 0]
        );
        let logo = t.child_by_name(proj, "logo.png").unwrap();
        assert_eq!(search_marks(&t, proj, &q, &[nm, logo, proj]), [1, 2, 0]);
        let q = Query::parse("folder: media").unwrap();
        assert_eq!(search_marks(&t, 0, &q, &[media, proj]), [2, 0]);
    }

    #[test]
    fn buckets_by_age_and_size() {
        let now = 2_000_000_000u64;
        let day = 86_400u32;
        let mut t = FileTree::new("r");
        for (name, size, days_ago) in [
            ("new", 10, 1),
            ("month", 2 << 20, 20),
            ("year", 200 << 20, 200),
            ("old", 2 << 30, 900),
        ] {
            let f = t.add_file(0, name, size, size);
            t.set_times(f, now as u32 - days_ago * day, 0);
        }
        let d = t.add_dir(0, "d");
        t.add_file(d, "unknown", 5, 5);
        t.finalize();
        let b = buckets(&t, 0, now);
        assert_eq!(b.age.map(|x| x.files), [1, 1, 1, 1, 1]);
        assert_eq!(b.age[3].size, 2 << 30);
        assert_eq!(b.size.map(|x| x.files), [1, 1, 1, 2]);
        assert_eq!(b.size[3].size, 15);
    }

    #[test]
    fn dominant_dirs_matches_breakdown_at_every_depth() {
        let t = sample();
        let proj = t.child_by_name(0, "proj").unwrap();
        let nm = t.child_by_name(proj, "node_modules").unwrap();
        let media = t.child_by_name(0, "media").unwrap();

        for depth in 1..=3 {
            let d = dominant_dirs(&t, 0, depth);
            assert_eq!(d[&media], Category::Media, "depth {depth}");
            assert_eq!(d[&proj], Category::Dev, "depth {depth}");
            assert_eq!(d.contains_key(&nm), depth >= 2, "depth {depth}");
        }
        assert_eq!(dominant_dirs(&t, 0, 2)[&nm], Category::Dev);
        assert!(dominant_dirs(&t, 0, 0).is_empty());
        // 从开发缓存目录内部开始，同样算开发缓存。
        assert_eq!(dominant_dirs(&t, proj, 1)[&nm], Category::Dev);
    }

    #[test]
    fn breakdown_totals_and_dominant_child_types() {
        let t = sample();
        let b = breakdown(&t, 0);
        assert_eq!(b.total.size[Category::Media.index()], 10_000);
        assert_eq!(b.total.size[Category::Dev.index()], 5_000);
        assert_eq!(b.total.files[Category::Document.index()], 1);
        assert_eq!(b.total.files.iter().sum::<u64>(), 5);

        let dominant = |name: &str| {
            let id = t.child_by_name(0, name).unwrap();
            b.children
                .iter()
                .find(|(c, _)| *c == id)
                .unwrap()
                .1
                .dominant()
        };
        assert_eq!(dominant("proj"), Category::Dev);
        assert_eq!(dominant("media"), Category::Media);
        assert_eq!(dominant("Win11.iso"), Category::Package);
    }

    #[test]
    fn breakdown_inside_dev_dir_counts_everything_as_dev() {
        let t = sample();
        let proj = t.child_by_name(0, "proj").unwrap();
        let nm = t.child_by_name(proj, "node_modules").unwrap();
        let b = breakdown(&t, nm);
        assert_eq!(b.total.size[Category::Dev.index()], 5_000);
        assert_eq!(CategoryTotals::default().dominant(), Category::Other);
    }

    #[test]
    fn search_returns_largest_matches() {
        let t = sample();
        let names = |q: &str, limit| -> Vec<&str> {
            search(&t, 0, &Query::parse(q).unwrap(), limit)
                .into_iter()
                .map(|n| t.name(n))
                .collect()
        };
        assert_eq!(names("ext:png;mkv", 10), ["a.mkv", "big.png", "logo.png"]);
        assert_eq!(names("size:>4000 file:", 2), ["a.mkv", "Win11.iso"]);
        assert_eq!(names("folder:", 10), ["media", "proj", "node_modules"]);
        assert!(names("nothing-matches", 10).is_empty());
        assert!(names("", 0).is_empty());
    }
}
