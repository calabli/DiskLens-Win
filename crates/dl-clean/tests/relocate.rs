//! 移动文件夹并留下连接点。跨磁盘的部分需要测试卷：
//!
//! ```text
//! $env:DL_TEST_VOLUME = 'T'
//! cargo test -p dl-clean --test relocate -- --ignored
//! ```

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};

use dl_clean::relocate::{make_junction, move_with_junction};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .join("target")
            .join("test-tmp")
            .join(format!("dl-clean-move-{tag}-{}", std::process::id()));
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

/// 一个有嵌套目录、中文名、只读文件和较大文件的文件夹。
fn fill(dir: &Path) {
    fs::create_dir_all(dir.join(r"子目录\更深")).unwrap();
    fs::write(dir.join("说明.txt"), "你好").unwrap();
    fs::write(dir.join(r"子目录\更深\data.bin"), vec![7u8; 3 << 20]).unwrap();
    let ro = dir.join(r"子目录\只读.cfg");
    fs::write(&ro, b"readonly").unwrap();
    let mut p = fs::metadata(&ro).unwrap().permissions();
    p.set_readonly(true);
    fs::set_permissions(&ro, p).unwrap();
}

fn read_all(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                out.push((rel, fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn no_progress() -> impl FnMut(u64, u64) -> bool {
    |_, _| true
}

#[test]
fn refuses_same_drive_and_folders_with_links() {
    let t = TempDir::new("refuse");
    let src = t.0.join("src");
    fill(&src);
    let err = move_with_junction(&src, &t.0.join("dest"), &mut no_progress()).unwrap_err();
    assert!(err.contains("同一个磁盘"), "{err}");

    make_junction(&src.join("link"), &t.0).unwrap();
    let err = move_with_junction(&src, Path::new(r"T:\x"), &mut no_progress()).unwrap_err();
    assert!(err.contains("链接") || err.contains("磁盘"), "{err}");
    fs::remove_dir(src.join("link")).unwrap();
    assert!(src.join("说明.txt").exists(), "nothing changed");
}

#[test]
fn junctions_point_at_their_target() {
    let t = TempDir::new("junction");
    let target = t.0.join("目标");
    fill(&target);
    let link = t.0.join("链接");
    make_junction(&link, &target).unwrap();
    assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(read_all(&link), read_all(&target));
    // 删除连接点只去掉链接本身。
    fs::remove_dir(&link).unwrap();
    assert!(target.join("说明.txt").exists());
}

fn test_volume() -> String {
    std::env::var("DL_TEST_VOLUME").expect("set DL_TEST_VOLUME")
}

#[test]
#[ignore = "需要测试卷，设置 DL_TEST_VOLUME 后用 --ignored 运行"]
fn moves_to_another_drive_and_leaves_a_junction() {
    let t = TempDir::new("move");
    let src = t.0.join("大文件夹");
    fill(&src);
    let before = read_all(&src);
    let dest = PathBuf::from(format!(
        r"{}:\dl-move-{}\大文件夹",
        test_volume(),
        std::process::id()
    ));

    let mut calls = 0;
    let moved = move_with_junction(&src, &dest, &mut |done, total| {
        calls += 1;
        assert!(done <= total);
        true
    })
    .unwrap();
    assert_eq!(moved.files, 3);
    assert_eq!(moved.bytes, (3 << 20) + 6 + 8);
    assert_eq!(moved.leftover, None);
    assert!(calls >= 2);

    assert!(
        fs::symlink_metadata(&src).unwrap().is_symlink(),
        "now a junction"
    );
    assert_eq!(read_all(&src), before, "same content through the junction");
    assert_eq!(read_all(&dest), before);
    // 旧文件夹已删除。
    let leftovers: Vec<_> = fs::read_dir(&t.0).unwrap().flatten().collect();
    assert_eq!(leftovers.len(), 1);

    fs::remove_dir(&src).unwrap();
    let _ = fs::remove_dir_all(dest.parent().unwrap());
}

#[test]
#[ignore = "需要测试卷，设置 DL_TEST_VOLUME 后用 --ignored 运行"]
fn file_in_use_changes_nothing() {
    use std::os::windows::fs::OpenOptionsExt;
    let t = TempDir::new("busy");
    let src = t.0.join("占用");
    fill(&src);
    let before = read_all(&src);
    let dest = PathBuf::from(format!(
        r"{}:\dl-move-busy-{}",
        test_volume(),
        std::process::id()
    ));

    // 不允许其他程序删除或改名的打开方式，模拟正在使用的文件。
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(src.join("说明.txt"))
        .unwrap();
    let err = move_with_junction(&src, &dest, &mut no_progress()).unwrap_err();
    drop(handle);
    assert!(err.contains("正被使用"), "{err}");
    assert!(!fs::symlink_metadata(&src).unwrap().is_symlink());
    assert_eq!(read_all(&src), before);
    assert!(!dest.exists());
}

#[test]
#[ignore = "需要测试卷，设置 DL_TEST_VOLUME 后用 --ignored 运行"]
fn cancelling_rolls_back() {
    let t = TempDir::new("cancel");
    let src = t.0.join("取消");
    fill(&src);
    let before = read_all(&src);
    let dest = PathBuf::from(format!(
        r"{}:\dl-move-cancel-{}",
        test_volume(),
        std::process::id()
    ));

    let mut n = 0;
    let err = move_with_junction(&src, &dest, &mut |_, _| {
        n += 1;
        n < 3
    })
    .unwrap_err();
    assert_eq!(err, "已取消");
    assert!(
        !fs::symlink_metadata(&src).unwrap().is_symlink(),
        "original folder restored"
    );
    assert_eq!(read_all(&src), before);
    assert!(!dest.exists(), "partial copy removed");
}
