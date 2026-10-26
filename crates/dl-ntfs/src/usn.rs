//! USN 日志增量刷新。
//!
//! NTFS 把每次文件变化（创建、删除、改名、移动、写入）按顺序记在 USN 日志里。
//! 扫描前记下日志的位置（[`JournalPos`]），刷新时只读从那以后的记录：
//!
//! 1. [`parse_records`]：解析 `FSCTL_READ_USN_JOURNAL` 返回的 `USN_RECORD_V2`；
//! 2. [`fold`]：同一个文件可能有很多条记录，合并成它最后的状态 [`Change`]；
//! 3. [`apply`]：按记录号（树节点的键）找到节点，删除、加入、改名、移动，并更新文件大小。
//!
//! 日志里没有文件大小，大小由调用方按文件 id 现查（Windows 上见 [`FileSizer`]）。
//! 日志被重建、或者变化太多旧记录已被覆盖时返回 [`Error::Journal`]，调用方应完整重扫。

use std::collections::HashMap;

use dl_tree::{FileTree, NodeId, NodeKind};

use crate::record::REF_MASK;
use crate::{Error, Result, le_u16, le_u32, le_u64};

pub const REASON_FILE_CREATE: u32 = 0x0000_0100;
pub const REASON_FILE_DELETE: u32 = 0x0000_0200;
pub const REASON_RENAME_OLD_NAME: u32 = 0x0000_1000;
pub const REASON_RENAME_NEW_NAME: u32 = 0x0000_2000;

pub const ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const ATTRIBUTE_SPARSE: u32 = 0x200;
pub const ATTRIBUTE_COMPRESSED: u32 = 0x800;

/// 根目录的记录号。
const ROOT_RECORD: u64 = 5;
/// `USN_RECORD_V2` 文件名之前的固定部分。
const RECORD_V2_HEADER: usize = 60;

/// 日志中的一个位置：日志 id（日志重建后会变）和下一条记录的 USN。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalPos {
    pub id: u64,
    pub next_usn: i64,
}

/// 一条 USN 记录中用得到的字段。文件引用号的高 16 位是序列号，记录被复用时会变。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsnRecord {
    pub frn: u64,
    pub parent: u64,
    pub usn: i64,
    pub reason: u32,
    pub attributes: u32,
    pub name: String,
}

/// 解析一段连续的 USN 记录。只认版本 2，其他版本跳过。
pub fn parse_records(buf: &[u8]) -> Result<Vec<UsnRecord>> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + 8 <= buf.len() {
        let len = le_u32(buf, off)? as usize;
        if len < 8 || off + len > buf.len() {
            return Err(Error::Corrupt("USN record length"));
        }
        let rec = &buf[off..off + len];
        if le_u16(rec, 4)? == 2 {
            if len < RECORD_V2_HEADER {
                return Err(Error::Corrupt("USN record length"));
            }
            let name_len = usize::from(le_u16(rec, 56)?);
            let name_off = usize::from(le_u16(rec, 58)?);
            let raw = rec
                .get(name_off..name_off + name_len)
                .ok_or(Error::Corrupt("USN record name"))?;
            let units: Vec<u16> = raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            out.push(UsnRecord {
                frn: le_u64(rec, 8)?,
                parent: le_u64(rec, 16)?,
                usn: le_u64(rec, 24)? as i64,
                reason: le_u32(rec, 40)?,
                attributes: le_u32(rec, 52)?,
                name: String::from_utf16_lossy(&units),
            });
        }
        off += len;
    }
    Ok(out)
}

/// 一个文件在这段日志结束时的状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// 完整的文件引用号（含序列号），按 id 打开文件时用。
    pub frn: u64,
    /// 父目录的完整引用号。
    pub parent: u64,
    pub name: String,
    pub attributes: u32,
    /// 这段日志里新建过（或记录被复用给了另一个文件）：原有的节点作废。
    pub created: bool,
    /// 最后被删除了。
    pub deleted: bool,
}

