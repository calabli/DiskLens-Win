//! 真实删除文件的测试。彻底删除在仓库的 target 目录（D 盘）里进行；
//! 回收站测试只在测试卷上运行，避免把测试文件放进 D 盘的回收站。

use std::fs;
use std::path::{Path, PathBuf};

use dl_clean::{Mode, OpLog, Outcome, Protector, Target, delete::delete, run};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .join("target")
            .join("test-tmp")
            .join(format!("dl-clean-{tag}-{}", std::process::id()));
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
fn permanent_delete_handles_files_and_folders() {
    let tmp = TempDir::new("permanent");
    let file = tmp.0.join("readonly.txt");
    fs::write(&file, b"x").unwrap();
    let mut perms = fs::metadata(&file).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&file, perms).unwrap();

    let dir = tmp.0.join("folder");
    fs::create_dir_all(dir.join("nested")).unwrap();
    fs::write(dir.join("nested/a.bin"), vec![0u8; 1024]).unwrap();

    delete(&file, Mode::Permanent).unwrap();
    delete(&dir, Mode::Permanent).unwrap();
    assert!(!file.exists());
    assert!(!dir.exists());
    assert_eq!(
        delete(&tmp.0.join("missing"), Mode::Permanent),
        Err("文件已不存在".to_string())
    );
}

/// 彻底删除逐个文件报告进度，合计等于删掉的文件大小；里面的连接点只删链接，
/// 不会顺着它删掉别处的文件；里面的只读文件也能删掉。
#[cfg(windows)]
#[test]
fn permanent_delete_reports_progress_and_never_follows_junctions() {
    use dl_clean::delete::delete_with;

    let tmp = TempDir::new("progress");
    let outside = tmp.0.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("precious.txt"), b"keep me").unwrap();

    let dir = tmp.0.join("big");
    fs::create_dir_all(dir.join("a/b")).unwrap();
    fs::write(dir.join("a/one.bin"), vec![1u8; 1000]).unwrap();
    fs::write(dir.join("a/b/two.bin"), vec![2u8; 2000]).unwrap();
    let ro = dir.join("a/b/readonly.bin");
    fs::write(&ro, vec![3u8; 300]).unwrap();
    let mut perms = fs::metadata(&ro).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&ro, perms).unwrap();
    let ok = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(dir.join("link"))
        .arg(&outside)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "mklink /J failed");

    let mut files = 0;
    let mut bytes = 0;
    delete_with(&dir, Mode::Permanent, &mut |b| {
        files += 1;
        bytes += b;
    })
    .unwrap();
    assert!(!dir.exists());
    assert_eq!((files, bytes), (3, 3300));
    assert_eq!(fs::read(outside.join("precious.txt")).unwrap(), b"keep me");
}

#[test]
fn run_blocks_protected_items_and_logs_everything() {
    let tmp = TempDir::new("run");
    let keep = tmp.0.join("keep");
    fs::create_dir_all(&keep).unwrap();
    let junk = tmp.0.join("junk.tmp");
    fs::write(&junk, b"junk").unwrap();

    let keep_str = keep.to_string_lossy().into_owned();
    let protector = Protector::new(&[&keep_str], &[]);
    let log = OpLog::new(tmp.0.join("logs"));
    let targets = [
        Target {
            path: keep.clone(),
            size: 0,
        },
        Target {
            path: junk.clone(),
            size: 4,
        },
    ];
    let (outcomes, log_error) = run(&targets, Mode::Permanent, &protector, &log);

    assert!(log_error.is_none());
    assert!(matches!(outcomes[0], Outcome::Blocked(_)));
    assert_eq!(outcomes[1], Outcome::Deleted);
    assert!(keep.exists(), "protected folder must survive");
    assert!(!junk.exists());

    let logged: usize = fs::read_dir(log.dir())
        .unwrap()
        .flatten()
        .map(|f| fs::read_to_string(f.path()).unwrap().lines().count())
        .sum();
    assert_eq!(logged, 2);
}

/// 需要测试卷：`$env:DL_TEST_VOLUME='T'; cargo test -p dl-clean --test delete -- --ignored`
#[cfg(windows)]
#[test]
#[ignore = "需要测试卷，设置 DL_TEST_VOLUME 后用 --ignored 运行"]
fn recycle_moves_to_recycle_bin_on_test_volume() {
    let letter = std::env::var("DL_TEST_VOLUME").expect("set DL_TEST_VOLUME");
    let dir = PathBuf::from(format!(
        r"{letter}:\dl-clean-recycle-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("to-recycle.bin");
    fs::write(&file, vec![1u8; 4096]).unwrap();

    delete(&file, Mode::Recycle).unwrap();
    assert!(!file.exists());

    // 回收站里应该能找到它（$Recycle.Bin 下的 $R 文件）。
    let bin = PathBuf::from(format!(r"{letter}:\$Recycle.Bin"));
    let found = walk_contains_size(&bin, 4096);
    fs::remove_dir_all(&dir).unwrap();
    assert!(found, "file not found in {}", bin.display());
}

#[cfg(windows)]
fn walk_contains_size(dir: &Path, size: u64) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let p = e.path();
        match e.metadata() {
            Ok(m) if m.is_dir() => walk_contains_size(&p, size),
            Ok(m) => m.len() == size && e.file_name().to_string_lossy().starts_with("$R"),
            Err(_) => false,
        }
    })
}
