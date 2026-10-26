//! 在测试卷上对比 MFT 直读与普通遍历的结果。
//!
//! 需要管理员权限和 `tests/fixtures/New-TestVolume.ps1` 生成的测试卷，默认忽略：
//!
//! ```text
//! $env:DL_TEST_VOLUME = 'T'
//! cargo test -p dl-ntfs --test compare_with_walk -- --ignored
//! ```

#![cfg(windows)]

use std::path::Path;

/// 两种方式本来就会有差异的顶层目录：
/// - hardlinks：MFT 对硬链接只算一次，普通遍历每个名称都算
/// - 系统目录：普通遍历通常无权读取
const EXPECTED_DIFFERENT: &[&str] = &["hardlinks", "System Volume Information", "$RECYCLE.BIN"];

#[test]
#[ignore = "需要管理员权限和测试卷，设置 DL_TEST_VOLUME 后用 --ignored 运行"]
fn mft_matches_walk_on_test_volume() {
    let letter = std::env::var("DL_TEST_VOLUME")
        .ok()
        .and_then(|s| s.chars().next())
        .expect("set DL_TEST_VOLUME to the test volume's drive letter");

    let mft = dl_ntfs::scan_volume(letter, |_| {}).expect("MFT scan (run as administrator)");
    let walk = dl_walk::walk(Path::new(&format!(r"{letter}:\"))).unwrap();
    let (m, w) = (&mft.tree, &walk.tree);

    assert_eq!(mft.corrupt_records, 0);
    let mut compared = 0;
    for id in w.children(w.root()) {
        let name = w.name(id);
        if EXPECTED_DIFFERENT.contains(&name) {
            continue;
        }
        let other = m
            .child_by_name(m.root(), name)
            .unwrap_or_else(|| panic!("{name} missing from MFT tree"));
        assert_eq!(m.size(other), w.size(id), "logical size of {name}");
        assert_eq!(
            m.file_count(other),
            w.file_count(id),
            "file count of {name}"
        );
        compared += 1;
    }
    assert!(
        compared >= 8,
        "test volume looks incomplete ({compared} folders compared)"
    );

    // 硬链接目录：MFT 只算一份 10 MiB，普通遍历算三份。
    let hm = m.child_by_name(m.root(), "hardlinks").unwrap();
    let hw = w.child_by_name(w.root(), "hardlinks").unwrap();
    assert_eq!(m.size(hm), 10 * 1024 * 1024);
    assert_eq!(w.size(hw), 3 * 10 * 1024 * 1024);
}
