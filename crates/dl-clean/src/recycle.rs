//! 回收站：读出回收站里每一项原来的位置和删除时间，并把它放回原处。
//!
//! Windows 把删除到回收站的项目放在它所在磁盘的 `$Recycle.Bin\<用户 SID>` 里，每项两个文件：
//! `$R` 开头的是内容本身（文件或整个文件夹），同名的 `$I` 文件记着原来的路径、大小和删除时间。
//!
//! `$I` 文件格式：版本号 i64（1 是 Vista / 7，2 是 Windows 10 起）、原大小 u64、删除时间
//! FILETIME u64；版本 2 接着是路径长度 u32（字符数，含结尾的 0）和 UTF-16 路径，
//! 版本 1 是固定 260 个 UTF-16 字符的路径。

use std::fs;
use std::path::{Path, PathBuf};

/// 回收站里的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinItem {
    /// 删除前的完整路径。
    pub original: PathBuf,
    pub size: u64,
    /// 删除时间（Unix 秒）。
    pub deleted: u64,
    /// 内容本身（`$R…`）。
    pub data: PathBuf,
    /// 描述文件（`$I…`）。
    pub info: PathBuf,
}

/// FILETIME（1601 年起的 100 纳秒数）换算成 Unix 秒。
fn unix_secs(filetime: u64) -> u64 {
    (filetime / 10_000_000).saturating_sub(11_644_473_600)
}

/// 解析 `$I` 文件，得到（原路径，原大小，删除时间）。格式不认识时返回 `None`。
pub fn parse_info(bytes: &[u8]) -> Option<(PathBuf, u64, u64)> {
    let u64_at = |at: usize| Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?));
    let version = u64_at(0)?;
    let size = u64_at(8)?;
    let deleted = unix_secs(u64_at(16)?);
    let utf16 = |b: &[u8]| -> Vec<u16> {
        b.as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .collect()
    };
    let units = match version {
        1 => utf16(bytes.get(24..24 + 520)?),
        2 => {
            let len = u32::from_le_bytes(bytes.get(24..28)?.try_into().ok()?) as usize;
            utf16(bytes.get(28..28 + len.checked_mul(2)?)?)
        }
        _ => return None,
    };
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    if end == 0 {
        return None;
    }
    Some((
        PathBuf::from(String::from_utf16_lossy(&units[..end])),
        size,
        deleted,
    ))
}

/// 列出磁盘 `volume_root`（例如 `D:\`）回收站里当前用户能看到的所有项目。
/// 其他用户的回收站没有权限读取，会被跳过。
pub fn list(volume_root: &Path) -> Vec<BinItem> {
    let mut out = Vec::new();
    let Ok(users) = fs::read_dir(volume_root.join("$Recycle.Bin")) else {
        return out;
    };
    for user in users.flatten() {
        let Ok(entries) = fs::read_dir(user.path()) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(rest) = name.strip_prefix("$I") else {
                continue;
            };
            let Ok(bytes) = fs::read(e.path()) else {
                continue;
            };
            let Some((original, size, deleted)) = parse_info(&bytes) else {
                continue;
            };
            let data = user.path().join(format!("$R{rest}"));
            if fs::symlink_metadata(&data).is_ok() {
                out.push(BinItem {
                    original,
                    size,
                    deleted,
                    data,
                    info: e.path(),
                });
            }
        }
    }
    out
}

/// 在 `items` 里找日志中的一项：原路径相同（不区分大小写），删除时间与 `time` 相差不超过
/// 10 分钟，取最接近的一个。
pub fn find<'a>(items: &'a [BinItem], path: &str, time: u64) -> Option<&'a BinItem> {
    let path = path.to_lowercase();
    items
        .iter()
        .filter(|i| i.original.to_string_lossy().to_lowercase() == path)
        .map(|i| (i.deleted.abs_diff(time), i))
        .filter(|&(diff, _)| diff <= 600)
        .min_by_key(|&(diff, _)| diff)
        .map(|(_, i)| i)
}

