//! 变化对比：两次扫描之间，哪些文件夹和文件变大了、变小了、新出现或不见了。
//!
//! 两棵树从根开始同时往下走，同一层按名称（不区分大小写）配对。只配上一边的是新增或删除，
//! 整个算一处变化，不再往里看。
//! 为了不把一处变化沿着所有上级文件夹重复列出，一个文件夹只有在它的变化不能主要归到
//! 某一个子项时才列出：如果某个子项的变化方向相同、且占了它变化的八成以上，就只往下看那个子项。

use std::collections::HashMap;

use dl_tree::{FileTree, NodeId, NodeKind};

/// 一处变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// 现在扫描结果里的节点；删除的项没有。
    pub now: Option<NodeId>,
    /// 之前扫描结果里的节点；新增的项没有。
    pub before: Option<NodeId>,
    pub size_before: u64,
    pub size_after: u64,
    pub is_dir: bool,
}

impl Change {
    pub fn delta(&self) -> i128 {
        i128::from(self.size_after) - i128::from(self.size_before)
    }
}

/// 找出变化不小于 `min_delta` 字节的项（根本身不算），顺序不定。
pub fn changes(now: &FileTree, before: &FileTree, min_delta: u64) -> Vec<Change> {
    let mut out = Vec::new();
    let mut walk = Walk {
        now,
        before,
        min: i128::from(min_delta.max(1)),
        out: &mut out,
    };
    // 根目录配对后只看子项。
    walk.children(Some(now.root()), Some(before.root()));
    out
}

struct Walk<'a> {
    now: &'a FileTree,
    before: &'a FileTree,
    min: i128,
    out: &'a mut Vec<Change>,
}

