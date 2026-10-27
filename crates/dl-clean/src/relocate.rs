//! 把文件夹移到另一个磁盘，并在原位置留下指向新位置的连接点（junction），
//! 依赖原路径的程序照常工作。
//!
//! 移动的是真实数据，所以每一步都可以撤回：
//!
//! 1. 检查：源是普通文件夹、里面没有链接；目标在另一个本地磁盘上、还不存在、空间足够。
//! 2. 把源文件夹改名（同一磁盘上，瞬间完成）。有文件正被使用时这一步就会失败，什么也没改。
//! 3. 复制到目标，逐个核对文件大小和数量。
//! 4. 在原位置建连接点并确认能打开。
//! 5. 删除改名后的旧文件夹。
//!
//! 第 2–4 步任何一步失败或被取消，都会删掉目标里的副本、把源文件夹改回原名。

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};

/// 目标磁盘至少要多留出这么多空间。
const HEADROOM: u64 = 64 << 20;

/// 一次移动的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub bytes: u64,
    pub files: u64,
    /// 旧文件夹没能完全删除时，它现在的位置（数据已在目标，可以手动删除）。
    pub leftover: Option<PathBuf>,
}

/// 源文件夹的清单：相对路径到文件大小。
struct Survey {
    files: HashMap<PathBuf, u64>,
    dirs: Vec<PathBuf>,
    bytes: u64,
}

/// 把 `src` 移到 `dest`（`dest` 是移动后文件夹的完整路径），并在 `src` 留下连接点。
/// `progress(已复制字节, 总字节)` 返回 `false` 即取消（会撤回）。
pub fn move_with_junction(
    src: &Path,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<Moved, String> {
    let survey = check(src, dest)?;

    // 第 2 步：改名。失败说明有文件正被使用。
    let old = sibling(src, "dl-moving");
    fs::rename(src, &old).map_err(|e| format!("文件夹里有文件正被使用，没有做任何改动（{e}）"))?;

    let result = copy_verify_link(&old, src, dest, &survey, progress);
    if let Err(e) = result {
        // 撤回：去掉可能已建的连接点，改回原名，删掉目标里的副本。
        if fs::symlink_metadata(src).is_ok() {
            let _ = fs::remove_dir(src);
        }
        let restored = fs::rename(&old, src);
        let _ = remove_tree(dest);
        return Err(match restored {
            Ok(()) => e,
            Err(r) => format!(
                "{e}；而且没能把原文件夹改回原名，它现在在 {}（{r}）",
                old.display()
            ),
        });
    }

    let leftover = remove_tree(&old).err().map(|_| old);
    Ok(Moved {
        bytes: survey.bytes,
        files: survey.files.len() as u64,
        leftover,
    })
}

/// 第 1 步：检查能不能移动，并列出源文件夹的内容。
fn check(src: &Path, dest: &Path) -> Result<Survey, String> {
    let meta = fs::symlink_metadata(src).map_err(|e| format!("无法读取源文件夹：{e}"))?;
    if meta.is_symlink() || !meta.is_dir() {
        return Err("只能移动普通文件夹（不能是链接或文件）".into());
    }
    let (Some(from), Some(to)) = (drive(src), drive(dest)) else {
        return Err("源和目标都必须是本机磁盘上的完整路径".into());
    };
    if src.parent().is_none() {
        return Err("不能移动整个磁盘".into());
    }
    if from == to {
        return Err("目标在同一个磁盘上，移动不会释放空间".into());
    }
    if fs::symlink_metadata(dest).is_ok() {
        return Err(format!("目标位置已经存在：{}", dest.display()));
    }
    let survey = survey(src)?;
    let free = sys::free_space(dest).map_err(|e| format!("无法读取目标磁盘的可用空间：{e}"))?;
    if free < survey.bytes + HEADROOM {
        return Err(format!(
            "目标磁盘空间不足：需要约 {} MB，只有 {} MB",
            (survey.bytes + HEADROOM) >> 20,
            free >> 20
        ));
    }
    Ok(survey)
}

fn survey(root: &Path) -> Result<Survey, String> {
    let mut s = Survey {
        files: HashMap::new(),
        dirs: Vec::new(),
        bytes: 0,
    };
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let entries = fs::read_dir(root.join(&rel))
            .map_err(|e| format!("无法读取 {}：{e}", rel.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            let path = rel.join(entry.file_name());
            if meta.is_symlink() {
                return Err(format!(
                    "文件夹里有链接或连接点（{}），暂不支持移动",
                    path.display()
                ));
            }
            if meta.is_dir() {
                s.dirs.push(path.clone());
                stack.push(path);
            } else {
                s.bytes += meta.len();
                s.files.insert(path, meta.len());
            }
        }
    }
    Ok(s)
}

/// 第 3、4 步：复制、核对、建连接点。
fn copy_verify_link(
    from: &Path,
    link: &Path,
    dest: &Path,
    expected: &Survey,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("无法创建目标文件夹：{e}"))?;
    for d in &expected.dirs {
        fs::create_dir_all(dest.join(d)).map_err(|e| format!("无法创建 {}：{e}", d.display()))?;
    }
    let mut copied = 0;
    if !progress(0, expected.bytes) {
        return Err("已取消".into());
    }
    for (rel, &size) in &expected.files {
        fs::copy(from.join(rel), dest.join(rel))
            .map_err(|e| format!("复制 {} 失败：{e}", rel.display()))?;
        copied += size;
        if !progress(copied, expected.bytes) {
            return Err("已取消".into());
        }
    }

    // 核对：每个文件都在、大小一致，没有多出来的文件。
    let copy = survey(dest)?;
    if copy.files != expected.files {
        return Err("复制后核对不一致，已撤回".into());
    }

    make_junction(link, dest).map_err(|e| format!("无法建立连接点：{e}"))?;
    let opened = fs::read_dir(link).map(|d| d.count()).ok();
    let listed = fs::read_dir(dest).map(|d| d.count()).ok();
    if opened.is_none() || opened != listed {
        return Err("连接点建立后无法正常打开，已撤回".into());
    }
    Ok(())
}

