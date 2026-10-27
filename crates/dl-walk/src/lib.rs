//! 普通目录遍历。
//!
//! 逐个打开目录读取条目，适用于 MFT 直读不可用的场合（非 NTFS 卷、网络盘、未提权）。
//! 多个线程同时读不同的目录，最后按目录逐层组装成树，结果与单线程一致。
//!
//! 不跟随符号链接和连接点，避免重复统计和循环；它们本身作为空的项记在树上（与 MFT 直读一致），
//! 以便之后查找失效链接。其他重解析点（例如 OneDrive 按需下载的文件夹）照常进入。
//! 读不了的目录计入 [`WalkResult::skipped`] 后继续，不中断整次扫描。
//!
//! 占用空间：Windows 上用 `GetCompressedFileSizeW` 取实际占用（压缩、稀疏文件都准确），
//! 再按簇向上取整。很小的文件在 NTFS 上可能直接存在 MFT 记录里、不占簇，这里会多算
//! 一个簇；MFT 直读没有这个误差。

use std::collections::VecDeque;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread;

use dl_tree::{FileTree, NodeId};

/// 每处理这么多条目回调一次进度。
const PROGRESS_EVERY: u64 = 10_000;

#[derive(Debug)]
pub struct WalkResult {
    pub tree: FileTree,
    /// 因权限等原因无法读取的目录数。
    pub skipped: u64,
}

/// 遍历 `root`，返回已汇总大小的文件树。
pub fn walk(root: &Path) -> io::Result<WalkResult> {
    walk_with_progress(root, &|_| true)
}

/// 同 [`walk`]，并定期以已处理的条目数调用 `progress`（可能从任意工作线程调用）。
/// `progress` 返回 `false` 即取消，此时返回 [`io::ErrorKind::Interrupted`]。
pub fn walk_with_progress(
    root: &Path,
    progress: &(dyn Fn(u64) -> bool + Sync),
) -> io::Result<WalkResult> {
    walk_with_threads(root, default_threads(), progress)
}

/// 同 [`walk_with_progress`]，扫描过程中把每个顶层项目（根目录下的文件夹和文件）
/// 目前累计的大小记到 `live` 里，界面可以随时取快照，边扫边看。
pub fn walk_live(
    root: &Path,
    progress: &(dyn Fn(u64) -> bool + Sync),
    live: &Live,
) -> io::Result<WalkResult> {
    run(root, default_threads(), progress, Some(live))
}

/// 一个顶层项目目前累计的大小。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveItem {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub allocated: u64,
    pub files: u64,
}

/// 扫描过程中按顶层项目累计的大小，见 [`walk_live`]。
#[derive(Debug, Default)]
pub struct Live {
    items: Mutex<Vec<LiveItem>>,
}

impl Live {
    /// 当前各顶层项目的累计大小，顺序为发现的顺序。
    pub fn snapshot(&self) -> Vec<LiveItem> {
        self.items.lock().expect("live poisoned").clone()
    }

    fn add(&self, item: LiveItem) -> usize {
        let mut items = self.items.lock().expect("live poisoned");
        items.push(item);
        items.len() - 1
    }

    fn grow(&self, index: usize, size: u64, allocated: u64, files: u64) {
        let mut items = self.items.lock().expect("live poisoned");
        let item = &mut items[index];
        item.size += size;
        item.allocated += allocated;
        item.files += files;
    }
}

/// 默认线程数：目录遍历以等待 I/O 为主，线程可以比 CPU 核数多一些。
pub fn default_threads() -> usize {
    thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(2, 16)
}

/// 一个目录读出来的条目。子目录用编号指向它自己的 [`Listing`]。
enum Item {
    Dir {
        name: String,
        id: usize,
        times: (u32, u32),
    },
    File {
        name: String,
        size: u64,
        allocated: u64,
        times: (u32, u32),
    },
}

/// 修改时间和访问时间（Unix 秒，取不到时为 0）。Windows 上目录项自带，不需要额外的系统调用。
fn times(meta: &Metadata) -> (u32, u32) {
    let secs = |t: io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| u32::try_from(d.as_secs()).unwrap_or(u32::MAX))
    };
    (secs(meta.modified()), secs(meta.accessed()))
}

/// 一个待读的目录：编号、路径、所属顶层项目在 [`Live`] 里的下标（根目录本身为 `None`）。
type Job = (usize, PathBuf, Option<usize>);

