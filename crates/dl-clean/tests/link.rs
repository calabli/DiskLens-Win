//! 硬链接替换的测试，在仓库的 target 目录（D 盘）里进行；跨磁盘的情况在测试卷上验证。

use std::fs;
use std::path::{Path, PathBuf};

use dl_clean::link::replace_with_hardlink;
use dl_clean::{OpLog, Outcome, Protector, Target, link_run};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .join("target")
            .join("test-tmp")
            .join(format!("dl-clean-link-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str, data: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        fs::write(&p, data).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn data(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

/// 是否已经是同一个文件：改一处，另一处跟着变。
fn linked(a: &Path, b: &Path) -> bool {
    let before = fs::read(b).unwrap();
    let mut changed = fs::read(a).unwrap();
    changed[0] ^= 0xff;
    fs::write(a, &changed).unwrap();
    let after = fs::read(b).unwrap();
    changed[0] ^= 0xff;
    fs::write(a, &changed).unwrap();
    before != after
}

#[test]
fn replaces_identical_copy_with_a_link() {
    let t = TempDir::new("ok");
    let content = data(3 * 1024 * 1024 + 17, 1);
    let keep = t.file("keep.bin", &content);
    let copy = t.file("copy.bin", &content);

    assert_eq!(replace_with_hardlink(&keep, &copy), Ok(true));
    assert_eq!(fs::read(&copy).unwrap(), content);
    assert!(linked(&keep, &copy));
    // 没有留下临时文件。
    assert_eq!(fs::read_dir(&t.0).unwrap().count(), 2);
    // 再做一次：已经是同一个文件。
    assert_eq!(replace_with_hardlink(&keep, &copy), Ok(false));
}

#[test]
fn refuses_when_content_differs_even_at_the_end() {
    let t = TempDir::new("differ");
    let content = data(2 * 1024 * 1024, 2);
    let mut other = content.clone();
    *other.last_mut().unwrap() ^= 1;
    let keep = t.file("keep.bin", &content);
    let copy = t.file("copy.bin", &other);

    let err = replace_with_hardlink(&keep, &copy).unwrap_err();
    assert!(err.contains("内容已经不同"), "{err}");
    assert_eq!(fs::read(&copy).unwrap(), other, "target untouched");
    assert!(!linked(&keep, &copy));

    let shorter = t.file("short.bin", &content[..100]);
    assert!(replace_with_hardlink(&keep, &shorter).is_err());
}

#[test]
fn replaces_read_only_copy() {
    let t = TempDir::new("readonly");
    let keep = t.file("keep.bin", b"same bytes");
    let copy = t.file("copy.bin", b"same bytes");
    let mut p = fs::metadata(&copy).unwrap().permissions();
    p.set_readonly(true);
    fs::set_permissions(&copy, p).unwrap();

    assert_eq!(replace_with_hardlink(&keep, &copy), Ok(true));
    assert!(linked(&keep, &copy));
}

#[test]
fn link_run_checks_protection_and_logs() {
    let t = TempDir::new("run");
    let keep = t.file("keep.bin", b"payload");
    let copy = t.file("copy.bin", b"payload");
    let guarded = t.file("guarded.bin", b"payload");
    let protector = Protector::new(&[], &[]);
    let protector_guarded = Protector::new(&[guarded.to_str().unwrap()], &[]);
    let log = OpLog::new(t.0.join("logs"));
    let target = |p: &Path| Target {
        path: p.to_path_buf(),
        size: 7,
    };

    let (out, err) = link_run(&[(target(&keep), target(&copy))], &protector, &log);
    assert_eq!(out, [Outcome::Linked]);
    assert!(err.is_none());
    let (out, _) = link_run(
        &[(target(&keep), target(&guarded))],
        &protector_guarded,
        &log,
    );
    assert!(matches!(out[0], Outcome::Blocked(_)));
    assert!(!linked(&keep, &guarded));

    let lines: Vec<serde_json::Value> = fs::read_dir(t.0.join("logs"))
        .unwrap()
        .flatten()
        .flat_map(|f| {
            fs::read_to_string(f.path())
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["action"], "hardlink");
    assert_eq!(lines[0]["result"], "linked");
    assert!(lines[0]["link_to"].as_str().unwrap().ends_with("keep.bin"));
    assert_eq!(lines[1]["result"], "blocked");
}

/// 需要测试卷：`$env:DL_TEST_VOLUME='T'; cargo test -p dl-clean --test link -- --ignored`
#[test]
#[ignore = "需要测试卷，设置 DL_TEST_VOLUME 后用 --ignored 运行"]
fn refuses_across_volumes() {
    let letter = std::env::var("DL_TEST_VOLUME").expect("set DL_TEST_VOLUME");
    let t = TempDir::new("cross");
    let keep = t.file("keep.bin", b"cross volume");
    let dir = PathBuf::from(format!(r"{letter}:\dl-clean-link-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("copy.bin");
    fs::write(&copy, b"cross volume").unwrap();

    let err = replace_with_hardlink(&keep, &copy).unwrap_err();
    fs::remove_dir_all(&dir).unwrap();
    assert!(err.contains("不在同一个磁盘"), "{err}");
}
