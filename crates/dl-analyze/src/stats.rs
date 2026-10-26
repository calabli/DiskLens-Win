//! 子树统计与搜索。

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use dl_tree::{FileTree, NodeId, NodeKind};

use crate::category::{Category, classify, is_dev_dir};
use crate::query::Query;

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
    let mut stack: Vec<NodeId> = tree.children(id).collect();
    while let Some(n) = stack.pop() {
        let is_dir = tree.kind(n) == NodeKind::Dir;
        if is_dir {
            stack.extend(tree.children(n));
        }
        let size = tree.size(n);
        if heap.len() == limit && heap.peek().is_some_and(|Reverse((s, _))| *s >= size) {
            continue;
        }
        if query.matches(tree.name(n), is_dir, size) {
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
