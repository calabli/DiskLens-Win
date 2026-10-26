//! 基准与对比工具。需要在管理员终端里运行：
//!
//! ```text
//! cargo run -p dl-ntfs --release --example scan -- D
//! cargo run -p dl-ntfs --release --example scan -- D --compare
//! cargo run -p dl-ntfs --release --example scan -- D --threads 1
//! ```
//!
//! `--compare` 会再用普通遍历扫一遍同一个卷，对比两种方式得到的大小和文件数。
//! `--threads N` 指定并发读取的线程数，默认按 CPU 核数（最多 8）。

#[cfg(windows)]
fn main() {
    use std::time::Instant;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let letter = args.first().and_then(|s| s.chars().next()).unwrap_or('D');
    let compare = args.iter().any(|a| a == "--compare");
    let threads = args
        .iter()
        .position(|a| a == "--threads")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(dl_ntfs::default_threads);

    let started = Instant::now();
    let scan = match dl_ntfs::scan_volume_with_threads(letter, threads, |_| {}) {
        Ok(scan) => scan,
        Err(e) => {
            eprintln!("MFT 扫描失败：{e}");
            std::process::exit(1);
        }
    };
    let elapsed = started.elapsed();
    let t = &scan.tree;
    let root = t.root();

    println!(
        "卷 {letter}:  MFT 直读用时 {:.2} 秒（{threads} 线程）",
        elapsed.as_secs_f64()
    );
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
    let mib = |b: usize| b as f64 / (1u64 << 20) as f64;
    println!(
        "  内存：文件树 {:.1} MiB（每百万节点 {:.1} MiB），进程峰值 {:.1} MiB",
        mib(t.heap_bytes()),
        mib(t.heap_bytes()) / t.len() as f64 * 1e6,
        mib(peak_working_set())
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

/// 进程至今的峰值工作集（物理内存）字节数。
#[cfg(windows)]
fn peak_working_set() -> usize {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // SAFETY: PROCESS_MEMORY_COUNTERS 是纯数据结构，全零是合法初值。
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: 当前进程的伪句柄总是有效；counters 是大小如实传入的可写结构体。
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    if ok != 0 {
        counters.PeakWorkingSetSize
    } else {
        0
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