impl Walk<'_> {
    fn size(t: &FileTree, id: Option<NodeId>) -> u64 {
        id.map_or(0, |n| t.size(n))
    }

    fn is_dir(t: &FileTree, id: Option<NodeId>) -> bool {
        id.is_some_and(|n| t.kind(n) == NodeKind::Dir)
    }

    /// 配对 `a`（现在）和 `b`（之前）的子项，逐个处理，返回各子项的变化。
    fn children(&mut self, a: Option<NodeId>, b: Option<NodeId>) -> Vec<i128> {
        let mut old: HashMap<String, NodeId> = match b {
            Some(b) if Self::is_dir(self.before, Some(b)) => self
                .before
                .children(b)
                .map(|c| (self.before.name(c).to_lowercase(), c))
                .collect(),
            _ => HashMap::new(),
        };
        let mut deltas = Vec::new();
        if let Some(a) = a.filter(|&a| Self::is_dir(self.now, Some(a))) {
            let kids: Vec<NodeId> = self.now.children(a).collect();
            for c in kids {
                let matched = old.remove(&self.now.name(c).to_lowercase());
                deltas.push(self.node(Some(c), matched));
            }
        }
        for (_, c) in old {
            deltas.push(self.node(None, Some(c)));
        }
        deltas
    }

    /// 处理一对节点（任一边可以没有），必要时记下这处变化，返回它的变化量。
    fn node(&mut self, a: Option<NodeId>, b: Option<NodeId>) -> i128 {
        let size_after = Self::size(self.now, a);
        let size_before = Self::size(self.before, b);
        let delta = i128::from(size_after) - i128::from(size_before);
        let is_dir = Self::is_dir(self.now, a) || Self::is_dir(self.before, b);
        // 没变的子树不用往下看；新增或删除的整个算一处。
        if a.is_none() || b.is_none() {
            if delta.abs() >= self.min {
                self.out.push(Change {
                    now: a,
                    before: b,
                    size_before,
                    size_after,
                    is_dir,
                });
            }
            return delta;
        }
        if delta == 0 {
            return 0;
        }
        let explained = if is_dir {
            let kids = self.children(a, b);
            kids.iter()
                .any(|&k| k.signum() == delta.signum() && k.abs() * 10 >= delta.abs() * 8)
        } else {
            false
        };
        if delta.abs() >= self.min && !explained {
            self.out.push(Change {
                now: a,
                before: b,
                size_before,
                size_after,
                is_dir,
            });
        }
        delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1 << 20;

    fn sizes(now: &FileTree, before: &FileTree, mut list: Vec<Change>) -> Vec<(String, i128)> {
        list.sort_by_key(|c| std::cmp::Reverse(c.delta()));
        list.into_iter()
            .map(|c| {
                let path = match (c.now, c.before) {
                    (Some(n), _) => now.path(n),
                    (None, Some(b)) => before.path(b),
                    (None, None) => unreachable!(),
                };
                (path, c.delta() / i128::from(MB))
            })
            .collect()
    }

    #[test]
    fn points_at_where_the_change_happened() {
        // 之前：projects\app\node_modules 100 MB，videos\a.mp4 500 MB，old.iso 300 MB
        let mut before = FileTree::new(r"D:\");
        let projects = before.add_dir(0, "projects");
        let app = before.add_dir(projects, "app");
        let nm = before.add_dir(app, "node_modules");
        before.add_file(nm, "x.js", 100 * MB, 100 * MB);
        let videos = before.add_dir(0, "videos");
        before.add_file(videos, "a.mp4", 500 * MB, 500 * MB);
        before.add_file(0, "old.iso", 300 * MB, 300 * MB);
        before.finalize();

        // 现在：node_modules 变成 900 MB，videos 多了 b.mp4 200 MB，old.iso 删了，新增 Games\big.pak 2000 MB
        let mut now = FileTree::new(r"D:\");
        let projects = now.add_dir(0, "Projects"); // 大小写不同也算同一个
        let app = now.add_dir(projects, "app");
        let nm = now.add_dir(app, "node_modules");
        now.add_file(nm, "x.js", 900 * MB, 900 * MB);
        let videos = now.add_dir(0, "videos");
        now.add_file(videos, "a.mp4", 500 * MB, 500 * MB);
        now.add_file(videos, "b.mp4", 200 * MB, 200 * MB);
        let games = now.add_dir(0, "Games");
        now.add_file(games, "big.pak", 2000 * MB, 2000 * MB);
        now.finalize();

        let list = changes(&now, &before, 50 * MB);
        assert_eq!(
            sizes(&now, &before, list),
            [
                (r"D:\Games".to_string(), 2000),
                (r"D:\Projects\app\node_modules\x.js".to_string(), 800),
                (r"D:\videos\b.mp4".to_string(), 200),
                (r"D:\old.iso".to_string(), -300),
            ]
        );
    }

    #[test]
    fn small_changes_are_left_out() {
        let mut before = FileTree::new("r");
        before.add_file(0, "a", 10 * MB, 10 * MB);
        before.finalize();
        let mut now = FileTree::new("r");
        now.add_file(0, "a", 12 * MB, 12 * MB);
        now.finalize();
        assert!(changes(&now, &before, 50 * MB).is_empty());
        assert_eq!(changes(&now, &before, MB).len(), 1);
    }

    #[test]
    fn a_folder_with_spread_out_growth_is_listed_itself() {
        let mut before = FileTree::new("r");
        let d = before.add_dir(0, "logs");
        before.add_file(d, "1.log", MB, MB);
        before.finalize();
        let mut now = FileTree::new("r");
        let d = now.add_dir(0, "logs");
        now.add_file(d, "1.log", MB, MB);
        for i in 2..12 {
            now.add_file(d, &format!("{i}.log"), 30 * MB, 30 * MB);
        }
        now.finalize();
        let list = changes(&now, &before, 100 * MB);
        assert_eq!(list.len(), 1);
        assert_eq!(now.path(list[0].now.unwrap()), r"r\logs");
        assert_eq!(list[0].delta(), i128::from(300 * MB));
    }
}