impl Change {
    /// 记录号，即树节点的键。
    pub fn record(&self) -> u64 {
        self.frn & REF_MASK
    }

    pub fn is_dir(&self) -> bool {
        self.attributes & ATTRIBUTE_DIRECTORY != 0
    }
}

/// 把按时间排列的记录合并成每个文件最后的状态，按首次出现的顺序返回。
pub fn fold(records: &[UsnRecord]) -> Vec<Change> {
    let mut order: Vec<Change> = Vec::new();
    let mut at: HashMap<u64, usize> = HashMap::new();
    for r in records {
        let i = *at.entry(r.frn & REF_MASK).or_insert_with(|| {
            order.push(Change {
                frn: r.frn,
                parent: r.parent,
                name: r.name.clone(),
                attributes: r.attributes,
                created: false,
                deleted: false,
            });
            order.len() - 1
        });
        let c = &mut order[i];
        // 序列号变了：旧文件已删除，记录给了新文件。
        if c.frn != r.frn || r.reason & REASON_FILE_CREATE != 0 {
            c.created = true;
            c.deleted = false;
            c.frn = r.frn;
        }
        if r.reason & REASON_FILE_DELETE != 0 {
            c.deleted = true;
        }
        // 只带旧名称的那条记录不更新名称和位置。
        let old_name_only =
            r.reason & REASON_RENAME_OLD_NAME != 0 && r.reason & REASON_RENAME_NEW_NAME == 0;
        if !old_name_only {
            c.parent = r.parent;
            c.name.clone_from(&r.name);
            c.attributes = r.attributes;
        }
    }
    order
}

/// 记录号到树节点的索引。硬链接的一条记录对应多个节点，第一个挂在树上的作为主节点。
#[derive(Debug, Default)]
pub struct NodeIndex {
    primary: HashMap<u64, NodeId>,
    extra: HashMap<u64, Vec<NodeId>>,
}

impl NodeIndex {
    pub fn build(tree: &FileTree) -> Self {
        let mut index = Self::default();
        let mut detached = Vec::new();
        for id in 0..tree.len() as NodeId {
            let Some(key) = tree.key(id) else { continue };
            if id != tree.root() && tree.parent(id).is_none() {
                detached.push((key, id));
                continue;
            }
            index.add(key, id);
        }
        // 已摘除的节点排在最后，只在没有其他节点时作为主节点。
        for (key, id) in detached {
            index.add(key, id);
        }
        index
    }

    fn add(&mut self, key: u64, id: NodeId) {
        if let Some(&first) = self.primary.get(&key) {
            if first != id {
                self.extra.entry(key).or_default().push(id);
            }
        } else {
            self.primary.insert(key, id);
        }
    }

    pub fn get(&self, key: u64) -> Option<NodeId> {
        self.primary.get(&key).copied()
    }

    pub fn len(&self) -> usize {
        self.primary.len()
    }

    pub fn is_empty(&self) -> bool {
        self.primary.is_empty()
    }

    fn take(&mut self, key: u64) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self.primary.remove(&key).into_iter().collect();
        ids.extend(self.extra.remove(&key).unwrap_or_default());
        ids
    }
}

/// [`apply`] 做了什么。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ApplyStats {
    pub added: u64,
    pub removed: u64,
    pub updated: u64,
    /// 找不到父目录（例如在扫描范围之外）而跳过的变化。
    pub skipped: u64,
}

