//! 在测试卷上验证 USN 增量刷新：扫描后改动文件，按日志刷新得到的树应与重新完整扫描一致。
//!
//! 需要管理员权限和测试卷，默认忽略。测试卷没有启用 USN 日志时会先启用（32 MB）：
//!
//! ```text
//! $env:DL_TEST_VOLUME = 'T'
//! cargo test -p dl-ntfs --test usn_refresh -- --ignored --nocapture
//! ```

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use dl_ntfs::usn::{self, FileSizer, NodeIndex};
use dl_tree::{FileTree, NodeId};

/// 刷新和完整扫描本来就可能不同的顶层项目：NTFS 元文件不写日志，系统目录会自己变化。
fn compared(path: &str, root: &str) -> bool {
    let rest = path.strip_prefix(root).unwrap_or(path);
    let top = rest.split('\\').next().unwrap_or("");
    !(top.starts_with('$') || top == "System Volume Information")
}

fn listing(t: &FileTree) -> Vec<(String, u64, u64, u32)> {
    let root = t.path(t.root());
    let mut v: Vec<_> = (0..t.len() as NodeId)
        .filter(|&i| i != t.root() && t.is_attached(i))
        .map(|i| (t.path(i), t.size(i), t.allocated(i), t.file_count(i)))
        .filter(|(p, ..)| compared(p, &root))
        .collect();
    v.sort();
    v
}

/// 在 `dir` 下做一批有代表性的改动。
fn make_changes(dir: &Path) {
    let sub = dir.join("子目录");
    let deep = sub.join("deep");
    fs::create_dir_all(&deep).unwrap();
    fs::write(dir.join("small.txt"), b"hello").unwrap(); // 常驻在 MFT 记录里
    fs::write(dir.join("big.bin"), vec![7u8; 3 * 1024 * 1024]).unwrap();
    fs::write(sub.join("move-me.bin"), vec![1u8; 200_000]).unwrap();
    fs::write(deep.join("doomed.bin"), vec![2u8; 50_000]).unwrap();
    fs::write(dir.join("grow.log"), b"x").unwrap();
    fs::write(dir.join("报告 2026.docx"), vec![3u8; 12_345]).unwrap();

    // 改名、移动、增长、删除文件和整个目录。
    fs::rename(dir.join("报告 2026.docx"), dir.join("报告-final.docx")).unwrap();
    fs::rename(sub.join("move-me.bin"), dir.join("moved.bin")).unwrap();
    fs::write(dir.join("grow.log"), vec![b'y'; 1_000_000]).unwrap();
    fs::remove_dir_all(&deep).unwrap();
    // 目录改名：子项的路径随之改变。
    fs::write(sub.join("inside.txt"), vec![4u8; 70_000]).unwrap();
    fs::rename(&sub, dir.join("改名后的目录")).unwrap();
    // 先删除再用同名新建。
    fs::remove_file(dir.join("big.bin")).unwrap();
    fs::write(dir.join("big.bin"), vec![8u8; 1024 * 1024]).unwrap();
}

#[test]
#[ignore = "需要管理员权限和测试卷，设置 DL_TEST_VOLUME 后用 --ignored 运行"]
fn refresh_matches_full_rescan() {
    let letter = std::env::var("DL_TEST_VOLUME")
        .ok()
        .and_then(|s| s.chars().next())
        .expect("set DL_TEST_VOLUME to the test volume's drive letter");
    let dir = PathBuf::from(format!(r"{letter}:\usn-test"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let volume = dl_ntfs::open_volume(letter).expect("open volume (run as administrator)");
    if usn::query_journal(&volume).is_err() {
        println!("测试卷没有启用 USN 日志，现在启用");
        usn::create_journal(&volume, 32 * 1024 * 1024).unwrap();
    }
    // 先记日志位置再扫描：扫描期间的变化会在刷新时重放一遍，结果不受影响。
    let (pos, _) = usn::query_journal(&volume).unwrap();
    let mut tree = dl_ntfs::scan_volume(letter, |_| true).unwrap().tree;

    make_changes(&dir);

    let started = Instant::now();
    let (records, next) = usn::read_journal(&volume, pos).unwrap();
    let changes = usn::fold(&records);
    let sizer = FileSizer::open(letter).unwrap();
    let mut index = NodeIndex::build(&tree);
    let stats = usn::apply(&mut tree, &mut index, &changes, |c| sizer.size(c));
    println!(
        "{} 条记录，{} 个文件有变化，{stats:?}，用时 {:?}",
        records.len(),
        changes.len(),
        started.elapsed()
    );
    assert!(next.next_usn > pos.next_usn);

    let fresh = dl_ntfs::scan_volume(letter, |_| true).unwrap().tree;
    let (got, want) = (listing(&tree), listing(&fresh));
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g, w, "first difference");
    }
    assert_eq!(got.len(), want.len(), "number of items");
    assert!(
        got.iter()
            .any(|(p, ..)| p.ends_with(r"改名后的目录\inside.txt")),
        "renamed folder"
    );

    // 刷新后再刷新一次：没有新变化。
    let (records, _) = usn::read_journal(&volume, next).unwrap();
    let again = usn::apply(&mut tree, &mut index, &usn::fold(&records), |c| {
        sizer.size(c)
    });
    assert_eq!(again.added + again.removed, 0, "{again:?}");

    fs::remove_dir_all(&dir).unwrap();
}