/// 待读的目录。`active` 是排队中加上正在读的目录数，归零即全部完成。
struct Queue {
    jobs: Vec<Job>,
    active: usize,
}

pub fn walk_with_threads(
    root: &Path,
    threads: usize,
    progress: &(dyn Fn(u64) -> bool + Sync),
) -> io::Result<WalkResult> {
    run(root, threads, progress, None)
}

fn run(
    root: &Path,
    threads: usize,
    progress: &(dyn Fn(u64) -> bool + Sync),
    live: Option<&Live>,
) -> io::Result<WalkResult> {
    let meta = fs::symlink_metadata(root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "root is not a directory",
        ));
    }

    let cluster = sys::cluster_size(root);
    let queue = Mutex::new(Queue {
        jobs: vec![(0, root.to_path_buf(), None)],
        active: 1,
    });
    let ready = Condvar::new();
    let next_id = AtomicUsize::new(1);
    let seen = AtomicU64::new(0);
    let skipped = AtomicU64::new(0);
    let cancelled = AtomicBool::new(false);

    let listings: Vec<(usize, Vec<Item>)> = thread::scope(|s| {
        let workers: Vec<_> = (0..threads.max(1))
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    while let Some((id, path, top)) = take_job(&queue, &ready) {
                        // 取消后仍要把排队的目录逐个标记完成，否则等待中的线程永远等不到结束。
                        if cancelled.load(Ordering::Relaxed) {
                            finish_job(&queue, &ready, Vec::new());
                            continue;
                        }
                        let mut items = Vec::new();
                        let mut subdirs = Vec::new();
                        // 这个目录里文件的合计，读完后一次性记到所属的顶层项目上。
                        let (mut dir_size, mut dir_allocated, mut dir_files) = (0, 0, 0);
                        match fs::read_dir(&path) {
                            Ok(entries) => {
                                for entry in entries.flatten() {
                                    let Ok(meta) = entry.metadata() else { continue };
                                    let name = entry.file_name().to_string_lossy().into_owned();
                                    if meta.is_symlink() && is_directory(&meta) {
                                        // 链接目录：记下名称，不进入。
                                        let child = next_id.fetch_add(1, Ordering::Relaxed);
                                        let times = times(&meta);
                                        items.push(Item::Dir {
                                            name,
                                            id: child,
                                            times,
                                        });
                                    } else if meta.is_dir() {
                                        let child = next_id.fetch_add(1, Ordering::Relaxed);
                                        // 根目录下的文件夹各自成为一个顶层项目，更深的沿用父目录的。
                                        let child_top = match (live, top) {
                                            (Some(l), None) => Some(l.add(LiveItem {
                                                name: name.clone(),
                                                is_dir: true,
                                                size: 0,
                                                allocated: 0,
                                                files: 0,
                                            })),
                                            _ => top,
                                        };
                                        let times = times(&meta);
                                        items.push(Item::Dir {
                                            name,
                                            id: child,
                                            times,
                                        });
                                        subdirs.push((child, entry.path(), child_top));
                                    } else {
                                        let size = meta.len();
                                        let allocated =
                                            sys::allocated_size(|| entry.path(), &meta, cluster);
                                        match (live, top) {
                                            (Some(l), None) => {
                                                l.add(LiveItem {
                                                    name: name.clone(),
                                                    is_dir: false,
                                                    size,
                                                    allocated,
                                                    files: 1,
                                                });
                                            }
                                            _ => {
                                                dir_size += size;
                                                dir_allocated += allocated;
                                                dir_files += 1;
                                            }
                                        }
                                        items.push(Item::File {
                                            name,
                                            size,
                                            allocated,
                                            times: times(&meta),
                                        });
                                    }
                                    let n = seen.fetch_add(1, Ordering::Relaxed) + 1;
                                    if n.is_multiple_of(PROGRESS_EVERY) && !progress(n) {
                                        cancelled.store(true, Ordering::Relaxed);
                                    }
                                    if cancelled.load(Ordering::Relaxed) {
                                        break;
                                    }
                                }
                            }
                            Err(_) => {
                                skipped.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        if let (Some(l), Some(t)) = (live, top)
                            && dir_files > 0
                        {
                            l.grow(t, dir_size, dir_allocated, dir_files);
                        }
                        out.push((id, items));
                        finish_job(&queue, &ready, subdirs);
                    }
                    out
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("walk worker panicked"))
            .collect()
    });

    if cancelled.into_inner() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "扫描已取消"));
    }
    let mut tree = assemble(root, next_id.into_inner(), listings);
    let (m, a) = times(&meta);
    tree.set_times(tree.root(), m, a);
    progress(seen.into_inner());
    Ok(WalkResult {
        tree,
        skipped: skipped.into_inner(),
    })
}

