//! 把文件树写成紧凑的二进制格式，以及读回来。
//!
//! 格式（全部小端）：`DLTR`、版本 u32、节点数 n u32、名称池字节数 u32，
//! 然后依次是各列：parent、first_child、next_sibling（u32 × n）、kind（u8 × n）、
//! size、allocated（u64 × n）、files、name_start（u32 × n）、name_len（u16 × n）、名称池，
//! 版本 2 起最后是键列：有无键 u8，有键时再跟 u64 × n。
//!
//! 读取时检查所有索引都在范围内，并从根出发走一遍：每个子节点的 parent 必须指回所在目录，
//! 且只能被访问一次。走不到的节点（已摘除的）把链接清空。这样损坏的数据不会造成越界或死循环。

use std::io::{self, Read, Write};

use super::{FileTree, NONE, NodeId, NodeKind};

const MAGIC: &[u8; 4] = b"DLTR";
const VERSION: u32 = 2;

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("文件树数据损坏：{msg}"))
}

fn write_u32s(w: &mut impl Write, v: &[u32]) -> io::Result<()> {
    v.iter().try_for_each(|x| w.write_all(&x.to_le_bytes()))
}

fn write_u64s(w: &mut impl Write, v: &[u64]) -> io::Result<()> {
    v.iter().try_for_each(|x| w.write_all(&x.to_le_bytes()))
}

fn read_bytes(r: &mut impl Read, len: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u32s(r: &mut impl Read, n: usize) -> io::Result<Vec<u32>> {
    let raw = read_bytes(r, n * 4)?;
    Ok(raw
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect())
}

fn read_u64s(r: &mut impl Read, n: usize) -> io::Result<Vec<u64>> {
    let raw = read_bytes(r, n * 8)?;
    Ok(raw
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| u64::from_le_bytes(*c))
        .collect())
}

