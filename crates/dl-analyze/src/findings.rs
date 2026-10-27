//! 专项查找：空文件夹、空文件、失效的符号链接 / 连接点、失效的快捷方式（.lnk）。
//!
//! 先从扫描结果里挑出候选，再逐个到磁盘上核实，宁可漏报也不误报：
//!
//! - 空文件夹：子树里没有文件；到磁盘上逐层确认只有空的子文件夹，含链接的不算。只报最外层。
//! - 空文件：磁盘上大小为 0（MFT 扫描里硬链接的第二个名称也记为 0，但并不是空文件）。
//! - 失效链接：只看符号链接和连接点，目标不存在才算；目标所在的磁盘不存在时不算
//!   （可能是没接上的移动硬盘）。云文件等其他重解析点不看。
//! - 失效快捷方式：从 .lnk 里读出本机路径形式的目标，判断方法同上；网络路径不看。

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;

use dl_tree::{FileTree, NodeId, NodeKind};

/// 核实空文件夹时最多往下看几层，防止异常深的目录拖慢查找。
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    EmptyDir,
    EmptyFile,
    BrokenLink,
    BrokenShortcut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub id: NodeId,
    pub path: PathBuf,
    pub kind: Kind,
    /// 链接或快捷方式原本指向的目标。
    pub target: Option<String>,
}

/// 候选：哪些检查值得做。同一个节点可能同时是几种候选（例如空的叶子目录）。
#[derive(Debug, Clone)]
pub struct Candidate {
    id: NodeId,
    path: PathBuf,
    /// 最外层的空文件夹（子树里没有文件，父目录不是）。
    empty_dir: bool,
    /// 没有子项的目录或大小为 0 的文件：可能是链接。
    maybe_link: bool,
    /// 大小为 0 的文件。
    empty_file: bool,
    shortcut: bool,
}

/// 从扫描结果里挑出 `root` 子树中的候选。只读树，不访问磁盘。
pub fn candidates(tree: &FileTree, root: NodeId) -> Vec<Candidate> {
    let mut out = Vec::new();
    // （节点，父目录是否也是空的）
    let mut stack: Vec<(NodeId, bool)> = tree.children(root).map(|c| (c, false)).collect();
    while let Some((n, parent_empty)) = stack.pop() {
        let mut c = Candidate {
            id: n,
            path: PathBuf::new(),
            empty_dir: false,
            maybe_link: false,
            empty_file: false,
            shortcut: false,
        };
        match tree.kind(n) {
            NodeKind::Dir => {
                let empty = tree.file_count(n) == 0;
                c.empty_dir = empty && !parent_empty;
                c.maybe_link = tree.children(n).next().is_none();
                stack.extend(tree.children(n).map(|k| (k, empty)));
            }
            NodeKind::File => {
                c.empty_file = tree.size(n) == 0;
                c.maybe_link = c.empty_file;
                c.shortcut = tree.name(n).to_ascii_lowercase().ends_with(".lnk");
            }
        }
        if c.empty_dir || c.maybe_link || c.empty_file || c.shortcut {
            c.path = PathBuf::from(tree.path(n));
            out.push(c);
        }
    }
    out
}

