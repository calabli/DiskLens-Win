//! 紧凑的内存文件树。
//!
//! 节点按列存放在平行数组里，彼此用 `u32` 索引引用，名称集中放在一个字符串池中。
//! 这样百万级节点只占几十 MB，并且没有逐节点的堆分配。
//!
//! 每个节点记两种大小：逻辑大小（文件内容的字节数）和占用空间（按簇分配、
//! 考虑压缩与稀疏后实际占用的磁盘空间）。
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
    /// 文件：逻辑大小；目录：`finalize` 之后为整棵子树的逻辑大小。
    size: Vec<u64>,
    /// 文件：占用空间；目录：`finalize` 之后为整棵子树的占用空间。
    allocated: Vec<u64>,
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
        tree.push(NONE, root_name, NodeKind::Dir, 0, 0);
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

    /// 树占用的堆内存字节数（按已分配容量计）。
    pub fn heap_bytes(&self) -> usize {
        fn cap<T>(v: &Vec<T>) -> usize {
            v.capacity() * size_of::<T>()
        }
        cap(&self.parent)
            + cap(&self.first_child)
            + cap(&self.next_sibling)
            + cap(&self.kind)
            + cap(&self.size)
            + cap(&self.allocated)
            + cap(&self.files)
            + cap(&self.name_start)
            + cap(&self.name_len)
            + self.names.capacity()
    }

    /// 释放 `Vec` 扩容留下的空闲容量。树在 `finalize` 之后不再增长。
    fn shrink_to_fit(&mut self) {
        self.parent.shrink_to_fit();
        self.first_child.shrink_to_fit();
        self.next_sibling.shrink_to_fit();
        self.kind.shrink_to_fit();
        self.size.shrink_to_fit();
        self.allocated.shrink_to_fit();
        self.files.shrink_to_fit();
        self.name_start.shrink_to_fit();
        self.name_len.shrink_to_fit();
        self.names.shrink_to_fit();
    }

    /// 在 `parent` 下加入一个目录。
    ///
    /// # Panics
    ///
    /// `parent` 不是已存在的目录，或在 [`finalize`](Self::finalize) 之后继续加入节点时。
    pub fn add_dir(&mut self, parent: NodeId, name: &str) -> NodeId {
        self.link(parent, name, NodeKind::Dir, 0, 0)
    }

    /// 在 `parent` 下加入一个文件，`size` 为逻辑大小，`allocated` 为占用空间。
    ///
    /// # Panics
    ///
    /// 同 [`add_dir`](Self::add_dir)。
    pub fn add_file(&mut self, parent: NodeId, name: &str, size: u64, allocated: u64) -> NodeId {
        self.link(parent, name, NodeKind::File, size, allocated)
    }

    fn link(
        &mut self,
        parent: NodeId,
        name: &str,
        kind: NodeKind,
        size: u64,
        allocated: u64,
    ) -> NodeId {
        assert!(!self.finalized, "cannot add nodes after finalize");
        assert!(
            self.kind.get(parent as usize) == Some(&NodeKind::Dir),
            "parent {parent} is not a directory"
        );
        let id = self.push(parent, name, kind, size, allocated);
        self.next_sibling[id as usize] = self.first_child[parent as usize];
        self.first_child[parent as usize] = id;
        id
    }

    fn push(
        &mut self,
        parent: NodeId,
        name: &str,
        kind: NodeKind,
        size: u64,
        allocated: u64,
    ) -> NodeId {
        let id = NodeId::try_from(self.kind.len()).expect("more than u32::MAX nodes");
        // NTFS 名称最多 255 个 UTF-16 单元，换成 UTF-8 不超过 765 字节。
        let len = u16::try_from(name.len()).expect("name longer than u16::MAX bytes");
        let start = u32::try_from(self.names.len()).expect("name pool exceeds 4 GiB");
        self.names.push_str(name);

        self.parent.push(parent);
        self.first_child.push(NONE);
        self.next_sibling.push(NONE);
        self.kind.push(kind);
        self.size.push(size);
        self.allocated.push(allocated);
        self.files.push(u32::from(kind == NodeKind::File));
        self.name_start.push(start);
        self.name_len.push(len);
        id
    }

    /// 把大小、占用空间和文件数逐级汇总到目录，并释放多余容量。整棵树建完后调用一次。
    pub fn finalize(&mut self) {
        if self.finalized {
            return;
        }
        for id in (1..self.len()).rev() {
            let p = self.parent[id] as usize;
            self.size[p] += self.size[id];
            self.allocated[p] += self.allocated[id];
            self.files[p] += self.files[id];
        }
        self.shrink_to_fit();
        self.finalized = true;
    }

    pub fn name(&self, id: NodeId) -> &str {
        let start = self.name_start[id as usize] as usize;
        &self.names[start..start + self.name_len[id as usize] as usize]
    }

    pub fn kind(&self, id: NodeId) -> NodeKind {
        self.kind[id as usize]
    }

    /// 逻辑大小。
    pub fn size(&self, id: NodeId) -> u64 {
        self.size[id as usize]
    }

    /// 占用空间。
    pub fn allocated(&self, id: NodeId) -> u64 {
        self.allocated[id as usize]
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

    /// 子节点按逻辑大小从大到小排序，大小相同时按名称排序。
    pub fn children_by_size(&self, id: NodeId) -> Vec<NodeId> {
        let mut kids: Vec<NodeId> = self.children(id).collect();
        kids.sort_by_key(|&c| (Reverse(self.size(c)), self.name(c)));
        kids
    }

    /// 按名称查找直接子节点。
    pub fn child_by_name(&self, id: NodeId, name: &str) -> Option<NodeId> {
        self.children(id).find(|&c| self.name(c) == name)
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
        let users = t.add_dir(0, "Users");
        let docs = t.add_dir(users, "Docs");
        t.add_file(docs, "a.txt", 100, 4_096);
        t.add_file(docs, "b.bin", 900, 4_096);
        t.add_file(users, "c.iso", 5_000, 8_192);
        t.add_file(0, "pagefile.sys", 2_000, 4_096);
        t.finalize();
        t
    }

    #[test]
    fn sizes_and_counts_roll_up() {
        let t = sample();
        assert_eq!(t.size(t.root()), 8_000);
        assert_eq!(t.allocated(t.root()), 20_480);
        assert_eq!(t.file_count(t.root()), 4);
        let users = t.child_by_name(0, "Users").unwrap();
        assert_eq!(t.size(users), 6_000);
        assert_eq!(t.allocated(users), 16_384);
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
    fn finalize_releases_spare_capacity() {
        let mut t = FileTree::new("root");
        for i in 0..1_000 {
            t.add_file(0, &format!("f{i}"), 1, 1);
        }
        let before = t.heap_bytes();
        t.finalize();
        assert!(t.heap_bytes() <= before);
        // 每个节点的列加起来约 37 字节，再加上名称。
        assert!(t.heap_bytes() < 1_001 * 48, "{}", t.heap_bytes());
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
        let f = t.add_file(0, "f", 1, 1);
        t.add_file(f, "x", 1, 1);
    }
}
