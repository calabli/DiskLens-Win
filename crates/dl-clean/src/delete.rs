//! 删除文件或文件夹。
//!
//! 移到回收站通过 Shell 的 `IFileOperation`，与资源管理器的行为一致，不弹任何对话框，
//! 确认由 DiskLens 自己完成。彻底删除时自己逐个删除文件，以便报告进度：连接点和符号链接
//! 只删链接本身，不进入；只读文件先去掉只读属性再删；最后还删不掉的（例如云盘占位文件）
//! 再交给 `IFileOperation`。每项删除后检查路径是否真的不在了，以此判断成败。

use std::fs::{self, Metadata};
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 移到回收站，可以恢复。
    Recycle,
    /// 彻底删除，无法恢复。
    Permanent,
}

/// 删除一项。失败时返回给用户看的原因。
pub fn delete(path: &Path, mode: Mode) -> Result<(), String> {
    delete_with(path, mode, &mut |_| {})
}

/// 同 [`delete`]，每删掉一个文件以它的字节数调用一次 `on_file`。
/// 移到回收站时不逐个报告，由调用方在整项完成后自己计入。
pub fn delete_with(path: &Path, mode: Mode, on_file: &mut dyn FnMut(u64)) -> Result<(), String> {
    if fs::symlink_metadata(path).is_err() {
        return Err("文件已不存在".into());
    }
    match mode {
        Mode::Recycle => sys::delete(path, mode)?,
        Mode::Permanent => {
            // 自己删不掉的部分交给系统再试一次；失败原因以系统给出的为准。
            if remove_all(path, on_file).is_err() && fs::symlink_metadata(path).is_ok() {
                sys::delete(path, mode)?;
            }
        }
    }
    if fs::symlink_metadata(path).is_ok() {
        return Err("未能删除，文件可能正被其他程序使用".into());
    }
    Ok(())
}

/// 删除时要区分的三种项。
enum Entry {
    /// 普通文件夹：先删里面的内容。
    Dir,
    /// 指向文件夹的连接点或符号链接：只删链接本身。
    DirLink,
    /// 文件，以及指向文件的符号链接。
    File,
}

#[cfg(windows)]
fn entry(meta: &Metadata) -> Entry {
    use std::os::windows::fs::MetadataExt;
    const DIRECTORY: u32 = 0x10;
    const REPARSE_POINT: u32 = 0x400;
    let a = meta.file_attributes();
    match (a & DIRECTORY != 0, a & REPARSE_POINT != 0) {
        (true, false) => Entry::Dir,
        (true, true) => Entry::DirLink,
        _ => Entry::File,
    }
}

#[cfg(not(windows))]
fn entry(meta: &Metadata) -> Entry {
    if meta.file_type().is_symlink() {
        Entry::File
    } else if meta.is_dir() {
        Entry::Dir
    } else {
        Entry::File
    }
}

/// 逐个删除 `path` 和它里面的一切。遇到删不掉的继续删别的，最后返回第一个错误。
fn remove_all(path: &Path, on_file: &mut dyn FnMut(u64)) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    match entry(&meta) {
        Entry::Dir => {
            let mut first = None;
            for e in fs::read_dir(path)? {
                if let Err(err) = e.and_then(|e| remove_all(&e.path(), on_file)) {
                    first.get_or_insert(err);
                }
            }
            if let Some(err) = first {
                return Err(err);
            }
            writable(path, || fs::remove_dir(path))
        }
        Entry::DirLink => fs::remove_dir(path),
        Entry::File => {
            writable(path, || fs::remove_file(path))?;
            on_file(meta.len());
            Ok(())
        }
    }
}

/// 执行 `op`；因为只读而被拒绝时去掉只读属性再试一次。
fn writable(path: &Path, op: impl Fn() -> io::Result<()>) -> io::Result<()> {
    match op() {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            let mut perms = fs::symlink_metadata(path)?.permissions();
            if !perms.readonly() {
                return Err(e);
            }
            // Windows 上只是清除“只读”属性，不会放宽访问权限。
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            fs::set_permissions(path, perms)?;
            op()
        }
        r => r,
    }
}

#[cfg(windows)]
mod sys {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOFX_EARLYFAILURE,
        FOFX_RECYCLEONDELETE, FileOperation, IFileOperation, IShellItem,
        SHCreateItemFromParsingName,
    };
    use windows::core::PCWSTR;

    use super::Mode;

    /// 当前线程的 COM 初始化，离开作用域时反初始化。
    struct Com;

    impl Com {
        fn init() -> Result<Self, String> {
            // SAFETY: 在当前线程初始化 COM；与 Drop 中的 CoUninitialize 成对。
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                .ok()
                .map(|()| Com)
                .map_err(|e| format!("初始化 COM 失败：{e}"))
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            // SAFETY: 只有 CoInitializeEx 成功后才会构造 Com。
            unsafe { CoUninitialize() };
        }
    }

    pub fn delete(path: &Path, mode: Mode) -> Result<(), String> {
        let _com = Com::init()?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();

        // SAFETY: 以下调用都在已初始化 COM 的线程上进行；wide 以 0 结尾且在调用期间有效。
        unsafe {
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)
                .map_err(|e| format!("无法创建文件操作：{e}"))?;
            let mut flags = FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOFX_EARLYFAILURE;
            if mode == Mode::Recycle {
                flags |= FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE;
            }
            op.SetOperationFlags(flags)
                .map_err(|e| format!("设置删除选项失败：{e}"))?;
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
                .map_err(|e| format!("无法打开该路径：{e}"))?;
            op.DeleteItem(&item, None)
                .map_err(|e| format!("无法删除：{e}"))?;
            op.PerformOperations()
                .map_err(|e| format!("删除失败：{e}"))?;
            if op
                .GetAnyOperationsAborted()
                .map_err(|e| e.to_string())?
                .as_bool()
            {
                return Err("删除被中止".into());
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod sys {
    use std::path::Path;

    use super::Mode;

    pub fn delete(path: &Path, mode: Mode) -> Result<(), String> {
        if mode == Mode::Recycle {
            return Err("此平台不支持回收站".into());
        }
        let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        }
        .map_err(|e| e.to_string())
    }
}
