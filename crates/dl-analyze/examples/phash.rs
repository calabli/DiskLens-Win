//! 计时：对一个文件夹里的图片计算感知哈希并分组。
//!
//! ```text
//! cargo run --release -p dl-analyze --example phash -- D:\壁纸
//! ```

use std::path::PathBuf;
use std::time::Instant;

use dl_analyze::similar::{PhashCache, candidates, find_similar};

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: phash <folder>"));
    let tree = dl_walk::walk(&dir).expect("walk").tree;
    let cands = candidates(&tree, tree.root(), 20 << 10);
    let bytes: u64 = cands.iter().map(|c| c.size).sum();
    let started = Instant::now();
    let report = find_similar(&cands, &PhashCache::default(), 8, 8, &|_, _| true).expect("find");
    let secs = started.elapsed().as_secs_f64();
    println!(
        "{} 张图片（{:.1} GB），{} 组相似，{} 张无法读取，用时 {secs:.1} 秒（每张 {:.0} 毫秒）",
        cands.len(),
        bytes as f64 / 1e9,
        report.groups.len(),
        report.skipped,
        secs * 1000.0 / cands.len().max(1) as f64
    );
    for g in report.groups.iter().take(10) {
        let names: Vec<_> = g
            .files
            .iter()
            .map(|f| format!("{} ({}x{})", f.path.display(), f.width, f.height))
            .collect();
        println!("  {}", names.join("  ≈  "));
    }
}
