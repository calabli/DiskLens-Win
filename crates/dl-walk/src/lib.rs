//! 普通目录遍历。
//!
//! 逐个打开目录读取条目，适用于 MFT 直读不可用的场合（非 NTFS 卷、网络盘、未提权）。
//! 不跟随符号链接和连接点，避免重复统计和循环。读不了的目录计入
//! [`WalkResult::skipped`] 后继续，不中断整次扫描。

use std::fs::{self, Metadata};
use std::io;
use std::path::Path;

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
    walk_with_progress(root, |_| {})
}

/// 同 [`walk`]，并定期以已处理的条目数调用 `progress`。
pub fn walk_with_progress(root: &Path, mut progress: impl FnMut(u64)) -> io::Result<WalkResult> {
    let meta = fs::symlink_metadata(root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "root is not a directory",
        ));
    }

    let mut tree = FileTree::new(&root.to_string_lossy());
    let mut skipped = 0;
    let mut seen = 0u64;
    let mut stack: Vec<(NodeId, std::path::PathBuf)> = vec![(tree.root(), root.to_path_buf())];

    while let Some((dir_id, dir_path)) = stack.pop() {
        let entries = match fs::read_dir(&dir_path) {
            Ok(entries) => entries,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if is_link(&meta) {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if meta.is_dir() {
                let id = tree.add_dir(dir_id, &name);
                stack.push((id, entry.path()));
            } else {
                // 普通遍历拿不到按簇的占用空间，暂以逻辑大小代替。
                tree.add_file(dir_id, &name, meta.len(), meta.len());
            }
            seen += 1;
            if seen.is_multiple_of(PROGRESS_EVERY) {
                progress(seen);
            }
        }
    }

    tree.finalize();
    progress(seen);
    Ok(WalkResult { tree, skipped })
}

/// 符号链接和连接点（以及其他重解析点）都不跟随。
fn is_link(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("dl-walk-{tag}-{}", std::process::id()));
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

        let top = tree.children_by_size(tree.root());
        assert_eq!(tree.name(top[0]), "a");
        assert_eq!(tree.size(top[0]), 1_250);
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
        assert_eq!(result.tree.size(result.tree.root()), 4_096);
    }
}
