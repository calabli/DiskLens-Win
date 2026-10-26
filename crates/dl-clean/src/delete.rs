//! 删除文件或文件夹。
//!
//! Windows 上通过 Shell 的 `IFileOperation`：与资源管理器的删除行为一致，
//! 能放进回收站，也能处理只读文件和非空文件夹。不弹任何对话框，确认由 DiskLens 自己完成。
//! 每项删除后检查路径是否真的不在了，以此判断成败。

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
    if std::fs::symlink_metadata(path).is_err() {
        return Err("文件已不存在".into());
    }
    sys::delete(path, mode)?;
    if std::fs::symlink_metadata(path).is_ok() {
        return Err("未能删除，文件可能正被其他程序使用".into());
    }
    Ok(())
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
