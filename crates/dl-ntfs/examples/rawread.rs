//! 诊断：逐步检查能否打开并读取卷设备。`cargo run -p dl-ntfs --example rawread -- D`

#[cfg(windows)]
fn main() {
    use dl_ntfs::ReadAt;

    let letter = std::env::args()
        .nth(1)
        .and_then(|s| s.chars().next())
        .unwrap_or('D');
    let vol = match dl_ntfs::open_volume(letter) {
        Ok(v) => {
            println!("open \\\\.\\{letter}: ok");
            v
        }
        Err(e) => {
            println!("open failed: {e}");
            return;
        }
    };
    for (len, off) in [(512usize, 0u64), (4096, 0), (4096, 4096), (1 << 20, 0)] {
        let mut buf = vec![0u8; len];
        match vol.read_exact_at(&mut buf, off) {
            Ok(()) => println!(
                "read {len} @ {off}: ok, oem={:?}",
                String::from_utf8_lossy(&buf[3..11])
            ),
            Err(e) => println!("read {len} @ {off}: {e} ({:?})", e.raw_os_error()),
        }
    }
}

#[cfg(not(windows))]
fn main() {}
