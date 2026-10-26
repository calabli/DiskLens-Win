//! 读取整个 MFT 并组装文件树。
//!
//! 1. 读引导扇区，定位 `$MFT` 的第 0 条记录，取出它的 data runs；
//! 2. 按 runs 顺序成块读取 MFT，逐条解析记录，把名称（父子链接）和大小收集到按记录号
//!    索引的数组里。扩展记录的属性归到它的基记录上，所以不需要解析 `$ATTRIBUTE_LIST`；
//! 3. 从根目录（记录 5）广度优先展开，生成 [`FileTree`]。广度优先保证父节点先于子节点
//!    加入，满足 `FileTree` 的约定。
//!
//! 硬链接：同一文件有多个名称时每个名称都会出现在树里，但大小只算在第一次出现的位置。
//! 目录只展开一次，因此不会因循环而无限展开。

use std::collections::VecDeque;
use std::io;

use dl_tree::{FileTree, NodeId};

use crate::boot::BootSector;
use crate::record::{NAMESPACE_DOS, parse_record};
use crate::{Error, Result};

/// 根目录固定是第 5 条记录。
const ROOT_RECORD: usize = 5;
/// 每次从卷上读取的字节数。
const CHUNK: usize = 4 << 20;
/// 每解析这么多条记录回调一次进度。
const PROGRESS_EVERY: u64 = 100_000;

const IN_USE: u8 = 1;
const IS_DIR: u8 = 2;

/// 按偏移读取。卷设备要求偏移和长度都按扇区对齐，调用方负责满足。
pub trait ReadAt {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
}

impl ReadAt for [u8] {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| io::ErrorKind::UnexpectedEof)?;
        let src = self
            .get(start..start + buf.len())
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

