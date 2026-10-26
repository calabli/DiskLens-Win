//! 分析查询计时：先用普通遍历扫描一个目录，再测各种查询的耗时。
//!
//! ```text
//! cargo run -p dl-analyze --release --example bench -- D:\
//! ```

use std::time::Instant;

use dl_analyze::{Query, breakdown, search};
use dl_tree::Rect;

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let out = f();
    println!(
        "  {label:<36} {:>8.1} ms",
        started.elapsed().as_secs_f64() * 1e3
    );
    out
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| r"D:\".into());
    let started = Instant::now();
    let tree = dl_walk::walk(std::path::Path::new(&path))
        .expect("walk failed")
        .tree;
    println!(
        "{path}：{} 个节点，扫描 {:.1} 秒",
        tree.len(),
        started.elapsed().as_secs_f64()
    );

    let root = tree.root();
    let biggest = tree.children_by_size(root)[0];
    println!(
        "最大的子目录：{}（{} 个文件）",
        tree.name(biggest),
        tree.file_count(biggest)
    );

    let area = Rect {
        x: 0.0,
        y: 0.0,
        w: 900.0,
        h: 700.0,
    };
    time("根目录 children_by_size", || tree.children_by_size(root));
    time("根目录 breakdown", || breakdown(&tree, root));
    time("根目录 treemap", || tree.treemap(root, area, 3_000, 6.0));
    time("最大子目录 breakdown", || breakdown(&tree, biggest));
    time("最大子目录 treemap", || {
        tree.treemap(biggest, area, 3_000, 6.0)
    });
    let q = Query::parse("size:>1gb").unwrap();
    time("全盘搜索 size:>1gb", || search(&tree, root, &q, 500));
    let q = Query::parse("node_modules folder:").unwrap();
    time("全盘搜索 node_modules folder:", || {
        search(&tree, root, &q, 500)
    });
}
