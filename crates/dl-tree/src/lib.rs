//! 紧凑的内存文件树。
//!
//! 节点按列存放在平行数组里，彼此用 `u32` 索引引用，名称集中放在一个字符串池中。
//! 这样百万级节点只占几十 MB，并且没有逐节点的堆分配。
//!
//! 约定：父节点必须先于子节点加入（父节点的 id 更小）。[`FileTree::finalize`]
//! 依赖这一点，只需倒序扫一遍就能把大小和文件数汇总到每个目录。

use std::cmp::Reverse;

/// 节点在树中的索引。根节点固定为 0。
pub type NodeId = u32;

const NONE: NodeId = NodeId::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Dir,
    File,
}

#[derive(Debug, Default)]
pub struct FileTree {
    parent: Vec<NodeId>,
    first_child: Vec<NodeId>,
    next_sibling: Vec<NodeId>,
    kind: Vec<NodeKind>,
    /// 文件：自身大小；目录：`finalize` 之后为整棵子树的大小。
    size: Vec<u64>,
    /// 文件：1；目录：`finalize` 之后为子树中的文件总数。
    files: Vec<u32>,
    name_start: Vec<u32>,
    name_len: Vec<u16>,
    names: String,
    finalized: bool,
}

impl FileTree {
    /// 新建一棵只有根目录的树。
    pub fn new(root_name: &str) -> Self {
        let mut tree = Self::default();
        tree.push(NONE, root_name, NodeKind::Dir, 0);
        tree
    }

    pub const fn root(&self) -> NodeId {
        0
    }

    pub fn len(&self) -> usize {
        self.kind.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kind.is_empty()
    }

    /// 在 `parent` 下加入一个节点，返回它的 id。目录的 `size` 传 0 即可。
    ///
    /// # Panics
    ///
    /// `parent` 不是已存在的目录，或在 [`finalize`](Self::finalize) 之后继续加入节点时。
    pub fn add(&mut self, parent: NodeId, name: &str, kind: NodeKind, size: u64) -> NodeId {
        assert!(!self.finalized, "cannot add nodes after finalize");
        assert!(
            self.kind.get(parent as usize) == Some(&NodeKind::Dir),
            "parent {parent} is not a directory"
        );
        let id = self.push(parent, name, kind, size);
        self.next_sibling[id as usize] = self.first_child[parent as usize];
        self.first_child[parent as usize] = id;
        id
    }

    fn push(&mut self, parent: NodeId, name: &str, kind: NodeKind, size: u64) -> NodeId {
        let id = NodeId::try_from(self.kind.len()).expect("more than u32::MAX nodes");
        // NTFS 名称最多 255 个 UTF-16 单元，换成 UTF-8 不超过 765 字节。
        let len = u16::try_from(name.len()).expect("name longer than u16::MAX bytes");
        let start = u32::try_from(self.names.len()).expect("name pool exceeds 4 GiB");
        self.names.push_str(name);

        self.parent.push(parent);
        self.first_child.push(NONE);
        self.next_sibling.push(NONE);
        self.kind.push(kind);
        self.size
            .push(if kind == NodeKind::File { size } else { 0 });
        self.files.push(u32::from(kind == NodeKind::File));
        self.name_start.push(start);
        self.name_len.push(len);
        id
    }

    /// 把文件大小和文件数逐级汇总到目录。整棵树建完后调用一次。
    pub fn finalize(&mut self) {
        if self.finalized {
            return;
        }
        for id in (1..self.len()).rev() {
            let p = self.parent[id] as usize;
            self.size[p] += self.size[id];
            self.files[p] += self.files[id];
        }
        self.finalized = true;
    }

    pub fn name(&self, id: NodeId) -> &str {
        let start = self.name_start[id as usize] as usize;
        &self.names[start..start + self.name_len[id as usize] as usize]
    }

    pub fn kind(&self, id: NodeId) -> NodeKind {
        self.kind[id as usize]
    }

    pub fn size(&self, id: NodeId) -> u64 {
        self.size[id as usize]
    }

    pub fn file_count(&self, id: NodeId) -> u32 {
        self.files[id as usize]
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        let p = self.parent[id as usize];
        (p != NONE).then_some(p)
    }

    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            tree: self,
            next: self.first_child[id as usize],
        }
    }

    /// 子节点按大小从大到小排序，大小相同时按名称排序。
    pub fn children_by_size(&self, id: NodeId) -> Vec<NodeId> {
        let mut kids: Vec<NodeId> = self.children(id).collect();
        kids.sort_by_key(|&c| (Reverse(self.size(c)), self.name(c)));
        kids
    }

    /// 从根到该节点的完整路径，用 `\` 连接。
    pub fn path(&self, id: NodeId) -> String {
        let mut parts = vec![self.name(id)];
        let mut cur = id;
        while let Some(p) = self.parent(cur) {
            parts.push(self.name(p));
            cur = p;
        }
        parts.reverse();
        let mut out = String::new();
        for part in parts {
            if !out.is_empty() && !out.ends_with('\\') {
                out.push('\\');
            }
            out.push_str(part);
        }
        out
    }
}

pub struct Children<'a> {
    tree: &'a FileTree,
    next: NodeId,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        if self.next == NONE {
            return None;
        }
        let id = self.next;
        self.next = self.tree.next_sibling[id as usize];
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FileTree {
        let mut t = FileTree::new(r"C:\");
        let users = t.add(0, "Users", NodeKind::Dir, 0);
        let docs = t.add(users, "Docs", NodeKind::Dir, 0);
        t.add(docs, "a.txt", NodeKind::File, 100);
        t.add(docs, "b.bin", NodeKind::File, 900);
        t.add(users, "c.iso", NodeKind::File, 5_000);
        t.add(0, "pagefile.sys", NodeKind::File, 2_000);
        t.finalize();
        t
    }

    #[test]
    fn sizes_and_counts_roll_up() {
        let t = sample();
        assert_eq!(t.size(t.root()), 8_000);
        assert_eq!(t.file_count(t.root()), 4);
        let users = t.children(0).find(|&c| t.name(c) == "Users").unwrap();
        assert_eq!(t.size(users), 6_000);
        assert_eq!(t.file_count(users), 3);
    }

    #[test]
    fn children_sorted_by_size() {
        let t = sample();
        let names: Vec<&str> = t
            .children_by_size(0)
            .into_iter()
            .map(|c| t.name(c))
            .collect();
        assert_eq!(names, ["Users", "pagefile.sys"]);
    }

    #[test]
    fn path_joins_from_root() {
        let t = sample();
        let file = (0..t.len() as NodeId)
            .find(|&i| t.name(i) == "b.bin")
            .unwrap();
        assert_eq!(t.path(file), r"C:\Users\Docs\b.bin");
        assert_eq!(t.path(t.root()), r"C:\");
    }

    #[test]
    fn finalize_is_idempotent() {
        let mut t = sample();
        t.finalize();
        assert_eq!(t.size(t.root()), 8_000);
    }

    #[test]
    #[should_panic(expected = "not a directory")]
    fn rejects_file_as_parent() {
        let mut t = FileTree::new("root");
        let f = t.add(0, "f", NodeKind::File, 1);
        t.add(f, "x", NodeKind::File, 1);
    }
}