/// 在 `root` 子树里查找，见 [`verify_all`]。
pub fn find(
    tree: &FileTree,
    root: NodeId,
    threads: usize,
    progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> io::Result<Vec<Finding>> {
    verify_all(&candidates(tree, root), threads, progress)
}

/// 到磁盘上逐个核实候选。`progress(已核实, 候选总数)` 返回 `false` 即取消，此时返回
/// [`io::ErrorKind::Interrupted`]；它可能从任意工作线程调用。结果按类型、再按路径排列。
pub fn verify_all(
    cands: &[Candidate],
    threads: usize,
    progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> io::Result<Vec<Finding>> {
    let total = cands.len() as u64;
    let next = AtomicUsize::new(0);
    let done = AtomicU64::new(0);
    let cancelled = AtomicBool::new(false);
    let mut found: Vec<Finding> = thread::scope(|s| {
        let workers: Vec<_> = (0..threads.clamp(1, cands.len().max(1)))
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= cands.len() || cancelled.load(Ordering::Relaxed) {
                            break;
                        }
                        if let Some(f) = verify(&cands[i]) {
                            mine.push(f);
                        }
                        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if !progress(d, total) {
                            cancelled.store(true, Ordering::Relaxed);
                        }
                    }
                    mine
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("findings worker panicked"))
            .collect()
    });
    if cancelled.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "查找已取消"));
    }
    found.sort_by(|a, b| {
        (a.kind as u8)
            .cmp(&(b.kind as u8))
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(found)
}

/// 到磁盘上核实一个候选。
fn verify(c: &Candidate) -> Option<Finding> {
    let finding = |kind, target| {
        Some(Finding {
            id: c.id,
            path: c.path.clone(),
            kind,
            target,
        })
    };
    let meta = fs::symlink_metadata(&c.path).ok()?;
    if meta.is_symlink() {
        let target = fs::read_link(&c.path).ok()?;
        return broken(&c.path, &target)
            .then(|| finding(Kind::BrokenLink, Some(display(&target))))
            .flatten();
    }
    if c.empty_dir && meta.is_dir() && really_empty(&c.path, 0) {
        return finding(Kind::EmptyDir, None);
    }
    if c.shortcut && meta.is_file() {
        let bytes = fs::read(&c.path).ok()?;
        let target = PathBuf::from(shortcut_target(&bytes)?);
        if !target.is_absolute() {
            return None;
        }
        return (drive_present(&target) && !exists(&target))
            .then(|| finding(Kind::BrokenShortcut, Some(display(&target))))
            .flatten();
    }
    if c.empty_file && meta.is_file() && meta.len() == 0 {
        return finding(Kind::EmptyFile, None);
    }
    None
}

/// 链接 `link` 指向 `target`，目标不存在、但目标所在的磁盘存在。
fn broken(link: &Path, target: &Path) -> bool {
    // 相对目标以链接所在目录为起点。
    let full = if target.is_absolute() {
        target.to_path_buf()
    } else {
        link.parent()
            .map_or_else(|| target.to_path_buf(), |p| p.join(target))
    };
    drive_present(&full)
        && matches!(fs::metadata(link), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

fn exists(path: &Path) -> bool {
    !matches!(fs::metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

/// 路径所在的磁盘（或共享）是否存在。
fn drive_present(path: &Path) -> bool {
    let mut root = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => root.push(c.as_os_str()),
            _ => break,
        }
    }
    !root.as_os_str().is_empty() && fs::metadata(&root).is_ok()
}

/// 目录里只有空的子目录（任意层），没有文件，也没有链接。
fn really_empty(dir: &Path, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries {
        let Ok(entry) = entry else { return false };
        let Ok(meta) = entry.metadata() else {
            return false;
        };
        if meta.is_symlink() || !meta.is_dir() || !really_empty(&entry.path(), depth + 1) {
            return false;
        }
    }
    true
}

/// 去掉 `\\?\` 前缀，方便阅读。
fn display(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

/// 从 .lnk 文件（Shell Link 二进制格式，MS-SHLLINK）里读出本机路径形式的目标。
/// 目标只有 Shell 项目标识（例如“此电脑”）或是网络路径时返回 `None`。
pub fn shortcut_target(b: &[u8]) -> Option<String> {
    let u16_at = |o: usize| b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |o: usize| {
        b.get(o..o + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    const HEADER: usize = 0x4C;
    const HAS_ID_LIST: u32 = 0x1;
    const HAS_LINK_INFO: u32 = 0x2;
    const LOCAL_BASE_PATH: u32 = 0x1;

    if u32_at(0)? != HEADER as u32 {
        return None;
    }
    let flags = u32_at(0x14)?;
    let mut off = HEADER;
    if flags & HAS_ID_LIST != 0 {
        off += 2 + usize::from(u16_at(off)?);
    }
    if flags & HAS_LINK_INFO == 0 {
        return None;
    }
    let li = off;
    let li_size = u32_at(li)? as usize;
    let header_size = u32_at(li + 4)? as usize;
    let info = b.get(li..li + li_size)?;
    if u32_at(li + 8)? & LOCAL_BASE_PATH == 0 {
        return None;
    }
    let (base, suffix) = if header_size >= 0x24 {
        (
            utf16z(info, u32_at(li + 28)? as usize)?,
            utf16z(info, u32_at(li + 32)? as usize)?,
        )
    } else {
        (
            ansiz(info, u32_at(li + 16)? as usize)?,
            ansiz(info, u32_at(li + 24)? as usize)?,
        )
    };
    let target = if suffix.is_empty() || base.ends_with('\\') {
        base + &suffix
    } else {
        format!("{base}\\{suffix}")
    };
    (!target.is_empty()).then_some(target)
}

/// `b[off..]` 处以 0 结尾的 UTF-16 字符串。
fn utf16z(b: &[u8], off: usize) -> Option<String> {
    let units: Vec<u16> = b
        .get(off..)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16(&units).ok()
}

/// `b[off..]` 处以 0 结尾的系统代码页（ANSI）字符串。
fn ansiz(b: &[u8], off: usize) -> Option<String> {
    let bytes: Vec<u8> = b
        .get(off..)?
        .iter()
        .copied()
        .take_while(|&c| c != 0)
        .collect();
    if bytes.is_ascii() {
        return String::from_utf8(bytes).ok();
    }
    sys::ansi_to_string(&bytes)
}

#[cfg(windows)]
mod sys {
    use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar};

    pub fn ansi_to_string(bytes: &[u8]) -> Option<String> {
        let len = i32::try_from(bytes.len()).ok()?;
        let mut wide = vec![0u16; bytes.len()];
        // SAFETY: 输入、输出缓冲区的指针和长度都来自有效的切片；ANSI 每字节至多一个 UTF-16 单元。
        let n = unsafe {
            MultiByteToWideChar(
                CP_ACP,
                0,
                bytes.as_ptr(),
                len,
                wide.as_mut_ptr(),
                wide.len() as i32,
            )
        };
        (n > 0).then(|| String::from_utf16_lossy(&wide[..n as usize]))
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn ansi_to_string(_: &[u8]) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                .join(format!("findings-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn run(dir: &Path) -> Vec<(Kind, String)> {
        let tree = dl_walk::walk(dir).unwrap().tree;
        find(&tree, tree.root(), 4, &|_, _| true)
            .unwrap()
            .into_iter()
            .map(|f| {
                let rel = f
                    .path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                (f.kind, rel)
            })
            .collect()
    }

    #[test]
    fn finds_empty_folders_and_files() {
        let t = TempDir::new("empty");
        fs::create_dir_all(t.path("empty/a/b/c")).unwrap();
        fs::create_dir_all(t.path("full/sub")).unwrap();
        fs::write(t.path("full/sub/x.txt"), b"x").unwrap();
        fs::create_dir_all(t.path("full/hollow")).unwrap();
        fs::write(t.path("zero.txt"), b"").unwrap();
        fs::write(t.path("full/__init__.py"), b"").unwrap();

        let got = run(&t.0);
        assert_eq!(
            got,
            [
                (Kind::EmptyDir, "empty".to_string()),
                (Kind::EmptyDir, r"full\hollow".to_string()),
                (Kind::EmptyFile, r"full\__init__.py".to_string()),
                (Kind::EmptyFile, "zero.txt".to_string()),
            ]
        );
    }

    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "mklink /J failed");
    }

    #[cfg(windows)]
    #[test]
    fn broken_junctions_only() {
        let t = TempDir::new("links");
        fs::create_dir_all(t.path("real")).unwrap();
        fs::write(t.path("real/f.bin"), b"data").unwrap();
        fs::create_dir_all(t.path("gone")).unwrap();
        junction(&t.path("good"), &t.path("real"));
        junction(&t.path("bad"), &t.path("gone"));
        fs::remove_dir(t.path("gone")).unwrap();
        // 只含一个连接点的文件夹不是空文件夹。
        fs::create_dir_all(t.path("holder")).unwrap();
        junction(&t.path(r"holder\inner"), &t.path("real"));

        let got = run(&t.0);
        assert_eq!(got, [(Kind::BrokenLink, "bad".to_string())]);
    }

    /// 用 Windows 自带的 WScript.Shell 建快捷方式，目标路径含中文。
    #[cfg(windows)]
    fn shortcut(lnk: &Path, target: &Path) {
        let script = format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.Save()",
            lnk.display(),
            target.display()
        );
        let ok = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "creating shortcut failed");
    }

    #[cfg(windows)]
    #[test]
    fn broken_shortcuts() {
        let t = TempDir::new("lnk");
        fs::write(t.path("报告 最终版.docx"), b"doc").unwrap();
        fs::write(t.path("会被删除.txt"), b"bye").unwrap();
        shortcut(&t.path("好的.lnk"), &t.path("报告 最终版.docx"));
        shortcut(&t.path("坏的.lnk"), &t.path("会被删除.txt"));
        shortcut(&t.path("没接的盘.lnk"), Path::new(r"Q:\不存在的盘\a.txt"));

        let bytes = fs::read(t.path("好的.lnk")).unwrap();
        assert_eq!(
            shortcut_target(&bytes).map(PathBuf::from),
            Some(t.path("报告 最终版.docx"))
        );

        fs::remove_file(t.path("会被删除.txt")).unwrap();
        let got = run(&t.0);
        assert_eq!(got, [(Kind::BrokenShortcut, "坏的.lnk".to_string())]);
    }

    #[test]
    fn rejects_malformed_shortcuts() {
        assert_eq!(shortcut_target(b""), None);
        assert_eq!(shortcut_target(&[0x4C, 0, 0, 0]), None);
        let mut header = vec![0u8; 0x4C];
        header[0] = 0x4C;
        header[0x14] = 0x3; // 声称有 IDList 和 LinkInfo，但数据不完整
        assert_eq!(shortcut_target(&header), None);
    }
}