/// 把回收站里的一项放回原处。原来的上级文件夹不在了就先建好；原位置已有同名项目时不覆盖。
pub fn restore(item: &BinItem) -> Result<(), String> {
    if fs::symlink_metadata(&item.original).is_ok() {
        return Err("原位置已经有同名的项目，没有覆盖".into());
    }
    if let Some(parent) = item.original.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法重建原来的文件夹：{e}"))?;
    }
    fs::rename(&item.data, &item.original).map_err(|e| format!("无法移回原处：{e}"))?;
    // 描述文件删不掉也不影响已经放回的内容；回收站里会留下一条打不开的记录。
    let _ = fs::remove_file(&item.info);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info_v2(path: &str, size: u64, filetime: u64) -> Vec<u8> {
        let units: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let mut b = Vec::new();
        b.extend_from_slice(&2u64.to_le_bytes());
        b.extend_from_slice(&size.to_le_bytes());
        b.extend_from_slice(&filetime.to_le_bytes());
        b.extend_from_slice(&(units.len() as u32).to_le_bytes());
        for u in units {
            b.extend_from_slice(&u.to_le_bytes());
        }
        b
    }

    #[test]
    fn parses_both_info_versions() {
        // 2026-11-06T19:33:13Z
        let ft = (1_793_993_593 + 11_644_473_600) * 10_000_000;
        let (p, size, t) = parse_info(&info_v2(r"D:\照片\a.jpg", 1234, ft)).unwrap();
        assert_eq!(p, PathBuf::from(r"D:\照片\a.jpg"));
        assert_eq!(size, 1234);
        assert_eq!(t, 1_793_993_593);

        let mut v1 = Vec::new();
        v1.extend_from_slice(&1u64.to_le_bytes());
        v1.extend_from_slice(&9u64.to_le_bytes());
        v1.extend_from_slice(&ft.to_le_bytes());
        let mut path: Vec<u16> = r"C:\x.txt".encode_utf16().collect();
        path.resize(260, 0);
        for u in path {
            v1.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_info(&v1).unwrap().0, PathBuf::from(r"C:\x.txt"));

        assert!(parse_info(&[0u8; 10]).is_none());
        assert!(parse_info(&info_v2("", 0, ft)).is_none());
    }

    #[test]
    fn finds_the_closest_matching_item() {
        let item = |path: &str, deleted: u64| BinItem {
            original: PathBuf::from(path),
            size: 1,
            deleted,
            data: PathBuf::new(),
            info: PathBuf::new(),
        };
        let items = [
            item(r"D:\a.txt", 1_000),
            item(r"D:\A.TXT", 5_000),
            item(r"D:\b.txt", 1_000),
        ];
        assert_eq!(find(&items, r"d:\a.txt", 1_020).unwrap().deleted, 1_000);
        assert_eq!(find(&items, r"D:\a.txt", 4_900).unwrap().deleted, 5_000);
        assert!(find(&items, r"D:\a.txt", 3_000).is_none());
        assert!(find(&items, r"D:\c.txt", 1_000).is_none());
    }

    #[test]
    fn restores_into_a_recreated_folder_and_never_overwrites() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .join("target")
            .join("test-tmp")
            .join(format!("dl-clean-recycle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("$RABC.txt"), b"hello").unwrap();
        fs::write(bin.join("$IABC.txt"), b"info").unwrap();
        let item = BinItem {
            original: dir.join("gone").join("deeper").join("a.txt"),
            size: 5,
            deleted: 0,
            data: bin.join("$RABC.txt"),
            info: bin.join("$IABC.txt"),
        };
        restore(&item).unwrap();
        assert_eq!(fs::read(&item.original).unwrap(), b"hello");
        assert!(!item.info.exists());

        // 原位置已经有东西时不覆盖。
        fs::write(bin.join("$RDEF.txt"), b"other").unwrap();
        let clash = BinItem {
            data: bin.join("$RDEF.txt"),
            info: bin.join("$IDEF.txt"),
            ..item.clone()
        };
        assert!(restore(&clash).is_err());
        assert_eq!(fs::read(&item.original).unwrap(), b"hello");
        fs::remove_dir_all(&dir).unwrap();
    }
}