/// 取一个待读目录；队列空但还有目录在读时等待，全部完成时返回 `None`。
fn take_job(queue: &Mutex<Queue>, ready: &Condvar) -> Option<Job> {
    let mut q = queue.lock().expect("walk queue poisoned");
    loop {
        if let Some(job) = q.jobs.pop() {
            return Some(job);
        }
        if q.active == 0 {
            return None;
        }
        q = ready.wait(q).expect("walk queue poisoned");
    }
}

/// 一个目录读完：把它的子目录放进队列，并唤醒等待的线程。
fn finish_job(queue: &Mutex<Queue>, ready: &Condvar, subdirs: Vec<Job>) {
    let mut q = queue.lock().expect("walk queue poisoned");
    q.active += subdirs.len();
    q.jobs.extend(subdirs);
    q.active -= 1;
    drop(q);
    ready.notify_all();
}

/// 从根目录开始逐层展开，保证父节点先于子节点加入树。
fn assemble(root: &Path, dirs: usize, listings: Vec<(usize, Vec<Item>)>) -> FileTree {
    let mut by_id: Vec<Option<Vec<Item>>> = (0..dirs).map(|_| None).collect();
    for (id, items) in listings {
        by_id[id] = Some(items);
    }

    let mut tree = FileTree::new(&root.to_string_lossy());
    let mut pending: VecDeque<(usize, NodeId)> = VecDeque::from([(0, tree.root())]);
    while let Some((id, node)) = pending.pop_front() {
        for item in by_id[id].take().unwrap_or_default() {
            match item {
                Item::Dir { name, id, times } => {
                    let child = tree.add_dir(node, &name);
                    tree.set_times(child, times.0, times.1);
                    pending.push_back((id, child));
                }
                Item::File {
                    name,
                    size,
                    allocated,
                    times,
                } => {
                    let child = tree.add_file(node, &name, size, allocated);
                    tree.set_times(child, times.0, times.1);
                }
            }
        }
    }
    tree.finalize();
    tree
}

/// 链接本身是不是目录（目录符号链接、连接点）。链接的元数据在 Windows 上类型是“符号链接”，
/// 不是目录，要看目录属性。`is_symlink` 只认符号链接和连接点，云文件等其他重解析点不算。
fn is_directory(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        meta.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
    }
    #[cfg(not(windows))]
    {
        meta.is_dir()
    }
}

#[cfg(windows)]
mod sys {
    //! 占用空间相关的 Windows API。

    use std::fs::Metadata;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{GetLastError, NO_ERROR};
    use windows_sys::Win32::Storage::FileSystem::{
        GetCompressedFileSizeW, GetDiskFreeSpaceW, GetVolumePathNameW, INVALID_FILE_SIZE,
    };

    /// 拿不到簇大小时的默认值，也是 NTFS 最常见的簇大小。
    const DEFAULT_CLUSTER: u64 = 4096;