/// 删除一棵普通目录树（里面不应有链接）；只读文件先去掉只读属性。
fn remove_tree(root: &Path) -> io::Result<()> {
    if fs::symlink_metadata(root).is_err() {
        return Ok(());
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_dir() && !meta.is_symlink() {
                stack.push(entry.path());
            } else if meta.permissions().readonly() {
                let mut p = meta.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                p.set_readonly(false);
                fs::set_permissions(entry.path(), p)?;
            }
        }
    }
    fs::remove_dir_all(root)
}

/// `path` 旁边一个不存在的新名称：`名称.tag-进程号`。
fn sibling(path: &Path, tag: &str) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    path.with_file_name(format!("{name}.{tag}-{}", std::process::id()))
}

/// 路径的盘符（大写）；不是 `X:\…` 形式时为 `None`。
fn drive(path: &Path) -> Option<char> {
    match path.components().next()? {
        Component::Prefix(p) => match p.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Some((d as char).to_ascii_uppercase()),
            _ => None,
        },
        _ => None,
    }
}

/// 在 `link` 建一个指向 `target` 的连接点。`link` 必须还不存在。
pub fn make_junction(link: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir(link)?;
    let result = sys::set_mount_point(link, target);
    if result.is_err() {
        let _ = fs::remove_dir(link);
    }
    result
}

#[cfg(windows)]
mod sys {
    use std::fs::OpenOptions;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;
    use windows::core::PCWSTR;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

    /// 去掉 `\\?\` 前缀的绝对路径。
    fn plain(path: &Path) -> io::Result<String> {
        let abs = std::path::absolute(path)?;
        let s = abs.to_string_lossy();
        Ok(s.strip_prefix(r"\\?\").unwrap_or(&s).to_string())
    }

    /// 写入挂载点类型的重解析数据（REPARSE_DATA_BUFFER）。
    pub fn set_mount_point(link: &Path, target: &Path) -> io::Result<()> {
        let target = plain(target)?;
        let substitute: Vec<u16> = format!(r"\??\{target}").encode_utf16().collect();
        let print: Vec<u16> = target.encode_utf16().collect();
        let sub_bytes = substitute.len() * 2;
        let print_bytes = print.len() * 2;
        let data_len = 8 + sub_bytes + 2 + print_bytes + 2;
        let too_long = || io::Error::new(io::ErrorKind::InvalidInput, "路径太长");
        let u16_of = |n: usize| u16::try_from(n).map_err(|_| too_long());

        let mut buf = Vec::with_capacity(8 + data_len);
        buf.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
        buf.extend_from_slice(&u16_of(data_len)?.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // SubstituteNameOffset
        buf.extend_from_slice(&u16_of(sub_bytes)?.to_le_bytes());
        buf.extend_from_slice(&u16_of(sub_bytes + 2)?.to_le_bytes()); // PrintNameOffset
        buf.extend_from_slice(&u16_of(print_bytes)?.to_le_bytes());
        for u in substitute.iter().chain([&0]).chain(&print).chain([&0]) {
            buf.extend_from_slice(&u.to_le_bytes());
        }

        let dir = OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(link)?;
        let mut returned = 0u32;
        // SAFETY: 句柄在 `dir` 存活期间有效；输入缓冲区按 REPARSE_DATA_BUFFER 的格式构造，长度如实传入。
        unsafe {
            DeviceIoControl(
                HANDLE(dir.as_raw_handle()),
                FSCTL_SET_REPARSE_POINT,
                Some(buf.as_ptr().cast()),
                buf.len() as u32,
                None,
                0,
                Some(&mut returned),
                None,
            )
        }
        .map_err(io::Error::other)
    }

    /// `path` 所在磁盘的可用空间（`path` 本身可以还不存在）。
    pub fn free_space(path: &Path) -> io::Result<u64> {
        let mut dir = path;
        while !dir.exists() {
            dir = dir
                .parent()
                .ok_or_else(|| io::Error::other("找不到目标磁盘"))?;
        }
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
        let mut free = 0u64;
        // SAFETY: `wide` 以 0 结尾；输出参数指向有效的 u64。
        unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }
            .map_err(io::Error::other)?;
        Ok(free)
    }
}

#[cfg(not(windows))]
mod sys {
    use std::io;
    use std::path::Path;

    pub fn set_mount_point(link: &Path, target: &Path) -> io::Result<()> {
        std::fs::remove_dir(link)?;
        std::os::unix::fs::symlink(target, link)
    }

    pub fn free_space(_: &Path) -> io::Result<u64> {
        Ok(u64::MAX)
    }
}