/// 把变化应用到树上。`size_of` 返回文件当前的（逻辑大小，占用空间），查不到时返回 `None`
/// （保留原值，新文件记为 0）。
pub fn apply(
    tree: &mut FileTree,
    index: &mut NodeIndex,
    changes: &[Change],
    size_of: impl FnMut(&Change) -> Option<(u64, u64)>,
) -> ApplyStats {
    let mut a = Applier {
        tree,
        index,
        by_record: changes
            .iter()
            .enumerate()
            .map(|(i, c)| (c.record(), i))
            .collect(),
        changes,
        size_of,
        state: vec![State::Pending; changes.len()],
        stats: ApplyStats::default(),
    };

    // 先删：删除的文件，以及记录被复用的旧节点。
    for c in changes {
        if c.deleted || c.created {
            for id in a.index.take(c.record()) {
                if a.tree.detach(id) {
                    a.stats.removed += 1;
                }
            }
        }
    }
    // 再按需加入或更新；父目录也在变化里时先处理父目录。
    for i in 0..changes.len() {
        a.ensure(i);
    }
    a.stats
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Pending,
    Visiting,
    Done,
}

struct Applier<'a, F> {
    tree: &'a mut FileTree,
    index: &'a mut NodeIndex,
    changes: &'a [Change],
    by_record: HashMap<u64, usize>,
    size_of: F,
    state: Vec<State>,
    stats: ApplyStats,
}

impl<F: FnMut(&Change) -> Option<(u64, u64)>> Applier<'_, F> {
    /// 确保第 `i` 个变化已应用，返回它对应的节点。
    fn ensure(&mut self, i: usize) -> Option<NodeId> {
        let c = &self.changes[i];
        let record = c.record();
        match self.state[i] {
            State::Done => return self.index.get(record),
            // 父子关系成环（日志不完整时可能出现），放弃这一项。
            State::Visiting => return None,
            State::Pending => {}
        }
        if c.deleted || record == ROOT_RECORD {
            self.state[i] = State::Done;
            return None;
        }
        self.state[i] = State::Visiting;
        let result = self.place(i);
        self.state[i] = State::Done;
        if result.is_none() {
            self.stats.skipped += 1;
        }
        result
    }

    fn place(&mut self, i: usize) -> Option<NodeId> {
        let c = &self.changes[i];
        let record = c.record();
        let parent_record = c.parent & REF_MASK;
        let parent = match self.by_record.get(&parent_record) {
            Some(&p) if parent_record != ROOT_RECORD && !self.changes[p].deleted => self.ensure(p),
            _ => self.index.get(parent_record),
        }?;
        if self.tree.kind(parent) != NodeKind::Dir || !self.tree.is_attached(parent) {
            return None;
        }

        let kind = if c.is_dir() {
            NodeKind::Dir
        } else {
            NodeKind::File
        };
        let existing = self
            .index
            .get(record)
            .filter(|&id| self.tree.kind(id) == kind);
        let id = match existing {
            Some(id) => {
                if !self.tree.move_to(id, parent) {
                    return None;
                }
                self.tree.rename(id, &c.name);
                self.stats.updated += 1;
                id
            }
            None => {
                for old in self.index.take(record) {
                    self.tree.detach(old);
                }
                let id = self.tree.insert(parent, &c.name, kind, 0, 0);
                self.tree.set_key(id, record);
                self.index.add(record, id);
                self.stats.added += 1;
                id
            }
        };
        if kind == NodeKind::File
            && let Some((size, allocated)) = (self.size_of)(c)
        {
            self.tree.set_file_size(id, size, allocated);
        }
        Some(id)
    }
}

#[cfg(windows)]
pub use win::{FileSizer, create_journal, query_journal, read_journal};

#[cfg(windows)]
mod win {
    use std::fs::File;
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_COMPRESSION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_DESCRIPTOR,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_STANDARD_INFO, FileCompressionInfo, FileIdType, FileStandardInfo, GetDiskFreeSpaceW,
        GetFileInformationByHandleEx, OpenFileById,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{
        FSCTL_CREATE_USN_JOURNAL, FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL,
    };

    use super::{
        ATTRIBUTE_COMPRESSED, ATTRIBUTE_SPARSE, Change, JournalPos, UsnRecord, parse_records,
    };
    use crate::{Error, Result, le_u64};