    /// 以 0 结尾的 UTF-16 路径。超过 MAX_PATH 的绝对路径加上 `\\?\` 前缀。
    fn wide(path: &Path) -> Vec<u16> {
        let s = path.as_os_str();
        let mut out: Vec<u16> = Vec::with_capacity(s.len() + 8);
        if s.len() >= 248 && path.is_absolute() {
            let lossy = path.to_string_lossy();
            if let Some(unc) = lossy.strip_prefix(r"\\") {
                if !unc.starts_with(r"?\") {
                    out.extend(r"\\?\UNC\".encode_utf16());
                    out.extend(unc.encode_utf16());
                    out.push(0);
                    return out;
                }
            } else {
                out.extend(r"\\?\".encode_utf16());
            }
        }
        out.extend(s.encode_wide());
        out.push(0);
        out
    }

    /// `path` 所在卷的簇大小。
    pub fn cluster_size(path: &Path) -> u64 {
        let p = wide(path);
        let mut volume = [0u16; 512];
        let mut spc = 0u32;
        let mut bps = 0u32;
        let mut free = 0u32;
        let mut total = 0u32;
        // SAFETY: p 以 0 结尾并在调用期间有效；volume 是长度如实传入的可写缓冲区；
        // 四个输出参数都是有效的可写指针。
        let ok = unsafe {
            GetVolumePathNameW(p.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) != 0
                && GetDiskFreeSpaceW(volume.as_ptr(), &mut spc, &mut bps, &mut free, &mut total)
                    != 0
        };
        let cluster = u64::from(spc) * u64::from(bps);
        if ok && cluster > 0 {
            cluster
        } else {
            DEFAULT_CLUSTER
        }
    }

    /// 文件实际占用的磁盘空间，按簇向上取整。
    ///
    /// 只有压缩或稀疏文件才需要问系统（每次都要按路径打开文件，很慢），
    /// 其他文件的占用就是逻辑大小按簇取整。`path` 只在需要时才生成。
    pub fn allocated_size(path: impl FnOnce() -> PathBuf, meta: &Metadata, cluster: u64) -> u64 {
        const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
        const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x800;
        let size = meta.len();
        let bytes = if meta.file_attributes()
            & (FILE_ATTRIBUTE_SPARSE_FILE | FILE_ATTRIBUTE_COMPRESSED)
            != 0
        {
            compressed_size(&path()).unwrap_or(size)
        } else {
            size
        };
        bytes.div_ceil(cluster) * cluster
    }

    /// 压缩、稀疏文件实际占用的字节数。
    fn compressed_size(path: &Path) -> Option<u64> {
        let p = wide(path);
        let mut high = 0u32;
        // SAFETY: p 以 0 结尾并在调用期间有效；high 是有效的可写指针。
        let low = unsafe { GetCompressedFileSizeW(p.as_ptr(), &mut high) };
        // INVALID_FILE_SIZE 也可能是合法的低 32 位，要再看错误码。
        // SAFETY: GetLastError 没有前置条件。
        let failed = low == INVALID_FILE_SIZE && unsafe { GetLastError() } != NO_ERROR;
        (!failed).then(|| (u64::from(high) << 32) | u64::from(low))
    }
}

#[cfg(not(windows))]
mod sys {
    use std::fs::Metadata;
    use std::path::{Path, PathBuf};

    pub fn cluster_size(_: &Path) -> u64 {
        4096
    }

    pub fn allocated_size(_: impl FnOnce() -> PathBuf, meta: &Metadata, cluster: u64) -> u64 {
        meta.len().div_ceil(cluster) * cluster
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试目录放在仓库的 target 下（D 盘），不在系统临时目录（C 盘）里扫描。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            // crates/dl-walk 往上两级是仓库根目录；不用 `..`，mklink 不认这种路径。
            let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .unwrap();
            let dir = repo
                .join("target")
                .join("test-tmp")
                .join(format!("dl-walk-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn walks_and_sums_sizes() {
        let tmp = TempDir::new("sums");
        fs::create_dir_all(tmp.0.join("a/b")).unwrap();
        fs::write(tmp.0.join("a/one.bin"), vec![0u8; 1_000]).unwrap();
        fs::write(tmp.0.join("a/b/two.bin"), vec![0u8; 250]).unwrap();
        fs::write(tmp.0.join("top.txt"), b"hello").unwrap();

        let WalkResult { tree, skipped } = walk(&tmp.0).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(tree.size(tree.root()), 1_255);
        assert_eq!(tree.file_count(tree.root()), 3);
        // 每个小文件至少占一个簇。
        assert!(tree.allocated(tree.root()) >= 3 * 512);

        let top = tree.children_by_size(tree.root());
        assert_eq!(tree.name(top[0]), "a");
        assert_eq!(tree.size(top[0]), 1_250);

        // 刚写的文件，修改时间就是现在。
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;
        let file = tree.child_by_name(tree.root(), "top.txt").unwrap();
        assert!(
            now.abs_diff(tree.modified(file)) < 600,
            "{}",
            tree.modified(file)
        );
        assert!(tree.accessed(file) > 0);
        assert!(tree.modified(top[0]) > 0, "folders have times too");
    }

    #[test]
    fn many_threads_match_one_thread() {
        let tmp = TempDir::new("threads");
        for d in 0..20 {
            let dir = tmp.0.join(format!("d{d}/sub{}", d % 3));
            fs::create_dir_all(&dir).unwrap();
            for f in 0..15 {
                fs::write(dir.join(format!("f{f}.bin")), vec![0u8; d * 100 + f]).unwrap();
            }
        }
        let flatten = |t: &FileTree| {
            let mut v: Vec<(String, u64, u32)> = (0..t.len() as NodeId)
                .map(|i| (t.path(i), t.size(i), t.file_count(i)))
                .collect();
            v.sort();
            v
        };
        let one = walk_with_threads(&tmp.0, 1, &|_| true).unwrap();
        let many = walk_with_threads(&tmp.0, 8, &|_| true).unwrap();
        assert_eq!(flatten(&one.tree), flatten(&many.tree));
        assert_eq!(one.tree.file_count(0), 300);
    }

    #[test]
    fn live_totals_match_the_final_tree() {
        let tmp = TempDir::new("live");
        for d in 0..6 {
            let dir = tmp.0.join(format!("d{d}/x/y"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(tmp.0.join(format!("d{d}/a.bin")), vec![0u8; d * 1_000]).unwrap();
            fs::write(dir.join("b.bin"), vec![0u8; 77 + d]).unwrap();
        }
        fs::create_dir_all(tmp.0.join("empty")).unwrap();
        fs::write(tmp.0.join("root.txt"), b"12345").unwrap();

        let live = Live::default();
        let result = walk_live(&tmp.0, &|_| true, &live).unwrap();
        let t = &result.tree;
        let mut got: Vec<(String, bool, u64, u64, u64)> = live
            .snapshot()
            .into_iter()
            .map(|i| (i.name, i.is_dir, i.size, i.allocated, i.files))
            .collect();
        got.sort();
        let mut want: Vec<(String, bool, u64, u64, u64)> = t
            .children(t.root())
            .map(|c| {
                (
                    t.name(c).to_string(),
                    t.kind(c) == dl_tree::NodeKind::Dir,
                    t.size(c),
                    t.allocated(c),
                    u64::from(t.file_count(c)),
                )
            })
            .collect();
        want.sort();
        assert_eq!(got, want);
        assert_eq!(got.len(), 8);
    }

    #[test]
    fn reports_progress() {
        let tmp = TempDir::new("progress");
        fs::write(tmp.0.join("x"), b"1").unwrap();
        let last = AtomicU64::new(0);
        walk_with_progress(&tmp.0, &|n| {
            last.store(n, Ordering::Relaxed);
            true
        })
        .unwrap();
        assert_eq!(last.into_inner(), 1);
    }

    #[test]
    fn progress_can_cancel() {
        let tmp = TempDir::new("cancel");
        for d in 0..4 {
            let dir = tmp.0.join(format!("d{d}"));
            fs::create_dir_all(&dir).unwrap();
            for f in 0..PROGRESS_EVERY / 4 + 10 {
                fs::write(dir.join(format!("{f}")), b"").unwrap();
            }
        }
        let err = walk_with_threads(&tmp.0, 4, &|_| false).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn rejects_file_root() {
        let tmp = TempDir::new("file-root");
        let file = tmp.0.join("f.txt");
        fs::write(&file, b"x").unwrap();
        assert_eq!(walk(&file).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(windows)]
    #[test]
    fn does_not_follow_junctions() {
        let tmp = TempDir::new("junction");
        fs::create_dir_all(tmp.0.join("real")).unwrap();
        fs::write(tmp.0.join("real/data.bin"), vec![0u8; 4_096]).unwrap();
        let link = tmp.0.join("link");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(tmp.0.join("real"))
            .output()
            .unwrap();
        assert!(status.status.success(), "mklink /J failed");

        let result = walk(&tmp.0).unwrap();
        let t = &result.tree;
        assert_eq!(t.size(t.root()), 4_096);
        // 连接点本身记在树上，但是空的。
        let link = t.child_by_name(t.root(), "link").expect("junction listed");
        assert_eq!(t.children(link).count(), 0);
        assert_eq!(t.file_count(t.root()), 1);
    }

    /// NTFS 压缩后的文件：逻辑大小不变，占用空间应明显变小。`compact` 不需要管理员权限。
    #[cfg(windows)]
    #[test]
    fn compressed_file_reports_real_allocation() {
        let tmp = TempDir::new("compressed");
        let file = tmp.0.join("zeros.bin");
        fs::write(&file, vec![0u8; 8 << 20]).unwrap();
        let ok = std::process::Command::new("compact")
            .args(["/c", "/q"])
            .arg(&file)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "compact failed");

        let result = walk(&tmp.0).unwrap();
        let t = &result.tree;
        assert_eq!(t.size(t.root()), 8 << 20);
        assert!(t.allocated(t.root()) < 1 << 20, "{}", t.allocated(t.root()));
    }
}
