//! 重复文件。
//!
//! 三步判定，每一步只处理上一步还没排除的文件：
//!
//! 1. 按大小分组（直接用扫描结果，不读磁盘）；大小独一无二的文件不可能重复。
//! 2. 打开文件，核对大小，取文件 id，计算头尾各 4 KB 的部分哈希；按（大小，部分哈希）再分组。
//!    文件 id 相同的是同一个文件的多个名称（硬链接），只算一个，不当作重复。
//! 3. 对仍然成组的文件计算全文哈希（xxHash3-128），按（大小，全文哈希）分组得到结果。
//!
//! 哈希结果存在 [`HashCache`] 里，按文件 id 记录大小和修改时间；再次查重时没有变化的
//! 文件不再读取。

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::UNIX_EPOCH;

use dl_tree::{FileTree, NodeId, NodeKind};
use xxhash_rust::xxh3::{Xxh3, xxh3_128};

/// 部分哈希读取文件头尾各这么多字节。不超过两倍于此的文件直接整个哈希。
const EDGE: u64 = 4096;
/// 计算全文哈希时每次读取的字节数。
const CHUNK: usize = 1 << 20;

/// 一个待查的文件。
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: NodeId,
    pub path: PathBuf,
    pub size: u64,
}

/// `root` 子树中可能重复的文件：不小于 `min_size`、且至少还有一个文件大小相同。
/// 按大小从大到小排列。
pub fn candidates(tree: &FileTree, root: NodeId, min_size: u64) -> Vec<Candidate> {
    let min_size = min_size.max(1);
    let mut by_size: HashMap<u64, Vec<NodeId>> = HashMap::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        match tree.kind(n) {
            NodeKind::Dir => stack.extend(tree.children(n)),
            NodeKind::File => {
                let size = tree.size(n);
                if size >= min_size {
                    by_size.entry(size).or_default().push(n);
                }
            }
        }
    }
    let mut out: Vec<Candidate> = by_size
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .flat_map(|(size, ids)| {
            ids.into_iter().map(move |id| Candidate {
                id,
                path: PathBuf::new(),
                size,
            })
        })
        .collect();
    for c in &mut out {
        c.path = PathBuf::from(tree.path(c.id));
    }
    out.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    out
}

/// 文件在卷上的身份：卷序列号加文件 id。同一文件的不同名称（硬链接）身份相同。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileKey {
    pub volume: u64,
    pub index: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CacheEntry {
    size: u64,
    /// 修改时间，Unix 纪元以来的纳秒。
    modified: u64,
    partial: u128,
    full: Option<u128>,
}

/// 哈希缓存，可在多次查重之间保存到磁盘。
#[derive(Debug, Default)]
pub struct HashCache {
    entries: Mutex<HashMap<FileKey, CacheEntry>>,
}

const CACHE_MAGIC: &[u8; 8] = b"DLHASH01";
/// 每条记录：卷 u64、id u64、大小 u64、修改时间 u64、部分哈希 u128、有无全文哈希 u8、全文哈希 u128。
const CACHE_ENTRY_BYTES: usize = 8 * 4 + 16 + 1 + 16;

