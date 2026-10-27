//! 紧凑的内存文件树。
//!
//! 节点按列存放在平行数组里，彼此用 `u32` 索引引用，名称集中放在一个字符串池中。
//! 这样百万级节点只占几十 MB，并且没有逐节点的堆分配。
//!
//! 每个节点记两种大小：逻辑大小（文件内容的字节数）和占用空间（按簇分配、
//! 考虑压缩与稀疏后实际占用的磁盘空间）。
//!
//! 约定：建树时父节点必须先于子节点加入（父节点的 id 更小）。[`FileTree::finalize`]
//! 依赖这一点，只需倒序扫一遍就能把大小和文件数汇总到每个目录。
//!
//! finalize 之后树仍可修改（[`insert`](FileTree::insert)、[`detach`](FileTree::detach)、
//! [`move_to`](FileTree::move_to) 等），每次修改都会同步更新所有祖先的汇总，
//! 用于删除文件和按 USN 日志增量刷新。新节点总是追加在末尾，已有节点的 id 不变。
//!
//! 每个节点可以带一个 `u64` 键，表示它在来源中的编号（例如 MFT 记录号），
//! 增量刷新时用来把变化对应到节点；还可以带修改时间和访问时间（Unix 秒，0 表示未知）。

mod codec;
mod layout;
mod sunburst;

use std::cmp::Reverse;

pub use layout::{Rect, squarify};
pub use sunburst::Segment;

/// 节点在树中的索引。根节点固定为 0。
pub type NodeId = u32;

