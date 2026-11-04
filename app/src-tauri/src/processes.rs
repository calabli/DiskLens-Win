//! 正在运行的进程。清理建议用它判断浏览器等程序是否开着，开着就不清理它的缓存。

use std::collections::HashSet;

/// 正在运行的进程名（例如 `chrome.exe`），全部小写。读取失败时为空。
pub fn running() -> HashSet<String> {
    sys::running()
}

#[cfg(windows)]
mod sys {
    use std::collections::HashSet;

    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    pub fn running() -> HashSet<String> {
        let mut out = HashSet::new();
        // SAFETY: 快照句柄在函数结束前关闭；结构体的 dwSize 按要求先填好。
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut entry);
            while ok != 0 {
                let name = &entry.szExeFile;
                let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                out.insert(String::from_utf16_lossy(&name[..end]).to_lowercase());
                ok = Process32NextW(snap, &mut entry);
            }
            CloseHandle(snap);
        }
        out
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn running() -> std::collections::HashSet<String> {
        std::collections::HashSet::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sees_its_own_process() {
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_string_lossy().to_lowercase();
        assert!(super::running().contains(&name), "{name} not found");
    }
}