impl HashCache {
    pub fn len(&self) -> usize {
        self.entries.lock().expect("hash cache poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, key: FileKey, size: u64, modified: u64) -> Option<CacheEntry> {
        let entries = self.entries.lock().expect("hash cache poisoned");
        entries
            .get(&key)
            .copied()
            .filter(|e| e.size == size && e.modified == modified)
    }

    fn put(&self, key: FileKey, entry: CacheEntry) {
        self.entries
            .lock()
            .expect("hash cache poisoned")
            .insert(key, entry);
    }

    /// 从文件读入。文件不存在或已损坏时得到空缓存。
    pub fn load(path: &Path) -> Self {
        let cache = Self::default();
        let Ok(bytes) = std::fs::read(path) else {
            return cache;
        };
        if bytes.len() < 16 || &bytes[..8] != CACHE_MAGIC {
            return cache;
        }
        let count = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
        let body = &bytes[16..];
        if body.len() != count.saturating_mul(CACHE_ENTRY_BYTES) {
            return cache;
        }
        let u64_at = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let u128_at = |b: &[u8], o: usize| u128::from_le_bytes(b[o..o + 16].try_into().unwrap());
        let mut entries = cache.entries.lock().expect("hash cache poisoned");
        for e in body.as_chunks::<CACHE_ENTRY_BYTES>().0 {
            let key = FileKey {
                volume: u64_at(e, 0),
                index: u64_at(e, 8),
            };
            entries.insert(
                key,
                CacheEntry {
                    size: u64_at(e, 16),
                    modified: u64_at(e, 24),
                    partial: u128_at(e, 32),
                    full: (e[48] == 1).then(|| u128_at(e, 49)),
                },
            );
        }
        drop(entries);
        cache
    }

    /// 写入文件：先写临时文件再改名。
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let entries = self.entries.lock().expect("hash cache poisoned");
        let mut buf = Vec::with_capacity(16 + entries.len() * CACHE_ENTRY_BYTES);
        buf.extend_from_slice(CACHE_MAGIC);
        buf.extend_from_slice(&(entries.len() as u64).to_le_bytes());
        for (k, e) in entries.iter() {
            for v in [k.volume, k.index, e.size, e.modified] {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf.extend_from_slice(&e.partial.to_le_bytes());
            buf.push(u8::from(e.full.is_some()));
            buf.extend_from_slice(&e.full.unwrap_or(0).to_le_bytes());
        }
        drop(entries);
        let tmp = path.with_extension("tmp");
        let mut file = File::create(&tmp)?;
        file.write_all(&buf)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    }
}

/// 查重进行到哪一步。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// 第 2 步：核对文件并计算部分哈希。
    Checking,
    /// 第 3 步：计算全文哈希。
    Hashing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DupFile {
    pub id: NodeId,
    pub path: PathBuf,
    /// 修改时间，Unix 纪元以来的毫秒。
    pub modified_ms: u64,
    /// 这个文件一共有几个名称（硬链接数）。大于 1 时删掉这个名称并不释放空间。
    pub links: u32,
    /// 所在卷的序列号。只有同一个卷上的文件才能互相做硬链接。
    pub volume: u64,
}

/// 一组内容完全相同的文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DupGroup {
    pub size: u64,
    pub hash: u128,
    pub files: Vec<DupFile>,
}

impl DupGroup {
    /// 只保留一份时能释放的空间。
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64 - 1)
    }
}

#[derive(Debug, Default)]
pub struct DupReport {
    /// 按可释放空间从大到小。
    pub groups: Vec<DupGroup>,
    /// 打不开、读不了或大小已经变了的文件数。
    pub skipped: u64,
    /// 因为是同一个文件的另一个名称（硬链接）而不算重复的数量。
    pub same_file: u64,
    /// 实际从磁盘读取的字节数（命中缓存的不算）。
    pub bytes_read: u64,
}

/// 第 2 步之后一个文件的信息。
struct Checked {
    candidate: usize,
    key: FileKey,
    links: u32,
    modified: u64,
    partial: u128,
    full: Option<u128>,
}

/// 默认线程数：哈希以等待磁盘为主，SSD 上多几个并发读取更快。
pub fn default_threads() -> usize {
    thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(2, 8)
}

