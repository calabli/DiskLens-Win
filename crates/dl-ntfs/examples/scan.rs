//! 基准与对比工具。需要在管理员终端里运行：
//!
//! ```text
//! cargo run -p dl-ntfs --release --example scan -- D
//! cargo run -p dl-ntfs --release --example scan -- D --compare
//! ```
//!
//! `--compare` 会再用普通遍历扫一遍同一个卷，对比两种方式得到的大小和文件数。

#[cfg(windows)]
fn main() {
    use std::time::Instant;

    let mut args = std::env::args().skip(1);
    let letter = args.next().and_then(|s| s.chars().next()).unwrap_or('D');
    let compare = args.any(|a| a == "--compare");

    let started = Instant::now();
    let scan = match dl_ntfs::scan_volume(letter, |_| {}) {
        Ok(scan) => scan,
        Err(e) => {
            eprintln!("MFT 扫描失败：{e}");
            std::process::exit(1);
        }
    };
    let elapsed = started.elapsed();
    let t = &scan.tree;
    let root = t.root();

    println!("卷 {letter}:  MFT 直读用时 {:.2} 秒", elapsed.as_secs_f64());
    println!(
        "  记录 {} 条（簇 {} B，记录 {} B），损坏 {}，孤立 {}",
        scan.records_in_use,
        scan.cluster_size,
        scan.record_size,
        scan.corrupt_records,
        scan.orphans
    );
    println!(
        "  节点 {}，文件 {}，逻辑大小 {}，占用空间 {}",
        t.len(),
        t.file_count(root),
        gib(t.size(root)),
        gib(t.allocated(root))
    );
    println!("  最大的 10 项：");
    for id in t.children_by_size(root).into_iter().take(10) {
        println!("    {:>12}  {}", gib(t.size(id)), t.name(id));
    }

    if compare {
        let path = format!(r"{}:\", letter.to_ascii_uppercase());
        let started = Instant::now();
        let walk = dl_walk::walk(std::path::Path::new(&path)).expect("普通遍历失败");
        let w = &walk.tree;
        println!(
            "普通遍历用时 {:.2} 秒，无权读取 {} 个目录",
            started.elapsed().as_secs_f64(),
            walk.skipped
        );
        println!("  {:<40} {:>14} {:>14}", "顶层项目", "MFT", "普通遍历");
        for id in t.children_by_size(root).into_iter().take(20) {
            let name = t.name(id);
            let other = w.child_by_name(w.root(), name).map(|c| w.size(c));
            let mark = if other == Some(t.size(id)) {
                ""
            } else {
                "  ← 不同"
            };
            println!(
                "  {:<40} {:>14} {:>14}{mark}",
                name,
                t.size(id),
                other.map_or("-".into(), |s| s.to_string())
            );
        }
    }
}

#[cfg(windows)]
fn gib(bytes: u64) -> String {
    format!("{:.2} GiB", bytes as f64 / (1u64 << 30) as f64)
}

#[cfg(not(windows))]
fn main() {
    eprintln!("仅支持 Windows");
}