impl FileTree {
    /// 把树写入 `w`。只能写已经 [`finalize`](Self::finalize) 的树。
    ///
    /// # Panics
    ///
    /// 树还没有 finalize 时。
    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        assert!(self.finalized, "only finalized trees can be written");
        let n = self.len() as u32;
        w.write_all(MAGIC)?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&n.to_le_bytes())?;
        w.write_all(&(self.names.len() as u32).to_le_bytes())?;
        write_u32s(w, &self.parent)?;
        write_u32s(w, &self.first_child)?;
        write_u32s(w, &self.next_sibling)?;
        let kinds: Vec<u8> = self
            .kind
            .iter()
            .map(|k| u8::from(*k == NodeKind::File))
            .collect();
        w.write_all(&kinds)?;
        write_u64s(w, &self.size)?;
        write_u64s(w, &self.allocated)?;
        write_u32s(w, &self.files)?;
        write_u32s(w, &self.name_start)?;
        self.name_len
            .iter()
            .try_for_each(|x| w.write_all(&x.to_le_bytes()))?;
        w.write_all(self.names.as_bytes())?;
        w.write_all(&[u8::from(self.has_keys())])?;
        write_u64s(w, &self.keys)
    }

    /// 从 `r` 读回 [`write_to`](Self::write_to) 写出的树。数据不完整或不一致时返回
    /// `InvalidData` 错误。
    pub fn read_from(r: &mut impl Read) -> io::Result<Self> {
        let mut magic = [0; 4];
        r.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(invalid("文件头不对"));
        }
        let version = read_u32(r)?;
        if !(1..=VERSION).contains(&version) {
            return Err(invalid("版本不支持"));
        }
        let n = read_u32(r)? as usize;
        let names_len = read_u32(r)? as usize;
        if n == 0 || n == NONE as usize {
            return Err(invalid("节点数不对"));
        }

        let parent = read_u32s(r, n)?;
        let first_child = read_u32s(r, n)?;
        let next_sibling = read_u32s(r, n)?;
        let kind = read_bytes(r, n)?
            .into_iter()
            .map(|b| match b {
                0 => Ok(NodeKind::Dir),
                1 => Ok(NodeKind::File),
                _ => Err(invalid("节点类型不对")),
            })
            .collect::<io::Result<Vec<_>>>()?;
        let size = read_u64s(r, n)?;
        let allocated = read_u64s(r, n)?;
        let files = read_u32s(r, n)?;
        let name_start = read_u32s(r, n)?;
        let name_len: Vec<u16> = read_bytes(r, n * 2)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let names = String::from_utf8(read_bytes(r, names_len)?)
            .map_err(|_| invalid("名称不是有效的 UTF-8"))?;
        let mut flag = [0u8; 1];
        let keys = if version >= 2 && {
            r.read_exact(&mut flag)?;
            flag[0] == 1
        } {
            read_u64s(r, n)?
        } else {
            Vec::new()
        };

        let mut tree = Self {
            parent,
            first_child,
            next_sibling,
            kind,
            size,
            allocated,
            files,
            name_start,
            name_len,
            names,
            keys,
            finalized: true,
        };
        tree.validate()?;
        Ok(tree)
    }

    /// 检查读回的数据，见模块说明。
    fn validate(&mut self) -> io::Result<()> {
        let n = self.len();
        let in_range = |v: NodeId| v == NONE || (v as usize) < n;
        for i in 0..n {
            if !(in_range(self.parent[i])
                && in_range(self.first_child[i])
                && in_range(self.next_sibling[i]))
            {
                return Err(invalid("节点编号越界"));
            }
            let start = self.name_start[i] as usize;
            let end = start + self.name_len[i] as usize;
            if end > self.names.len()
                || !self.names.is_char_boundary(start)
                || !self.names.is_char_boundary(end)
            {
                return Err(invalid("名称位置不对"));
            }
        }
        if self.parent[0] != NONE || self.kind[0] != NodeKind::Dir {
            return Err(invalid("根节点不对"));
        }

        let mut seen = vec![false; n];
        seen[0] = true;
        let mut stack = vec![0 as NodeId];
        while let Some(dir) = stack.pop() {
            let mut child = self.first_child[dir as usize];
            if child != NONE && self.kind[dir as usize] != NodeKind::Dir {
                return Err(invalid("文件下面挂着子节点"));
            }
            while child != NONE {
                let c = child as usize;
                if seen[c] || self.parent[c] != dir {
                    return Err(invalid("节点链接不对"));
                }
                seen[c] = true;
                stack.push(child);
                child = self.next_sibling[c];
            }
        }
        for (i, _) in seen.iter().enumerate().filter(|(_, s)| !**s) {
            self.parent[i] = NONE;
            self.first_child[i] = NONE;
            self.next_sibling[i] = NONE;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FileTree {
        let mut t = FileTree::new(r"D:\");
        let a = t.add_dir(0, "资料");
        t.add_file(a, "报告.docx", 1_000, 4_096);
        let b = t.add_dir(a, "b");
        t.add_file(b, "x.bin", 7, 4_096);
        t.add_file(0, "c.iso", 5_000, 8_192);
        t.finalize();
        t
    }

    fn roundtrip(t: &FileTree) -> FileTree {
        let mut buf = Vec::new();
        t.write_to(&mut buf).unwrap();
        FileTree::read_from(&mut buf.as_slice()).unwrap()
    }

    fn describe(t: &FileTree) -> Vec<(String, u64, u64, u32)> {
        (0..t.len() as NodeId)
            .filter(|&i| t.is_attached(i))
            .map(|i| (t.path(i), t.size(i), t.allocated(i), t.file_count(i)))
            .collect()
    }

    #[test]
    fn roundtrip_keeps_everything() {
        let t = sample();
        let back = roundtrip(&t);
        assert_eq!(describe(&back), describe(&t));
        let names: Vec<&str> = back
            .children_by_size(0)
            .iter()
            .map(|&c| back.name(c))
            .collect();
        assert_eq!(names, ["c.iso", "资料"]);
    }

    #[test]
    fn roundtrip_after_detach() {
        let mut t = sample();
        let a = t.child_by_name(0, "资料").unwrap();
        let b = t.child_by_name(a, "b").unwrap();
        t.detach(b);
        let back = roundtrip(&t);
        assert_eq!(describe(&back), describe(&t));
        assert_eq!(back.child_by_name(a, "b"), None);
        assert_eq!(back.size(0), 6_000);
    }

    #[test]
    fn rejects_damaged_data() {
        let t = sample();
        let mut buf = Vec::new();
        t.write_to(&mut buf).unwrap();

        assert!(
            FileTree::read_from(&mut &buf[..buf.len() - 1]).is_err(),
            "truncated"
        );
        let mut bad = buf.clone();
        bad[0] = b'X';
        assert!(FileTree::read_from(&mut bad.as_slice()).is_err(), "magic");

        // 把第 2 个节点的 parent 改成指向自己之后的节点。
        let mut bad = buf.clone();
        let off = 16 + 4;
        bad[off..off + 4].copy_from_slice(&3u32.to_le_bytes());
        let err = FileTree::read_from(&mut bad.as_slice()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // 让根的第一个子节点的 next_sibling 指向自己，形成环。
        let mut bad = buf.clone();
        let first = t.children(0).next().unwrap();
        let n = t.len();
        let off = 16 + 8 * n + 4 * first as usize;
        bad[off..off + 4].copy_from_slice(&first.to_le_bytes());
        assert!(FileTree::read_from(&mut bad.as_slice()).is_err(), "cycle");
    }

    #[test]
    fn roundtrip_keys_and_moves() {
        let mut t = sample();
        for i in 0..t.len() as NodeId {
            t.set_key(i, 100 + u64::from(i));
        }
        // 把旧目录移进新建的目录：新的父节点 id 比自己大。
        let a = t.child_by_name(0, "资料").unwrap();
        let newer = t.insert(0, "新", NodeKind::Dir, 0, 0);
        assert!(t.move_to(a, newer));
        let back = roundtrip(&t);
        assert_eq!(describe(&back), describe(&t));
        assert_eq!(back.key(a), Some(100 + u64::from(a)));
        assert_eq!(back.key(newer), None);
        assert_eq!(back.path(a), r"D:\新\资料");
    }

    #[test]
    fn detached_nodes_are_unlinked_on_load() {
        let mut t = sample();
        let a = t.child_by_name(0, "资料").unwrap();
        let b = t.child_by_name(a, "b").unwrap();
        t.detach(a);
        let back = roundtrip(&t);
        assert!(!back.is_attached(a));
        assert!(!back.is_attached(b));
        assert_eq!(back.parent(b), None);
    }
}