/// 在 `candidates` 中找出内容相同的文件组。`progress` 返回 `false` 即取消，此时返回
/// [`io::ErrorKind::Interrupted`]；它可能从任意工作线程调用。
pub fn find_duplicates(
    candidates: &[Candidate],
    cache: &HashCache,
    threads: usize,
    progress: &(dyn Fn(&Progress) -> bool + Sync),
) -> io::Result<DupReport> {
    let cancelled = AtomicBool::new(false);
    let skipped = AtomicU64::new(0);
    let bytes_read = AtomicU64::new(0);
    let report = |phase, done: &AtomicU64, total, bytes: &AtomicU64, bytes_total| {
        let p = Progress {
            phase,
            files_done: done.load(Ordering::Relaxed),
            files_total: total,
            bytes_done: bytes.load(Ordering::Relaxed),
            bytes_total,
        };
        if !progress(&p) {
            cancelled.store(true, Ordering::Relaxed);
        }
    };

    // 第 2 步：核对、取 id、部分哈希。
    let done = AtomicU64::new(0);
    let total_bytes: u64 = candidates.iter().map(|c| c.size).sum();
    let bytes_done = AtomicU64::new(0);
    let checked = par_map(candidates.len(), threads, &cancelled, |i| {
        let c = &candidates[i];
        let result = check(c, cache, &bytes_read);
        done.fetch_add(1, Ordering::Relaxed);
        bytes_done.fetch_add(c.size, Ordering::Relaxed);
        report(
            Phase::Checking,
            &done,
            candidates.len() as u64,
            &bytes_done,
            total_bytes,
        );
        match result {
            Some((key, links, modified, partial, full)) => Some(Checked {
                candidate: i,
                key,
                links,
                modified,
                partial,
                full,
            }),
            None => {
                skipped.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    });
    if cancelled.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "查重已取消"));
    }

    // 同一个文件的多个名称只留第一个。
    let mut same_file = 0;
    let mut seen: HashSet<FileKey> = HashSet::new();
    let mut unique: Vec<Checked> = Vec::new();
    for c in checked.into_iter().flatten() {
        if !seen.insert(c.key) {
            same_file += 1;
        } else {
            unique.push(c);
        }
    }

    // 按（大小，部分哈希）分组，只留成组的。
    let size_of = |c: &Checked| candidates[c.candidate].size;
    let mut by_partial: HashMap<(u64, u128), Vec<Checked>> = HashMap::new();
    for c in unique {
        by_partial
            .entry((size_of(&c), c.partial))
            .or_default()
            .push(c);
    }
    let mut pending: Vec<Checked> = by_partial
        .into_values()
        .filter(|g| g.len() > 1)
        .flatten()
        .collect();

    // 第 3 步：没有缓存的全文哈希现算。
    let need: Vec<usize> = (0..pending.len())
        .filter(|&i| pending[i].full.is_none())
        .collect();
    let done = AtomicU64::new(0);
    let hash_total: u64 = need.iter().map(|&i| size_of(&pending[i])).sum();
    let hashed = AtomicU64::new(0);
    let fulls = par_map(need.len(), threads, &cancelled, |j| {
        let c = &pending[need[j]];
        let cand = &candidates[c.candidate];
        let result = full_hash(&cand.path, cand.size, &hashed, &bytes_read, &cancelled);
        done.fetch_add(1, Ordering::Relaxed);
        report(
            Phase::Hashing,
            &done,
            need.len() as u64,
            &hashed,
            hash_total,
        );
        result
    });
    if cancelled.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "查重已取消"));
    }
    for (j, full) in fulls.into_iter().enumerate() {
        let c = &mut pending[need[j]];
        match full {
            Some(h) => {
                c.full = Some(h);
                cache.put(
                    c.key,
                    CacheEntry {
                        size: candidates[c.candidate].size,
                        modified: c.modified,
                        partial: c.partial,
                        full: Some(h),
                    },
                );
            }
            None => {
                skipped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    // 按（大小，全文哈希）分组。
    let mut by_full: HashMap<(u64, u128), Vec<DupFile>> = HashMap::new();
    for c in &pending {
        let Some(full) = c.full else { continue };
        let cand = &candidates[c.candidate];
        by_full.entry((cand.size, full)).or_default().push(DupFile {
            id: cand.id,
            path: cand.path.clone(),
            modified_ms: c.modified / 1_000_000,
            links: c.links,
            volume: c.key.volume,
        });
    }
    let mut groups: Vec<DupGroup> = by_full
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|((size, hash), mut files)| {
            files.sort_by(|a, b| a.path.cmp(&b.path));
            DupGroup { size, hash, files }
        })
        .collect();
    groups.sort_by(|a, b| {
        b.wasted()
            .cmp(&a.wasted())
            .then_with(|| a.files[0].path.cmp(&b.files[0].path))
    });

    Ok(DupReport {
        groups,
        skipped: skipped.into_inner(),
        same_file,
        bytes_read: bytes_read.into_inner(),
    })
}

/// 第 2 步：打开文件，核对大小，返回（身份，名称数，修改时间，部分哈希，全文哈希）。
/// 小文件的部分哈希就是全文哈希。打不开或大小已变时返回 `None`。
fn check(
    c: &Candidate,
    cache: &HashCache,
    bytes_read: &AtomicU64,
) -> Option<(FileKey, u32, u64, u128, Option<u128>)> {
    let mut file = File::open(&c.path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() != c.size {
        return None;
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64);
    let (key, links) = sys::identity(&file, &meta)?;
    if let Some(e) = cache.get(key, c.size, modified) {
        return Some((key, links, modified, e.partial, e.full));
    }

    let (partial, full) = if c.size <= 2 * EDGE {
        let mut buf = Vec::with_capacity(c.size as usize);
        file.read_to_end(&mut buf).ok()?;
        if buf.len() as u64 != c.size {
            return None;
        }
        let h = xxh3_128(&buf);
        (h, Some(h))
    } else {
        let mut buf = vec![0u8; 2 * EDGE as usize];
        let (head, tail) = buf.split_at_mut(EDGE as usize);
        file.read_exact(head).ok()?;
        file.seek(SeekFrom::Start(c.size - EDGE)).ok()?;
        file.read_exact(tail).ok()?;
        (xxh3_128(&buf), None)
    };
    bytes_read.fetch_add(c.size.min(2 * EDGE), Ordering::Relaxed);
    cache.put(
        key,
        CacheEntry {
            size: c.size,
            modified,
            partial,
            full,
        },
    );
    Some((key, links, modified, partial, full))
}

/// 第 3 步：全文哈希。读到的字节数与预期不符时（文件在查重期间被改动）返回 `None`。
fn full_hash(
    path: &Path,
    size: u64,
    hashed: &AtomicU64,
    bytes_read: &AtomicU64,
    cancelled: &AtomicBool,
) -> Option<u128> {
    let mut file = sys::open_sequential(path).ok()?;
    let mut hasher = Xxh3::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        hasher.update(&buf[..n]);
        total += n as u64;
        hashed.fetch_add(n as u64, Ordering::Relaxed);
        bytes_read.fetch_add(n as u64, Ordering::Relaxed);
    }
    (total == size).then(|| hasher.digest128())
}

/// 用 `threads` 个线程对 `0..len` 并行求值，结果按下标排列；取消后剩余的项为 `None`。
fn par_map<R: Send>(
    len: usize,
    threads: usize,
    cancelled: &AtomicBool,
    f: impl Fn(usize) -> Option<R> + Sync,
) -> Vec<Option<R>> {
    let next = AtomicUsize::new(0);
    let mut out: Vec<Option<R>> = (0..len).map(|_| None).collect();
    let parts: Vec<Vec<(usize, R)>> = thread::scope(|s| {
        let workers: Vec<_> = (0..threads.clamp(1, len.max(1)))
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= len || cancelled.load(Ordering::Relaxed) {
                            break;
                        }
                        if let Some(r) = f(i) {
                            mine.push((i, r));
                        }
                    }
                    mine
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().expect("dupes worker panicked"))
            .collect()
    });
    for (i, r) in parts.into_iter().flatten() {
        out[i] = Some(r);
    }
    out
}

