//! NTFS 直读。
//!
//! 打开卷设备（如 `\\.\D:`），按 `$MFT` 自身的 data runs 顺序读出全部文件记录，
//! 解析其中的 `$FILE_NAME` 和 `$DATA` 属性，再从根目录（记录 5）开始组装成
//! [`dl_tree::FileTree`]。不逐个打开目录，所以比普通遍历快一到两个数量级。
//!
//! 打开卷需要管理员权限；失败时调用方应降级为 `dl-walk` 的普通遍历。
//!
//! 模块划分：
//! - [`boot`]：引导扇区，得到簇大小、MFT 位置和记录大小
//! - [`record`]：单条文件记录的 fixup 与属性解析
//! - [`runs`]：非常驻属性的 data runs
//! - [`scan`]：读取整个 MFT 并组装文件树

pub mod boot;
pub mod record;
pub mod runs;
pub mod scan;

use std::fmt;
use std::io;

pub use scan::{MftScan, ReadAt, default_threads, scan_reader, scan_reader_with_threads};
#[cfg(windows)]
pub use scan::{open_volume, scan_volume, scan_volume_with_threads};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// 打开卷被拒绝，通常是因为没有以管理员身份运行。
    AccessDenied,
    NotNtfs,
    Corrupt(&'static str),
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "读取卷失败：{e}"),
            Self::AccessDenied => f.write_str("需要管理员权限才能直接读取卷"),
            Self::NotNtfs => f.write_str("不是 NTFS 卷"),
            Self::Corrupt(what) => write!(f, "MFT 数据异常：{what}"),
            Self::Unsupported(what) => write!(f, "暂不支持：{what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::PermissionDenied {
            Self::AccessDenied
        } else {
            Self::Io(e)
        }
    }
}

// 小端读取。越界一律视为数据损坏，不 panic。

pub(crate) fn le_u16(b: &[u8], off: usize) -> Result<u16> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(Error::Corrupt("truncated field"))
}

pub(crate) fn le_u32(b: &[u8], off: usize) -> Result<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(Error::Corrupt("truncated field"))
}

pub(crate) fn le_u64(b: &[u8], off: usize) -> Result<u64> {
    b.get(off..off + 8)
        .map(|s| u64::from_le_bytes(s.try_into().expect("slice of 8 bytes")))
        .ok_or(Error::Corrupt("truncated field"))
}
