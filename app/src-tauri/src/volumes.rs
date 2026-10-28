//! 列出本机的卷（盘符、卷标、文件系统、容量）。

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    /// 例如 `D:\`。
    pub root: String,
    pub label: String,
    pub file_system: String,
    pub total: u64,
    pub free: u64,
}

#[tauri::command]
pub fn list_volumes() -> Vec<Volume> {
    sys::list()
}

/// 根目录为 `root`（如 `D:\`）的卷。
pub fn find(root: &str) -> Option<Volume> {
    sys::list()
        .into_iter()
        .find(|v| v.root.eq_ignore_ascii_case(root))
}

#[cfg(windows)]
mod sys {
    use super::Volume;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };

    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn from_wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    /// 本地硬盘和可移动盘；光驱、网络映射盘不列出。
    pub fn list() -> Vec<Volume> {
        // SAFETY: GetLogicalDrives 没有参数和前置条件。
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .filter_map(|i| {
                let root = format!("{}:\\", char::from(b'A' + i));
                let w = wide(&root);
                // SAFETY: w 以 0 结尾，在调用期间有效。
                let kind = unsafe { GetDriveTypeW(w.as_ptr()) };
                if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE {
                    return None;
                }
                let mut label = [0u16; 261];
                let mut fs = [0u16; 261];
                // SAFETY: 两个输出缓冲区的长度如实传入；不需要的输出传空指针。
                let ok = unsafe {
                    GetVolumeInformationW(
                        w.as_ptr(),
                        label.as_mut_ptr(),
                        label.len() as u32,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        fs.as_mut_ptr(),
                        fs.len() as u32,
                    )
                };
                if ok == 0 {
                    return None; // 例如没插卡的读卡器
                }
                let (mut free, mut total) = (0u64, 0u64);
                // SAFETY: 输出参数都是有效的可写指针。
                let ok = unsafe {
                    GetDiskFreeSpaceExW(w.as_ptr(), &mut free, &mut total, std::ptr::null_mut())
                };
                (ok != 0).then(|| Volume {
                    root,
                    label: from_wide(&label),
                    file_system: from_wide(&fs),
                    total,
                    free,
                })
            })
            .collect()
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn list() -> Vec<super::Volume> {
        Vec::new()
    }
}