#[cfg(windows)]
mod sys {
    use std::fs::{File, Metadata, OpenOptions};
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_SEQUENTIAL_SCAN, GetFileInformationByHandle,
    };

    use super::FileKey;

    /// 文件的身份和名称数（硬链接数）。
    pub fn identity(file: &File, _: &Metadata) -> Option<(FileKey, u32)> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: 句柄在 `file` 存活期间有效；`info` 是有效的输出缓冲区。
        let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
        (ok != 0).then(|| {
            let key = FileKey {
                volume: u64::from(info.dwVolumeSerialNumber),
                index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            };
            (key, info.nNumberOfLinks)
        })
    }

    /// 告诉系统将顺序读取，预读更积极。
    pub fn open_sequential(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN)
            .open(path)
    }
}

#[cfg(not(windows))]
mod sys {
    use std::fs::{File, Metadata};
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    use super::FileKey;

    pub fn identity(_: &File, meta: &Metadata) -> Option<(FileKey, u32)> {
        let key = FileKey {
            volume: meta.dev(),
            index: meta.ino(),
        };
        Some((key, meta.nlink() as u32))
    }

    pub fn open_sequential(path: &Path) -> io::Result<File> {
        File::open(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 测试文件放在仓库的 target 下（D 盘）。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .unwrap();
            let dir = repo
                .join("target")
                .join("test-tmp")
                .join(format!("dupes-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, rel: &str, data: &[u8]) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, data).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn run(dir: &Path, cache: &HashCache, min_size: u64) -> DupReport {
        let tree = dl_walk::walk(dir).unwrap().tree;
        let cands = candidates(&tree, tree.root(), min_size);
        find_duplicates(&cands, cache, 4, &|_| true).unwrap()
    }

    fn names(g: &DupGroup) -> Vec<String> {
        g.files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    /// 测试集：真正的重复、只有中间不同、只有开头不同、小文件、空文件、硬链接。
    fn fixture(tag: &str) -> TempDir {
        let t = TempDir::new(tag);
        let big = pattern(200_000, 1);
        t.write("a/movie.mkv", &big);
        t.write("b/movie copy.mkv", &big);
        t.write("c/movie (2).mkv", &big);

        // 头尾各 4 KB 相同，只有中间一个字节不同：部分哈希相同，全文哈希不同。
        let mut middle = big.clone();
        middle[100_000] ^= 0xff;
        t.write("d/almost.mkv", &middle);

        // 大小相同但开头不同。
        let mut head = big.clone();
        head[0] ^= 0xff;
        t.write("d/head.mkv", &head);

        t.write("small/x.txt", b"same small content");
        t.write("small/y.txt", b"same small content");
        t.write("small/z.txt", b"diff small content");
        t.write("empty/1.txt", b"");
        t.write("empty/2.txt", b"");

        fs::hard_link(t.0.join("a/movie.mkv"), t.0.join("a/movie-link.mkv")).unwrap();
        t
    }

    #[test]
    fn finds_true_duplicates_only() {
        let t = fixture("find");
        let report = run(&t.0, &HashCache::default(), 1);

        assert_eq!(report.groups.len(), 2, "{:#?}", report.groups);
        let big = &report.groups[0];
        assert_eq!(big.size, 200_000);
        assert_eq!(big.files.len(), 3, "hard link is not a separate copy");
        assert_eq!(big.wasted(), 400_000);
        assert!(names(big).iter().all(|n| n.starts_with("movie")));
        assert_eq!(names(&report.groups[1]), ["x.txt", "y.txt"]);
        assert_eq!(report.same_file, 1);
        assert_eq!(report.skipped, 0);
        // 有硬链接的那份知道自己还有另一个名称。
        let linked: Vec<u32> = big.files.iter().map(|f| f.links).collect();
        assert_eq!(linked.iter().filter(|&&n| n == 2).count(), 1, "{linked:?}");
    }

    #[test]
    fn respects_min_size() {
        let t = fixture("min");
        let report = run(&t.0, &HashCache::default(), 1_000);
        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].size, 200_000);
    }

    #[test]
    fn cache_skips_unchanged_files_and_notices_changes() {
        let t = fixture("cache");
        let cache = HashCache::default();
        let first = run(&t.0, &cache, 1);
        assert!(first.bytes_read > 0);

        // 存盘再读回，第二次不需要读任何文件内容。
        let path = t.0.join("hashes.bin");
        cache.save(&path).unwrap();
        let reloaded = HashCache::load(&path);
        assert_eq!(reloaded.len(), cache.len());
        fs::remove_file(&path).unwrap();
        let second = run(&t.0, &reloaded, 1);
        assert_eq!(second.bytes_read, 0);
        assert_eq!(second.groups, first.groups);

        // 同样大小、不同内容（修改时间也变了）：必须重新计算，不能用旧哈希。
        std::thread::sleep(std::time::Duration::from_millis(20));
        t.write("small/y.txt", b"SAME small content");
        let third = run(&t.0, &reloaded, 1);
        assert_eq!(third.groups.len(), 1);
        assert_eq!(third.groups[0].size, 200_000);
    }

    #[test]
    fn can_cancel() {
        let t = fixture("cancel");
        let tree = dl_walk::walk(&t.0).unwrap().tree;
        let cands = candidates(&tree, tree.root(), 1);
        let err = find_duplicates(&cands, &HashCache::default(), 2, &|_| false).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn corrupt_cache_file_is_ignored() {
        let t = TempDir::new("badcache");
        let path = t.0.join("hashes.bin");
        fs::write(&path, b"DLHASH01\x05\0\0\0\0\0\0\0garbage").unwrap();
        assert!(HashCache::load(&path).is_empty());
        assert!(HashCache::load(&t.0.join("missing.bin")).is_empty());
    }
}
