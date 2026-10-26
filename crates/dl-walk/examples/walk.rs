//! 普通遍历计时：`cargo run -p dl-walk --release --example walk -- D:\some\dir [线程数]`

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().map_or(r"D:\", String::as_str);
    let threads = args
        .get(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(dl_walk::default_threads);

    let started = std::time::Instant::now();
    let result = dl_walk::walk_with_threads(std::path::Path::new(path), threads, &|_| true)
        .expect("walk failed");
    let t = &result.tree;
    println!(
        "{path}：{threads} 线程 {:.2} 秒，{} 个文件，逻辑 {:.2} GiB，占用 {:.2} GiB，无权读取 {}",
        started.elapsed().as_secs_f64(),
        t.file_count(t.root()),
        t.size(t.root()) as f64 / (1u64 << 30) as f64,
        t.allocated(t.root()) as f64 / (1u64 << 30) as f64,
        result.skipped
    );
}