const NONE: NodeId = NodeId::MAX;
/// 没有键的节点在键列里存这个值。
const NO_KEY: u64 = u64::MAX;

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
    /// 节点在来源中的编号；没有设置过任何键时为空。
    keys: Vec<u64>,
    /// 修改时间（Unix 秒，0 表示未知）；没有设置过任何时间时为空。
    modified: Vec<u32>,
    /// 访问时间，规则同 `modified`。
    accessed: Vec<u32>,
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
            + cap(&self.keys)
            + cap(&self.modified)
            + cap(&self.accessed)
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
        self.keys.shrink_to_fit();
        self.modified.shrink_to_fit();
        self.accessed.shrink_to_fit();
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
        if !self.keys.is_empty() {
            self.keys.push(NO_KEY);
        }
        if !self.modified.is_empty() {
            self.modified.push(0);
            self.accessed.push(0);
        }
        id
    }

    /// 设置节点的修改时间和访问时间（Unix 秒，0 表示未知）。
    pub fn set_times(&mut self, id: NodeId, modified: u32, accessed: u32) {
        if self.modified.is_empty() {
            self.modified = vec![0; self.len()];
            self.accessed = vec![0; self.len()];
        }
        self.modified[id as usize] = modified;
        self.accessed[id as usize] = accessed;
    }

    /// 修改时间（Unix 秒）；未知时为 0。
    pub fn modified(&self, id: NodeId) -> u32 {
        self.modified.get(id as usize).copied().unwrap_or(0)
    }

    /// 访问时间（Unix 秒）；未知时为 0。
    pub fn accessed(&self, id: NodeId) -> u32 {
        self.accessed.get(id as usize).copied().unwrap_or(0)
    }

    /// 是否记录了时间。
    pub fn has_times(&self) -> bool {
        !self.modified.is_empty()
    }

    /// 设置节点的键。
    pub fn set_key(&mut self, id: NodeId, key: u64) {
        if self.keys.is_empty() {
            self.keys = vec![NO_KEY; self.len()];
        }
        self.keys[id as usize] = key;
    }

    /// 节点的键；没有设置过时为 `None`。
    pub fn key(&self, id: NodeId) -> Option<u64> {
        self.keys.get(id as usize).copied().filter(|&k| k != NO_KEY)
    }

    /// 是否有节点设置过键。
    pub fn has_keys(&self) -> bool {
        !self.keys.is_empty()
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

    /// 把节点从树中摘除（例如文件已被删除），并从所有祖先的汇总中扣除它的大小和文件数。
    /// 节点本身的数据保留，只是不再挂在树上。根节点或已摘除的节点返回 `false`。
    ///
    /// 硬链接的大小只记在其中一个名称上；摘除那个名称后，另一个名称仍显示为 0。
    ///
    /// # Panics
    ///
    /// 在 [`finalize`](Self::finalize) 之前调用时。
    pub fn detach(&mut self, id: NodeId) -> bool {
        assert!(self.finalized, "detach requires a finalized tree");
        let Some(parent) = self.parent(id) else {
            return false;
        };
        let next = self.next_sibling[id as usize];
        if self.first_child[parent as usize] == id {
            self.first_child[parent as usize] = next;
        } else {
            let Some(prev) = self
                .children(parent)
                .find(|&c| self.next_sibling[c as usize] == id)
            else {
                return false;
            };
            self.next_sibling[prev as usize] = next;
        }

        let (size, allocated, files) = (self.size(id), self.allocated(id), self.file_count(id));
        self.add_to_ancestors(
            parent,
            -(size as i128),
            -(allocated as i128),
            -i64::from(files),
        );
        self.parent[id as usize] = NONE;
        self.next_sibling[id as usize] = NONE;
        true
    }

    /// 把 `dir` 及其所有祖先的汇总加上给定的增量（可以为负）。
    fn add_to_ancestors(&mut self, dir: NodeId, size: i128, allocated: i128, files: i64) {
        fn apply<T: TryFrom<i128> + Into<i128> + Copy + Default>(v: &mut T, delta: i128) {
            *v = T::try_from(((*v).into() + delta).max(0)).unwrap_or_default();
        }
        let mut cur = Some(dir);
        while let Some(n) = cur {
            let i = n as usize;
            apply(&mut self.size[i], size);
            apply(&mut self.allocated[i], allocated);
            apply(&mut self.files[i], i128::from(files));
            cur = self.parent(n);
        }
    }

    /// 在 finalize 之后加入一个节点，并更新祖先的汇总。新节点追加在末尾。
    ///
    /// # Panics
    ///
    /// `parent` 不是目录时。
    pub fn insert(
        &mut self,
        parent: NodeId,
        name: &str,
        kind: NodeKind,
        size: u64,
        allocated: u64,
    ) -> NodeId {
        assert!(self.finalized, "insert requires a finalized tree");
        assert!(
            self.kind.get(parent as usize) == Some(&NodeKind::Dir),
            "parent {parent} is not a directory"
        );
        let (size, allocated) = if kind == NodeKind::Dir {
            (0, 0)
        } else {
            (size, allocated)
        };
        let id = self.push(parent, name, kind, size, allocated);
        self.next_sibling[id as usize] = self.first_child[parent as usize];
        self.first_child[parent as usize] = id;
        let files = i64::from(self.files[id as usize]);
        self.add_to_ancestors(parent, size.into(), allocated.into(), files);
        id
    }

    /// 修改文件的大小和占用空间，并更新祖先的汇总。目录不做任何事。
    pub fn set_file_size(&mut self, id: NodeId, size: u64, allocated: u64) {
        let i = id as usize;
        if self.kind[i] != NodeKind::File {
            return;
        }
        let ds = i128::from(size) - i128::from(self.size[i]);
        let da = i128::from(allocated) - i128::from(self.allocated[i]);
        self.size[i] = size;
        self.allocated[i] = allocated;
        if let Some(p) = self.parent(id) {
            self.add_to_ancestors(p, ds, da, 0);
        }
    }

    /// 改名。旧名称留在名称池里不回收。
    pub fn rename(&mut self, id: NodeId, name: &str) {
        if self.name(id) == name {
            return;
        }
        let len = u16::try_from(name.len()).expect("name longer than u16::MAX bytes");
        let start = u32::try_from(self.names.len()).expect("name pool exceeds 4 GiB");
        self.names.push_str(name);
        self.name_start[id as usize] = start;
        self.name_len[id as usize] = len;
    }

    /// 把节点（连同子树）移到目录 `new_parent` 下，并更新新旧两边祖先的汇总。
    /// 节点原本已经摘除（不在任何目录下）时，相当于重新挂回去。
    /// `new_parent` 是节点自身或其后代时不移动，返回 `false`。
    ///
    /// # Panics
    ///
    /// `new_parent` 不是目录时。
    pub fn move_to(&mut self, id: NodeId, new_parent: NodeId) -> bool {
        assert!(
            self.kind.get(new_parent as usize) == Some(&NodeKind::Dir),
            "parent {new_parent} is not a directory"
        );
        if id == self.root() || self.is_within(new_parent, id) {
            return false;
        }
        if self.parent(id) == Some(new_parent) {
            return true;
        }
        if self.parent(id).is_some() {
            self.detach(id);
        }
        let i = id as usize;
        self.parent[i] = new_parent;
        self.next_sibling[i] = self.first_child[new_parent as usize];
        self.first_child[new_parent as usize] = id;
        let (size, allocated, files) = (self.size[i], self.allocated[i], self.files[i]);
        self.add_to_ancestors(new_parent, size.into(), allocated.into(), files.into());
        true
    }

    /// `id` 是否是 `ancestor` 本身或其后代。
    fn is_within(&self, id: NodeId, ancestor: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(n) = cur {
            if n == ancestor {
                return true;
            }
            cur = self.parent(n);
        }
        false
    }

    /// 节点是否还挂在树上（沿父节点能走到根）。摘除的节点及其子树返回 `false`。
    pub fn is_attached(&self, id: NodeId) -> bool {
        (id as usize) < self.len() && self.is_within(id, self.root())
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
    fn detach_updates_ancestors_and_unlinks() {
        let mut t = sample();
        let users = t.child_by_name(0, "Users").unwrap();
        let docs = t.child_by_name(users, "Docs").unwrap();
        let b = t.child_by_name(docs, "b.bin").unwrap();

        assert!(t.detach(b));
        assert_eq!(t.size(docs), 100);
        assert_eq!(t.size(users), 5_100);
        assert_eq!(t.size(0), 7_100);
        assert_eq!(t.allocated(0), 16_384);
        assert_eq!(t.file_count(0), 3);
        assert_eq!(t.child_by_name(docs, "b.bin"), None);
        assert!(!t.detach(b), "already detached");

        // 摘除整个目录，并且是父目录的第一个子节点。
        let first = t.children(0).next().unwrap();
        let before = t.size(0) - t.size(first);
        assert!(t.detach(first));
        assert_eq!(t.size(0), before);
        assert!(!t.detach(0), "root cannot be detached");
    }

    #[test]
    fn edits_after_finalize_keep_totals() {
        let mut t = sample();
        let users = t.child_by_name(0, "Users").unwrap();
        let docs = t.child_by_name(users, "Docs").unwrap();

        let f = t.insert(docs, "new.log", NodeKind::File, 50, 4_096);
        assert_eq!(t.size(docs), 1_050);
        assert_eq!(t.size(0), 8_050);
        assert_eq!(t.file_count(0), 5);

        t.set_file_size(f, 10, 0);
        assert_eq!(t.size(users), 5_010 + 1_000);
        assert_eq!(t.allocated(0), 20_480);

        let d = t.insert(0, "Archive", NodeKind::Dir, 999, 999);
        assert_eq!(t.size(d), 0, "directories start empty");
        assert!(t.move_to(docs, d));
        assert_eq!(t.size(d), 1_010);
        assert_eq!(t.size(users), 5_000);
        assert_eq!(t.size(0), 8_010);
        assert_eq!(t.path(f), r"C:\Archive\Docs\new.log");

        assert!(!t.move_to(d, docs), "cannot move into own subtree");
        t.rename(d, "旧");
        assert_eq!(t.path(f), r"C:\旧\Docs\new.log");

        t.detach(d);
        assert!(!t.is_attached(f));
        assert!(t.is_attached(users));
        assert_eq!(t.size(0), 7_000);
        // 重新挂回去。
        assert!(t.move_to(d, users));
        assert_eq!(t.size(0), 8_010);
        assert!(t.is_attached(f));
    }

    #[test]
    fn times_are_optional() {
        let mut t = sample();
        assert!(!t.has_times());
        assert_eq!(t.modified(1), 0);
        t.set_times(1, 1_700_000_000, 1_750_000_000);
        assert_eq!(
            (t.modified(1), t.accessed(1)),
            (1_700_000_000, 1_750_000_000)
        );
        assert_eq!(t.modified(2), 0);
        let n = t.insert(0, "new", NodeKind::File, 1, 1);
        assert_eq!(t.modified(n), 0);
    }

    #[test]
    fn keys_are_optional() {
        let mut t = sample();
        assert!(!t.has_keys());
        assert_eq!(t.key(1), None);
        t.set_key(1, 42);
        assert_eq!(t.key(1), Some(42));
        assert_eq!(t.key(2), None);
        let n = t.insert(0, "x", NodeKind::File, 1, 1);
        assert_eq!(t.key(n), None);
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