    /// 一次读取日志的缓冲区大小。
    const READ_BUFFER: usize = 64 * 1024;

    fn journal_error(e: &io::Error) -> Error {
        Error::Journal(e.to_string())
    }

    fn ioctl(volume: &File, code: u32, input: &[u8], output: &mut [u8]) -> io::Result<usize> {
        let mut returned = 0u32;
        // SAFETY: 句柄在 `volume` 存活期间有效；输入输出缓冲区的指针和长度来自有效的切片；
        // 同步调用，不使用 OVERLAPPED。
        let ok = unsafe {
            DeviceIoControl(
                volume.as_raw_handle(),
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                output.as_mut_ptr().cast(),
                output.len() as u32,
                &mut returned,
                null_mut(),
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(returned as usize)
        }
    }

    /// 日志的当前状态：id、最早仍保留的 USN、下一条记录的 USN。
    pub fn query_journal(volume: &File) -> Result<(JournalPos, i64)> {
        // USN_JOURNAL_DATA_V0：UsnJournalID u64、FirstUsn i64、NextUsn i64……
        let mut out = [0u8; 80];
        ioctl(volume, FSCTL_QUERY_USN_JOURNAL, &[], &mut out).map_err(|e| journal_error(&e))?;
        let pos = JournalPos {
            id: le_u64(&out, 0)?,
            next_usn: le_u64(&out, 16)? as i64,
        };
        Ok((pos, le_u64(&out, 8)? as i64))
    }

    /// 在卷上启用 USN 日志（已启用时只调整大小）。这会修改卷的设置，只在用户同意后调用。
    pub fn create_journal(volume: &File, max_size: u64) -> Result<()> {
        // CREATE_USN_JOURNAL_DATA：MaximumSize u64、AllocationDelta u64。
        let mut input = [0u8; 16];
        input[0..8].copy_from_slice(&max_size.to_le_bytes());
        input[8..16].copy_from_slice(&(max_size / 8).to_le_bytes());
        ioctl(volume, FSCTL_CREATE_USN_JOURNAL, &input, &mut [])
            .map(|_| ())
            .map_err(|e| journal_error(&e))
    }

    /// 读出 `since` 之后的全部记录，返回记录和读到的位置（下次从这里继续）。
    pub fn read_journal(volume: &File, since: JournalPos) -> Result<(Vec<UsnRecord>, JournalPos)> {
        let (now, first_usn) = query_journal(volume)?;
        if now.id != since.id {
            return Err(Error::Journal("日志已被重建".into()));
        }
        if since.next_usn < first_usn {
            return Err(Error::Journal("变化太多，较早的日志已被覆盖".into()));
        }

        let mut records = Vec::new();
        let mut start = since.next_usn;
        let mut buf = vec![0u8; READ_BUFFER];
        // 只读到开始刷新时的末尾，期间仍在发生的变化留给下一次。
        while start < now.next_usn {
            // READ_USN_JOURNAL_DATA_V1：StartUsn i64、ReasonMask u32、ReturnOnlyOnClose u32、
            // Timeout u64、BytesToWaitFor u64、UsnJournalID u64、Min/MaxMajorVersion u16，补齐到 48 字节。
            let mut input = [0u8; 48];
            input[0..8].copy_from_slice(&start.to_le_bytes());
            input[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
            input[32..40].copy_from_slice(&since.id.to_le_bytes());
            input[40..42].copy_from_slice(&2u16.to_le_bytes());
            input[42..44].copy_from_slice(&2u16.to_le_bytes());
            let n = ioctl(volume, FSCTL_READ_USN_JOURNAL, &input, &mut buf)
                .map_err(|e| journal_error(&e))?;
            if n < 8 {
                break;
            }
            let next = le_u64(&buf, 0)? as i64;
            records.extend(parse_records(&buf[8..n])?);
            if next <= start {
                break;
            }
            start = next;
        }
        Ok((
            records,
            JournalPos {
                id: since.id,
                next_usn: start,
            },
        ))
    }

    /// 按文件 id 查询文件当前的大小。
    pub struct FileSizer {
        /// 卷上任意一个打开的句柄，`OpenFileById` 用它确定是哪个卷。
        hint: File,
        cluster: u64,
    }

    impl FileSizer {
        /// 打开 `letter` 盘。
        pub fn open(letter: char) -> Result<Self> {
            let root = format!(r"{}:\", letter.to_ascii_uppercase());
            let hint = std::fs::OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(&root)?;
            let wide: Vec<u16> = root.encode_utf16().chain([0]).collect();
            let (mut spc, mut bps, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
            // SAFETY: `wide` 以 0 结尾；四个输出参数都指向有效的 u32。
            let ok = unsafe {
                GetDiskFreeSpaceW(wide.as_ptr(), &mut spc, &mut bps, &mut free, &mut total)
            };
            if ok == 0 {
                return Err(io::Error::last_os_error().into());
            }
            Ok(Self {
                hint,
                cluster: u64::from(spc) * u64::from(bps),
            })
        }

        /// 文件的（逻辑大小，占用空间）。文件已不存在或打不开时返回 `None`。
        ///
        /// 与 MFT 扫描的口径一致：常驻在 MFT 记录里的小文件占用记 0；压缩和稀疏文件按实际
        /// 分配的空间计。
        pub fn size(&self, change: &Change) -> Option<(u64, u64)> {
            let mut id = FILE_ID_DESCRIPTOR {
                dwSize: size_of::<FILE_ID_DESCRIPTOR>() as u32,
                Type: FileIdType,
                ..Default::default()
            };
            id.Anonymous.FileId = change.frn as i64;
            // SAFETY: `hint` 是有效句柄；`id` 是按文档填好的 FILE_ID_DESCRIPTOR。
            let raw = unsafe {
                OpenFileById(
                    self.hint.as_raw_handle(),
                    &id,
                    FILE_READ_ATTRIBUTES,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    std::ptr::null(),
                    FILE_FLAG_BACKUP_SEMANTICS,
                )
            };
            if raw == INVALID_HANDLE_VALUE || raw.is_null() {
                return None;
            }
            // SAFETY: `raw` 是刚打开的有效句柄，交给 OwnedHandle 负责关闭。
            let file = unsafe { OwnedHandle::from_raw_handle(raw) };

            let mut info = FILE_STANDARD_INFO::default();
            // SAFETY: 缓冲区是一个 FILE_STANDARD_INFO，大小如实传入。
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    file.as_raw_handle(),
                    FileStandardInfo,
                    (&raw mut info).cast(),
                    size_of::<FILE_STANDARD_INFO>() as u32,
                )
            };
            if ok == 0 {
                return None;
            }
            let size = info.EndOfFile.max(0) as u64;
            let mut allocated = info.AllocationSize.max(0) as u64;
            if change.attributes & (ATTRIBUTE_COMPRESSED | ATTRIBUTE_SPARSE) != 0 {
                let mut c = FILE_COMPRESSION_INFO::default();
                // SAFETY: 同上，缓冲区是一个 FILE_COMPRESSION_INFO。
                let ok = unsafe {
                    GetFileInformationByHandleEx(
                        file.as_raw_handle(),
                        FileCompressionInfo,
                        (&raw mut c).cast(),
                        size_of::<FILE_COMPRESSION_INFO>() as u32,
                    )
                };
                if ok != 0 {
                    allocated = c.CompressedFileSize.max(0) as u64;
                }
            }
            if self.cluster > 0 && !allocated.is_multiple_of(self.cluster) {
                // 不是整簇，说明内容常驻在 MFT 记录里，不另占簇。
                allocated = 0;
            }
            Some((size, allocated))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEQ: u64 = 1 << 48;

    fn rec(frn: u64, parent: u64, reason: u32, name: &str, dir: bool) -> UsnRecord {
        UsnRecord {
            frn,
            parent,
            usn: 0,
            reason,
            attributes: if dir { ATTRIBUTE_DIRECTORY } else { 0x20 },
            name: name.into(),
        }
    }

    /// 根（记录 5）下有 docs（30）和 a.txt（40），docs 下有 b.txt（41）。
    fn tree() -> (FileTree, NodeIndex) {
        let mut t = FileTree::new(r"T:\");
        t.set_key(0, 5);
        let docs = t.add_dir(0, "docs");
        t.set_key(docs, 30);
        let a = t.add_file(0, "a.txt", 100, 4_096);
        t.set_key(a, 40);
        let b = t.add_file(docs, "b.txt", 200, 4_096);
        t.set_key(b, 41);
        t.finalize();
        let index = NodeIndex::build(&t);
        (t, index)
    }

    fn listing(t: &FileTree) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = (0..t.len() as NodeId)
            .filter(|&i| t.is_attached(i))
            .map(|i| (t.path(i), t.size(i)))
            .collect();
        v.sort();
        v
    }

    fn run(t: &mut FileTree, index: &mut NodeIndex, records: &[UsnRecord]) -> ApplyStats {
        let changes = fold(records);
        apply(t, index, &changes, |c| {
            Some(match c.name.as_str() {
                "new.bin" => (5_000, 8_192),
                "b.txt" => (250, 4_096),
                _ => (1, 0),
            })
        })
    }

    #[test]
    fn parses_v2_records() {
        let name: Vec<u8> = "报告.docx"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let len = (RECORD_V2_HEADER + name.len()).next_multiple_of(8);
        let mut b = vec![0u8; len];
        b[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        b[4..6].copy_from_slice(&2u16.to_le_bytes());
        b[8..16].copy_from_slice(&(SEQ | 77).to_le_bytes());
        b[16..24].copy_from_slice(&((SEQ * 5) | 5).to_le_bytes());
        b[24..32].copy_from_slice(&1234i64.to_le_bytes());
        b[40..44].copy_from_slice(&(REASON_FILE_CREATE | 0x8000_0000).to_le_bytes());
        b[52..56].copy_from_slice(&0x20u32.to_le_bytes());
        b[56..58].copy_from_slice(&(name.len() as u16).to_le_bytes());
        b[58..60].copy_from_slice(&(RECORD_V2_HEADER as u16).to_le_bytes());
        b[RECORD_V2_HEADER..RECORD_V2_HEADER + name.len()].copy_from_slice(&name);
        let mut two = b.clone();
        two.extend_from_slice(&b);

        let recs = parse_records(&two).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].name, "报告.docx");
        assert_eq!(recs[0].frn & REF_MASK, 77);
        assert_eq!(recs[0].parent & REF_MASK, 5);
        assert_eq!(recs[0].usn, 1234);

        assert!(parse_records(&two[..len + 10]).is_err(), "truncated");
    }

    #[test]
    fn fold_keeps_final_state() {
        let records = [
            rec(SEQ | 50, 5, REASON_FILE_CREATE, "tmp", false),
            rec(SEQ | 50, 5, REASON_RENAME_OLD_NAME, "tmp", false),
            rec(SEQ | 50, 30, REASON_RENAME_NEW_NAME, "final.txt", false),
            rec(SEQ | 51, 5, REASON_FILE_CREATE, "gone", false),
            rec(SEQ | 51, 5, REASON_FILE_DELETE, "gone", false),
            // 记录 41 被复用给新文件：日志里总有一条带新序列号的创建记录。
            rec((2 * SEQ) | 41, 5, REASON_FILE_CREATE, "reused.bin", false),
            // 同一段日志里先删后建、序列号变化，也算新建。
            rec(SEQ | 52, 5, REASON_FILE_DELETE, "old", false),
            rec((2 * SEQ) | 52, 5, 0x2, "new", false),
        ];
        let changes = fold(&records);
        assert_eq!(changes.len(), 4);
        assert_eq!(
            (
                changes[0].name.as_str(),
                changes[0].parent,
                changes[0].created
            ),
            ("final.txt", 30, true)
        );
        assert!(changes[1].created && changes[1].deleted);
        assert!(changes[2].created && !changes[2].deleted);
        assert_eq!(changes[2].frn, (2 * SEQ) | 41);
        assert!(changes[3].created && !changes[3].deleted);
        assert_eq!(changes[3].name, "new");
    }

    #[test]
    fn applies_creates_moves_and_deletes() {
        let (mut t, mut index) = tree();
        let records = [
            // 新文件出现在新目录之前（父目录也在这批变化里）。
            rec(SEQ | 61, SEQ | 60, REASON_FILE_CREATE, "new.bin", false),
            rec(SEQ | 60, SEQ | 5, REASON_FILE_CREATE, "新目录", true),
            // docs 改名并移进新目录。
            rec(SEQ | 30, 5, REASON_RENAME_OLD_NAME, "docs", true),
            rec(SEQ | 30, SEQ | 60, REASON_RENAME_NEW_NAME, "文档", true),
            // b.txt 内容变化。
            rec(SEQ | 41, SEQ | 30, 0x2, "b.txt", false),
            // a.txt 删除。
            rec(SEQ | 40, 5, REASON_FILE_DELETE, "a.txt", false),
            // 父目录不在树里。
            rec(SEQ | 70, SEQ | 999, REASON_FILE_CREATE, "outside", false),
        ];
        let stats = run(&mut t, &mut index, &records);
        assert_eq!(
            listing(&t),
            [
                (r"T:\".to_string(), 5_250),
                (r"T:\新目录".to_string(), 5_250),
                (r"T:\新目录\new.bin".to_string(), 5_000),
                (r"T:\新目录\文档".to_string(), 250),
                (r"T:\新目录\文档\b.txt".to_string(), 250),
            ]
        );
        assert_eq!(t.file_count(0), 2);
        assert_eq!(
            stats,
            ApplyStats {
                added: 2,
                removed: 1,
                updated: 2,
                skipped: 1
            }
        );
    }

    #[test]
    fn deleting_a_folder_and_reusing_records() {
        let (mut t, mut index) = tree();
        let records = [
            rec(SEQ | 41, SEQ | 30, REASON_FILE_DELETE, "b.txt", false),
            rec(SEQ | 30, 5, REASON_FILE_DELETE, "docs", true),
            // 记录 40 复用给一个新目录。
            rec((2 * SEQ) | 40, 5, REASON_FILE_CREATE, "again", true),
        ];
        run(&mut t, &mut index, &records);
        assert_eq!(
            listing(&t),
            [(r"T:\".to_string(), 0), (r"T:\again".to_string(), 0)]
        );
        assert_eq!(t.file_count(0), 0);

        // 再刷新一次：已删除的记录再次出现也不会出错。
        let again = [rec(SEQ | 30, 5, REASON_FILE_DELETE, "docs", true)];
        let stats = run(&mut t, &mut index, &again);
        assert_eq!(stats.removed, 0);
    }

    #[test]
    fn moving_a_file_out_of_a_folder_that_is_then_deleted() {
        let (mut t, mut index) = tree();
        let records = [
            rec(SEQ | 41, SEQ | 30, REASON_RENAME_OLD_NAME, "b.txt", false),
            rec(SEQ | 41, SEQ | 5, REASON_RENAME_NEW_NAME, "b.txt", false),
            rec(SEQ | 30, 5, REASON_FILE_DELETE, "docs", true),
        ];
        run(&mut t, &mut index, &records);
        assert_eq!(
            listing(&t),
            [
                (r"T:\".to_string(), 350),
                (r"T:\a.txt".to_string(), 100),
                (r"T:\b.txt".to_string(), 250),
            ]
        );
    }
}