impl ReadAt for std::fs::File {
    fn read_exact_at(&self, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
        while !buf.is_empty() {
            #[cfg(windows)]
            let n = std::os::windows::fs::FileExt::seek_read(self, buf, offset)?;
            #[cfg(unix)]
            let n = std::os::unix::fs::FileExt::read_at(self, buf, offset)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            buf = &mut buf[n..];
            offset += n as u64;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct MftScan {
    pub tree: FileTree,
    /// 正在使用的基记录数。
    pub records_in_use: u64,
    /// 解析失败而跳过的记录数（例如撕裂写入）。
    pub corrupt_records: u64,
    /// 正在使用、但从根目录走不到的记录数。
    pub orphans: u64,
    pub cluster_size: u64,
    pub record_size: u32,
}

/// 以管理员身份打开卷设备，例如 `open_volume('D')` 打开 `\\.\D:`。
#[cfg(windows)]
pub fn open_volume(letter: char) -> Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;

    if !letter.is_ascii_alphabetic() {
        return Err(Error::Unsupported("盘符无效"));
    }
    let path = format!(r"\\.\{}:", letter.to_ascii_uppercase());
    Ok(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(path)?)
}

/// 直读整个卷的 MFT，`progress` 以已处理的记录数被定期调用。
#[cfg(windows)]
pub fn scan_volume(letter: char, progress: impl FnMut(u64)) -> Result<MftScan> {
    let volume = open_volume(letter)?;
    let root = format!(r"{}:\", letter.to_ascii_uppercase());
    scan_reader(&volume, &root, progress)
}

pub fn scan_reader<R: ReadAt + ?Sized>(
    dev: &R,
    root_name: &str,
    mut progress: impl FnMut(u64),
) -> Result<MftScan> {
    let mut head = vec![0u8; 4096];
    dev.read_exact_at(&mut head, 0)?;
    let boot = BootSector::parse(&head)?;
    let rs = boot.record_size as usize;
    let cluster = boot.cluster_size;

    // 第 0 条记录就是 $MFT 自己，按扇区对齐读取。
    let sector = boot.bytes_per_sector as usize;
    let mut first = vec![0u8; rs.div_ceil(sector) * sector];
    dev.read_exact_at(&mut first, boot.mft_lcn * cluster)?;
    let mft = parse_record(&mut first[..rs], true)?.ok_or(Error::Corrupt("$MFT record"))?;
    if mft.has_attribute_list {
        return Err(Error::Unsupported(
            "$MFT 高度碎片化，data runs 分散在多条记录中",
        ));
    }
    let data = mft
        .streams
        .iter()
        .find(|s| !s.named && s.size.is_some())
        .ok_or(Error::Corrupt("$MFT has no data"))?;
    let runs = data
        .runs
        .as_deref()
        .ok_or(Error::Corrupt("$MFT data is resident"))?;
    let total = data.size.unwrap_or(0) / rs as u64;
    let total_usize = usize::try_from(total).map_err(|_| Error::Corrupt("MFT size"))?;

    let mut col = Collector::new(total_usize, cluster);
    let mut chunk = vec![0u8; CHUNK];
    let mut pending: Vec<u8> = Vec::with_capacity(CHUNK + rs);
    let mut next: u64 = 0;

    'runs: for run in runs {
        let lcn = run.lcn.ok_or(Error::Corrupt("sparse run in $MFT"))?;
        let run_bytes = run.clusters * cluster;
        let mut done = 0u64;
        while done < run_bytes {
            let n = (run_bytes - done).min(CHUNK as u64) as usize;
            dev.read_exact_at(&mut chunk[..n], lcn * cluster + done)?;
            done += n as u64;
            pending.extend_from_slice(&chunk[..n]);

            // 记录可能跨块甚至跨 run（簇比记录小时），凑满一条才处理。
            let mut pos = 0;
            while pos + rs <= pending.len() {
                if next >= total {
                    break 'runs;
                }
                col.ingest(next as usize, &mut pending[pos..pos + rs]);
                next += 1;
                pos += rs;
                if next.is_multiple_of(PROGRESS_EVERY) {
                    progress(next);
                }
            }
            pending.drain(..pos);
        }
    }
    progress(next);

    let records_in_use = col.in_use;
    let corrupt_records = col.corrupt;
    let (tree, orphans) = col.build(root_name);
    Ok(MftScan {
        tree,
        records_in_use,
        corrupt_records,
        orphans,
        cluster_size: cluster,
        record_size: boot.record_size,
    })
}

/// 一个名称：把记录 `child` 以 `name` 挂在目录 `parent` 下。
struct Link {
    parent: u32,
    child: u32,
    name_start: u32,
    name_len: u16,
}

/// 按记录号收集扫描结果，最后一次性组装成树。
struct Collector {
    flags: Vec<u8>,
    size: Vec<u64>,
    allocated: Vec<u64>,
    links: Vec<Link>,
    names: String,
    in_use: u64,
    corrupt: u64,
    cluster_size: u64,
}

impl Collector {
    fn new(records: usize, cluster_size: u64) -> Self {
        Self {
            cluster_size,
            flags: vec![0; records],
            size: vec![0; records],
            allocated: vec![0; records],
            links: Vec::with_capacity(records),
            names: String::with_capacity(records * 16),
            in_use: 0,
            corrupt: 0,
        }
    }

    fn ingest(&mut self, number: usize, buf: &mut [u8]) {
        let record = match parse_record(buf, false) {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(_) => {
                self.corrupt += 1;
                return;
            }
        };
        let target = if record.base == 0 {
            number
        } else {
            record.base as usize
        };
        if target >= self.flags.len() {
            self.corrupt += 1;
            return;
        }
        if record.base == 0 {
            self.flags[target] |= IN_USE | if record.is_dir { IS_DIR } else { 0 };
            self.in_use += 1;
        }
        for s in &record.streams {
            if let (false, Some(size)) = (s.named, s.size) {
                self.size[target] += size;
            }
            self.allocated[target] += s.clusters * self.cluster_size;
        }
        for n in record.names {
            if n.namespace == NAMESPACE_DOS || n.parent >= self.flags.len() as u64 {
                continue;
            }
            let Ok(name_len) = u16::try_from(n.name.len()) else {
                continue;
            };
            self.links.push(Link {
                parent: n.parent as u32,
                child: target as u32,
                name_start: self.names.len() as u32,
                name_len,
            });
            self.names.push_str(&n.name);
        }
    }

    fn build(self, root_name: &str) -> (FileTree, u64) {
        let n = self.flags.len();
        let mut tree = FileTree::new(root_name);
        if n <= ROOT_RECORD {
            return (tree, 0);
        }

        // 按父记录号对链接做计数排序，得到每个目录的子项区间。
        let mut start = vec![0u32; n + 1];
        for l in &self.links {
            start[l.parent as usize + 1] += 1;
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut order = vec![0u32; self.links.len()];
        for (i, l) in self.links.iter().enumerate() {
            let slot = &mut fill[l.parent as usize];
            order[*slot as usize] = i as u32;
            *slot += 1;
        }

        let mut placed = vec![false; n];
        let mut sized = vec![false; n];
        let mut queue: VecDeque<(usize, NodeId)> = VecDeque::new();
        placed[ROOT_RECORD] = true;
        queue.push_back((ROOT_RECORD, tree.root()));

        while let Some((dir, node)) = queue.pop_front() {
            for &li in &order[start[dir] as usize..start[dir + 1] as usize] {
                let l = &self.links[li as usize];
                let child = l.child as usize;
                if child == dir || self.flags[child] & IN_USE == 0 {
                    continue;
                }
                let s = l.name_start as usize;
                let name = &self.names[s..s + usize::from(l.name_len)];
                if self.flags[child] & IS_DIR != 0 {
                    if placed[child] {
                        continue;
                    }
                    placed[child] = true;
                    let id = tree.add_dir(node, name);
                    queue.push_back((child, id));
                } else {
                    placed[child] = true;
                    let (size, allocated) = if sized[child] {
                        (0, 0)
                    } else {
                        sized[child] = true;
                        (self.size[child], self.allocated[child])
                    };
                    tree.add_file(node, name, size, allocated);
                }
            }
        }
        tree.finalize();

        let orphans = (0..n)
            .filter(|&i| self.flags[i] & IN_USE != 0 && !placed[i])
            .count() as u64;
        (tree, orphans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::build::{
        NonResident, RECORD_SIZE, file_name, non_resident, record, resident,
    };
    use crate::record::{ATTR_DATA, NAMESPACE_DOS};

    const CLUSTER: usize = 4096;
    const IN_USE: u16 = 1;
    const DIR: u16 = 3;
    /// MFT 共 32 条记录，分成两个 run：簇 4..8 放 0–15 号，簇 20..24 放 16–31 号。
    const MFT_RECORDS: usize = 32;
    const RUN2_LCN: usize = 20;

    fn place(img: &mut [u8], number: usize, rec: &[u8]) {
        let off = if number < 16 {
            4 * CLUSTER + number * RECORD_SIZE
        } else {
            RUN2_LCN * CLUSTER + (number - 16) * RECORD_SIZE
        };
        img[off..off + RECORD_SIZE].copy_from_slice(rec);
    }

    fn image() -> Vec<u8> {
        let mut img = vec![0u8; 32 * CLUSTER];
        img[3..11].copy_from_slice(b"NTFS    ");
        img[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
        img[0x0D] = 8;
        img[0x28..0x30].copy_from_slice(&256u64.to_le_bytes());
        img[0x30..0x38].copy_from_slice(&4u64.to_le_bytes());
        img[0x40] = 0xF6;

        let mft_bytes = (MFT_RECORDS * RECORD_SIZE) as u64;
        // run1：4 个簇 @ 4；run2：4 个簇 @ 4+16。
        let mft_runs = [0x11, 0x04, 0x04, 0x11, 0x04, 0x10, 0x00];
        place(
            &mut img,
            0,
            &record(
                0,
                IN_USE,
                0,
                &[
                    file_name(5, "$MFT", 3),
                    non_resident(&NonResident {
                        name: None,
                        size: mft_bytes,
                        allocated: mft_bytes,
                        flags: 0,
                        total_allocated: None,
                        runs: &mft_runs,
                    }),
                ],
            ),
        );
        place(&mut img, 5, &record(5, DIR, 0, &[file_name(5, ".", 3)]));
        // $BadClus:$Bad 名义上占满整个卷，实际只有稀疏 run，不能算进占用空间。
        let volume = 200u64 << 30;
        place(
            &mut img,
            8,
            &record(
                8,
                IN_USE,
                0,
                &[
                    file_name(5, "$BadClus", 3),
                    non_resident(&NonResident {
                        name: Some("$Bad"),
                        size: volume,
                        allocated: volume,
                        flags: 0,
                        total_allocated: None,
                        runs: &[0x04, 0x00, 0x00, 0x40, 0x03, 0x00],
                    }),
                ],
            ),
        );
        place(
            &mut img,
            16,
            &record(
                16,
                DIR,
                0,
                &[
                    file_name(5, "USERS~1", NAMESPACE_DOS),
                    file_name(5, "Users", 1),
                ],
            ),
        );
        place(
            &mut img,
            17,
            &record(
                17,
                IN_USE,
                0,
                &[
                    file_name(16, "a.txt", 3),
                    resident(ATTR_DATA, None, b"hello"),
                ],
            ),
        );
        place(
            &mut img,
            18,
            &record(
                18,
                IN_USE,
                0,
                &[
                    file_name(5, "big.bin", 3),
                    file_name(16, "big-link.bin", 0),
                    non_resident(&NonResident {
                        name: None,
                        size: 10_000,
                        allocated: 12_288,
                        flags: 0,
                        total_allocated: None,
                        runs: &[0x11, 0x03, 0x1C, 0x00],
                    }),
                ],
            ),
        );
        place(
            &mut img,
            19,
            &record(
                19,
                IN_USE,
                0,
                &[
                    file_name(16, "sparse.bin", 3),
                    non_resident(&NonResident {
                        name: None,
                        size: 1_000_000,
                        allocated: 1_048_576,
                        flags: 0x8000,
                        total_allocated: Some(4_096),
                        runs: &[0x11, 0x01, 0x1F, 0x01, 0xFF, 0x00],
                    }),
                ],
            ),
        );
        // 记录 17 的扩展记录：带一个非常驻的备用数据流。
        place(
            &mut img,
            20,
            &record(
                20,
                IN_USE,
                17,
                &[non_resident(&NonResident {
                    name: Some("ads"),
                    size: 3_000,
                    allocated: 4_096,
                    flags: 0,
                    total_allocated: None,
                    runs: &[0x11, 0x01, 0x1E, 0x00],
                })],
            ),
        );
        place(
            &mut img,
            21,
            &record(21, 0, 0, &[file_name(5, "deleted.txt", 3)]),
        );
        place(
            &mut img,
            22,
            &record(22, IN_USE, 0, &[file_name(30, "lost.txt", 3)]),
        );
        place(
            &mut img,
            23,
            &record(23, DIR, 0, &[file_name(16, "empty", 3)]),
        );
        let mut torn = record(24, IN_USE, 0, &[file_name(5, "torn.bin", 3)]);
        torn[510] ^= 0xFF;
        place(&mut img, 24, &torn);
        img
    }

    #[test]
    fn builds_tree_from_synthetic_volume() {
        let img = image();
        let mut calls = 0;
        let scan = scan_reader(img.as_slice(), r"T:\", |_| calls += 1).unwrap();
        let t = &scan.tree;
        let root = t.root();

        let top: Vec<&str> = t
            .children_by_size(root)
            .into_iter()
            .map(|c| t.name(c))
            .collect();
        assert_eq!(top, ["Users", "$MFT", "big.bin", "$BadClus"]);

        let users = t.child_by_name(root, "Users").unwrap();
        let mut in_users: Vec<&str> = t.children(users).map(|c| t.name(c)).collect();
        in_users.sort_unstable();
        assert_eq!(in_users, ["a.txt", "big-link.bin", "empty", "sparse.bin"]);

        // a.txt：常驻内容 5 字节；备用数据流不计入逻辑大小，只计入占用空间。
        let a = t.child_by_name(users, "a.txt").unwrap();
        assert_eq!((t.size(a), t.allocated(a)), (5, 4_096));
        // 稀疏文件：逻辑 1 MB，实际只占 4 KB。
        let sparse = t.child_by_name(users, "sparse.bin").unwrap();
        assert_eq!((t.size(sparse), t.allocated(sparse)), (1_000_000, 4_096));
        // 硬链接只算一次：广度优先先到根目录下的 big.bin。
        let big = t.child_by_name(root, "big.bin").unwrap();
        let link = t.child_by_name(users, "big-link.bin").unwrap();
        assert_eq!((t.size(big), t.size(link)), (10_000, 0));

        assert_eq!(t.size(root), 32_768 + 10_000 + 5 + 1_000_000);
        assert_eq!(t.allocated(root), 32_768 + 12_288 + 4_096 + 4_096);
        assert_eq!(t.file_count(root), 6);

        assert_eq!(scan.corrupt_records, 1);
        assert_eq!(scan.orphans, 1);
        // 0、5、8、16–19、22、23 号；扩展记录 20 不单独计数。
        assert_eq!(scan.records_in_use, 9);
        assert!(calls >= 1);
    }

    #[test]
    fn rejects_non_ntfs_image() {
        let mut img = image();
        img[3..11].copy_from_slice(b"FAT32   ");
        assert!(matches!(
            scan_reader(img.as_slice(), "X", |_| {}),
            Err(Error::NotNtfs)
        ));
    }

    #[test]
    fn survives_truncated_volume() {
        let img = image();
        let cut = &img[..RUN2_LCN * CLUSTER];
        assert!(matches!(scan_reader(cut, "X", |_| {}), Err(Error::Io(_))));
    }
}
