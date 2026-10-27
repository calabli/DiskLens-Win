//! 管理员权限：查询当前是否已提权，以及以管理员身份重新启动。
//!
//! 重新启动时新进程带上 `--wait-pid <旧进程>`：WebView2 的数据目录同一时间只能被一个权限级别
//! 的进程使用，新进程要等旧进程退出后再创建窗口。`--scan <路径>` 让新进程启动后直接扫描。

use tauri::AppHandle;

/// 启动参数里 `--scan` 之后的路径。
#[tauri::command]
pub fn startup_scan() -> Option<String> {
    arg_value("--scan")
}

/// 当前进程是否以管理员身份运行。
#[tauri::command]
pub fn is_elevated() -> bool {
    sys::is_elevated()
}

/// 以管理员身份重新启动，启动后扫描 `scan`（可选）。用户在 UAC 提示里取消时返回错误，程序继续运行。
#[tauri::command]
pub fn restart_as_admin(scan: Option<String>, app: AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut args = format!("--wait-pid {}", std::process::id());
    if let Some(path) = scan {
        // 参数整体放进引号；路径末尾的 `\` 会转义引号，去掉它（`D:\` 写成 `D:`，扫描时再补上）。
        args.push_str(&format!(" --scan \"{}\"", path.trim_end_matches('\\')));
    }
    sys::run_elevated(&exe, &args)?;
    app.exit(0);
    Ok(())
}

/// 启动时调用：如果是重新启动出来的，等旧进程退出（最多 10 秒）。
pub fn wait_for_previous() {
    if let Some(pid) = arg_value("--wait-pid").and_then(|v| v.parse().ok()) {
        sys::wait_for_exit(pid, 10_000);
    }
}

fn arg_value(name: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            let v = args.next()?;
            // `D:` 是重新启动时为了避开引号转义去掉了末尾的 `\`。
            return Some(if v.len() == 2 && v.ends_with(':') {
                v + "\\"
            } else {
                v
            });
        }
    }
    None
}

#[cfg(windows)]
mod sys {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    pub fn is_elevated() -> bool {
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut len = 0;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                (&raw mut elevation).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            );
            CloseHandle(token);
            ok != 0 && elevation.TokenIsElevated != 0
        }
    }

    pub fn run_elevated(exe: &Path, args: &str) -> Result<(), String> {
        let verb = wide("runas".as_ref());
        let file = wide(exe.as_os_str());
        let params = wide(args.as_ref());
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                params.as_ptr(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // 返回值大于 32 表示成功；用户在 UAC 提示里取消时是 ERROR_CANCELLED。
        if result as isize > 32 {
            Ok(())
        } else {
            let e = std::io::Error::last_os_error();
            Err(if e.raw_os_error() == Some(1223) {
                "已取消".into()
            } else {
                format!("无法以管理员身份启动：{e}")
            })
        }
    }

    pub fn wait_for_exit(pid: u32, timeout_ms: u32) {
        unsafe {
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if !process.is_null() {
                WaitForSingleObject(process, timeout_ms);
                CloseHandle(process);
            }
        }
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn is_elevated() -> bool {
        false
    }

    pub fn run_elevated(_: &std::path::Path, _: &str) -> Result<(), String> {
        Err("只在 Windows 上支持".into())
    }

    pub fn wait_for_exit(_: u32, _: u32) {}
}
