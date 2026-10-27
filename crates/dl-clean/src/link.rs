//! 用硬链接替换重复文件：把 `target` 换成 `source` 的另一个名称，两处共用一份数据，
//! 释放 `target` 原来占用的空间。
//!
//! `target` 原有的数据不会进回收站，所以替换前逐字节核对两者内容，任何不同都不替换。
//! 替换过程：先在 `target` 旁边建一个指向 `source` 的临时名称，再改名覆盖 `target`；
//! 中途失败时删掉临时名称，`target` 保持原样。

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// 逐字节比较时每次读取的字节数。
const CHUNK: usize = 1 << 20;

/// 文件在卷上的身份：卷序列号加文件 id。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    volume: u64,
    index: u64,
}

/// 把 `target` 替换为指向 `source` 的硬链接。两者已经是同一个文件时什么也不做，返回 `false`。
pub fn replace_with_hardlink(source: &Path, target: &Path) -> Result<bool, String> {
    let (src_meta, src_id) = inspect(source).map_err(|e| format!("无法读取保留的那份：{e}"))?;
    let (dst_meta, dst_id) = inspect(target).map_err(|e| format!("无法读取：{e}"))?;
    if src_id == dst_id {
        return Ok(false);
    }
    if src_id.volume != dst_id.volume {
        return Err("两个文件不在同一个磁盘上，不能用硬链接".into());
    }
    if src_meta.len() != dst_meta.len() {
        return Err("大小已经不同，可能在查重后被修改过，没有替换".into());
    }
    if !same_content(source, target).map_err(|e| format!("核对内容时出错：{e}"))? {
        return Err("内容已经不同，可能在查重后被修改过，没有替换".into());
    }

    let tmp = temp_name(target);
    fs::hard_link(source, &tmp).map_err(|e| format!("无法创建硬链接：{e}"))?;
    let result = (|| {
        // 只读文件不能被改名覆盖。
        if dst_meta.permissions().readonly() {
            let mut p = dst_meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            p.set_readonly(false);
            fs::set_permissions(target, p)?;
        }
        fs::rename(&tmp, target)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(format!("替换失败：{e}"));
    }
    match inspect(target) {
        Ok((_, id)) if id == src_id => Ok(true),
        _ => Err("替换后核对失败".into()),
    }
}

fn inspect(path: &Path) -> io::Result<(fs::Metadata, Identity)> {
    let file = File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "不是文件"));
    }
    Ok((meta.clone(), sys::identity(&file, &meta)?))
}

/// 逐字节比较两个文件。
fn same_content(a: &Path, b: &Path) -> io::Result<bool> {
    let (mut fa, mut fb) = (File::open(a)?, File::open(b)?);
    let (mut ba, mut bb) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
    loop {
        let na = read_full(&mut fa, &mut ba)?;
        let nb = read_full(&mut fb, &mut bb)?;
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
    }
}

/// 尽量读满缓冲区，只有到文件末尾时才少于缓冲区大小。
fn read_full(f: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// `target` 同目录下的一个临时名称。
fn temp_name(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    target.with_file_name(format!(".{name}.dl{}-{nanos}.tmp", std::process::id()))
}

#[cfg(windows)]
mod sys {
    use std::fs::{File, Metadata};
    use std::io;
    use std::os::windows::io::AsRawHandle;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    use super::Identity;

    pub fn identity(file: &File, _: &Metadata) -> io::Result<Identity> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: 句柄在 `file` 存活期间有效；`info` 是有效的输出缓冲区。
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(io::Error::other)?;
        Ok(Identity {
            volume: u64::from(info.dwVolumeSerialNumber),
            index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        })
    }
}

#[cfg(not(windows))]
mod sys {
    use std::fs::{File, Metadata};
    use std::io;
    use std::os::unix::fs::MetadataExt;

    use super::Identity;

    pub fn identity(_: &File, meta: &Metadata) -> io::Result<Identity> {
        Ok(Identity {
            volume: meta.dev(),
            index: meta.ino(),
        })
    }
}
