//! 检查更新、下载并安装：只在用户点“检查更新”时访问 GitHub 查询最新发布的版本，
//! 不在后台自动联网。用户点“下载并安装”后下载安装程序（显示进度），核对大小和 SHA-256，
//! 然后启动安装程序并退出 DiskLens，由安装程序覆盖旧版本。
//!
//! 网络请求用系统自带的 WinHTTP，自动使用系统的代理设置。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;
use tauri::ipc::Channel;

const API: &str = "https://api.github.com/repos/calabli/DiskLens-Win/releases/latest";
/// 只允许打开这个仓库的发布页。
const RELEASES_PAGE: &str = "https://github.com/calabli/DiskLens-Win/releases";
/// 只允许下载这个仓库发布的文件。
const DOWNLOADS: &str = "https://github.com/calabli/DiskLens-Win/releases/download/";
/// 两次推送下载进度之间至少隔这么久。
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// 正在下载时置为 true 即取消。
static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    current: String,
    latest: String,
    /// 最新发布的版本比当前版本新。
    newer: bool,
    /// 发布页地址。
    url: String,
    /// 发布说明（Markdown 原文）。
    notes: String,
    /// 发布时间（RFC 3339）。
    published_at: String,
    /// 安装程序；发布里没有时为空，只能打开发布页手动下载。
    installer: Option<Installer>,
}

/// 发布里的安装程序（`…-setup.exe`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installer {
    name: String,
    url: String,
    size: u64,
    /// SHA-256，十六进制小写；GitHub 没给时为空，只核对大小。
    sha256: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    done: u64,
    total: u64,
}

/// 查询最新发布的版本。
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let mut body = Vec::new();
        sys::get(API, &mut |chunk| {
            body.extend_from_slice(chunk);
            Ok(())
        })
        .map_err(|e| dl_i18n::trf!("无法连接 GitHub：{e}", e = e))?;
        parse(&current, &String::from_utf8_lossy(&body))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 下载安装程序到临时文件夹，过程中推送进度；下载完核对大小和 SHA-256，返回文件路径。
#[tauri::command]
pub async fn download_update(
    installer: Installer,
    on_progress: Channel<DownloadProgress>,
) -> Result<String, String> {
    if !installer.url.starts_with(DOWNLOADS) || !installer.name.ends_with("-setup.exe") {
        return Err(dl_i18n::tr("只能下载 DiskLens 的安装程序").into());
    }
    CANCEL.store(false, Ordering::Relaxed);
    tauri::async_runtime::spawn_blocking(move || {
        let dir = download_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|e| dl_i18n::trf!("无法创建下载文件夹：{e}", e = e))?;
        let path = dir.join(&installer.name);
        let result = fetch(&installer, &path, &on_progress);
        if result.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        result.map(|()| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 取消正在进行的下载。
#[tauri::command]
pub fn cancel_download() {
    CANCEL.store(true, Ordering::Relaxed);
}

/// 启动下载好的安装程序，然后退出 DiskLens，让安装程序可以覆盖旧文件。
#[tauri::command]
pub fn install_update(path: String, app: AppHandle) -> Result<(), String> {
    let path = PathBuf::from(path);
    if path.parent() != Some(download_dir().as_path())
        || path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("exe"))
    {
        return Err(dl_i18n::tr("只能运行下载好的 DiskLens 安装程序").into());
    }
    sys::open(&path)?;
    // 留一点时间让安装程序先启动起来。
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        app.exit(0);
    });
    Ok(())
}

/// 在浏览器里打开发布页。只接受这个仓库的发布页地址。
#[tauri::command]
pub fn open_release_page(url: String) -> Result<(), String> {
    if !url.starts_with(RELEASES_PAGE) {
        return Err(dl_i18n::tr("只能打开 DiskLens 的发布页").into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(&url)
            .spawn()
            .map(|_| ())
            .map_err(|e| dl_i18n::trf!("无法打开浏览器：{e}", e = e))
    }
    #[cfg(not(windows))]
    Err(dl_i18n::tr("只在 Windows 上支持").into())
}

/// 下载的安装程序放在 `%TEMP%\DiskLens-update`。
fn download_dir() -> PathBuf {
    std::env::temp_dir().join("DiskLens-update")
}

/// 下载到 `path`，边下边算 SHA-256，最后核对。
fn fetch(
    installer: &Installer,
    path: &Path,
    on_progress: &Channel<DownloadProgress>,
) -> Result<(), String> {
    let mut file =
        std::fs::File::create(path).map_err(|e| dl_i18n::trf!("无法保存安装程序：{e}", e = e))?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    let mut last: Option<Instant> = None;
    sys::get(&installer.url, &mut |chunk| {
        if CANCEL.load(Ordering::Relaxed) {
            return Err(dl_i18n::tr("已取消").into());
        }
        file.write_all(chunk)
            .map_err(|e| dl_i18n::trf!("无法保存安装程序：{e}", e = e))?;
        hasher.update(chunk);
        done += chunk.len() as u64;
        if last.is_none_or(|l| l.elapsed() >= PROGRESS_EVERY) {
            last = Some(Instant::now());
            let _ = on_progress.send(DownloadProgress {
                done,
                total: installer.size,
            });
        }
        Ok(())
    })
    .map_err(|e| {
        if e == dl_i18n::tr("已取消") {
            e
        } else {
            dl_i18n::trf!("下载失败：{e}", e = e)
        }
    })?;
    file.flush()
        .map_err(|e| dl_i18n::trf!("无法保存安装程序：{e}", e = e))?;
    let _ = on_progress.send(DownloadProgress {
        done,
        total: installer.size,
    });
    if done != installer.size {
        return Err(dl_i18n::trf!(
            "下载的文件大小不对（{done} 字节，应为 {installer} 字节），请重试",
            installer = installer.size,
            done = done
        ));
    }
    if let Some(expected) = &installer.sha256 {
        let actual: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(dl_i18n::tr("下载的文件校验不通过，可能下载过程中出了错，请重试").into());
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
    /// 例如 `sha256:bf00…`。
    #[serde(default)]
    digest: Option<String>,
}

fn parse(current: &str, body: &str) -> Result<UpdateInfo, String> {
    let r: Release = serde_json::from_str(body)
        .map_err(|_| dl_i18n::tr("GitHub 返回的内容看不懂，可能暂时不可用").to_string())?;
    let latest = r.tag_name.trim_start_matches(['v', 'V']).to_string();
    let installer = r
        .assets
        .into_iter()
        .find(|a| {
            a.name.ends_with("_x64-setup.exe") && a.browser_download_url.starts_with(DOWNLOADS)
        })
        .map(|a| Installer {
            sha256: a
                .digest
                .and_then(|d| d.strip_prefix("sha256:").map(str::to_lowercase)),
            name: a.name,
            url: a.browser_download_url,
            size: a.size,
        });
    Ok(UpdateInfo {
        newer: version(&latest) > version(current),
        current: current.to_string(),
        latest,
        url: if r.html_url.starts_with(RELEASES_PAGE) {
            r.html_url
        } else {
            RELEASES_PAGE.to_string()
        },
        notes: r.body.unwrap_or_default(),
        published_at: r.published_at.unwrap_or_default(),
        installer,
    })
}

/// `1.2.3`（可带 `-beta` 之类的后缀，忽略）换算成可比较的三元组；缺的部分当作 0。
fn version(v: &str) -> (u64, u64, u64) {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// `https://host/path` 拆成主机和路径。
fn split_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    let slash = rest.find('/').unwrap_or(rest.len());
    let (host, path) = rest.split_at(slash);
    (!host.is_empty()).then_some((host, if path.is_empty() { "/" } else { path }))
}

#[cfg(windows)]
mod sys {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Networking::WinHttp::{
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
        WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders,
        WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// 关闭 WinHTTP 句柄。
    struct Handle(*mut core::ffi::c_void);

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: 句柄由 WinHTTP 返回，只关闭一次。
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    fn check(h: *mut core::ffi::c_void) -> Result<Handle, String> {
        if h.is_null() {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(Handle(h))
        }
    }

    fn last_error() -> String {
        std::io::Error::last_os_error().to_string()
    }

    /// HTTPS GET `url`（会自动跟随跳转），响应正文分块交给 `sink`；`sink` 返回错误时中止。
    /// 只有最终状态是 200 才算成功。
    pub fn get(url: &str, sink: &mut dyn FnMut(&[u8]) -> Result<(), String>) -> Result<(), String> {
        let (host, path) = super::split_url(url).ok_or(dl_i18n::tr("网址不对"))?;
        let agent = wide(concat!("DiskLens/", env!("CARGO_PKG_VERSION")));
        let host = wide(host);
        let verb = wide("GET");
        let path = wide(path);
        let headers = wide("Accept: application/vnd.github+json\r\n");
        // SAFETY: 所有字符串以 0 结尾，在调用期间有效；句柄用 Handle 在离开作用域时关闭。
        unsafe {
            let session = check(WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            ))?;
            let connect = check(WinHttpConnect(
                session.0,
                host.as_ptr(),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ))?;
            let request = check(WinHttpOpenRequest(
                connect.0,
                verb.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ))?;
            if WinHttpSendRequest(
                request.0,
                headers.as_ptr(),
                u32::MAX,
                std::ptr::null(),
                0,
                0,
                0,
            ) == 0
                || WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0
            {
                return Err(last_error());
            }
            let mut status = 0u32;
            let mut len = size_of::<u32>() as u32;
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                std::ptr::null(),
                (&raw mut status).cast(),
                &mut len,
                std::ptr::null_mut(),
            );
            match status {
                200 => {}
                404 => return Err(dl_i18n::tr("还没有发布过任何版本，或文件已不存在").into()),
                403 | 429 => {
                    return Err(dl_i18n::tr("GitHub 暂时限制了访问次数，请过一会儿再试").into());
                }
                s => return Err(dl_i18n::trf!("GitHub 返回了错误（HTTP {s}）", s = s)),
            }
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let mut available = 0u32;
                if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                    return Err(last_error());
                }
                if available == 0 {
                    return Ok(());
                }
                let want = (available as usize).min(buf.len());
                let mut read = 0u32;
                if WinHttpReadData(request.0, buf.as_mut_ptr().cast(), want as u32, &mut read) == 0
                {
                    return Err(last_error());
                }
                sink(&buf[..read as usize])?;
            }
        }
    }

    /// 用系统默认方式打开文件（安装程序需要管理员权限时会弹出系统确认）。
    pub fn open(path: &Path) -> Result<(), String> {
        let file: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let verb = wide("open");
        // SAFETY: 字符串以 0 结尾且在调用期间有效。
        let r = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if r as isize > 32 {
            Ok(())
        } else {
            Err(dl_i18n::trf!(
                "无法启动安装程序：{last_error}",
                last_error = last_error()
            ))
        }
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn get(_: &str, _: &mut dyn FnMut(&[u8]) -> Result<(), String>) -> Result<(), String> {
        Err(dl_i18n::tr("只在 Windows 上支持").into())
    }

    pub fn open(_: &std::path::Path) -> Result<(), String> {
        Err(dl_i18n::tr("只在 Windows 上支持").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(version("0.2.0") > version("0.1.9"));
        assert!(version("1.0") > version("0.9.9"));
        assert_eq!(version("0.1.0-beta.2"), version("0.1.0"));
        assert_eq!(version("abc"), (0, 0, 0));
    }

    #[test]
    fn reads_the_latest_release_and_its_installer() {
        let body = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/calabli/DiskLens-Win/releases/tag/v0.2.0","body":"新功能","published_at":"2026-12-01T00:00:00Z",
            "assets":[
              {"name":"DiskLens_0.2.0_x64_zh-CN.msi","size":10,"browser_download_url":"https://github.com/calabli/DiskLens-Win/releases/download/v0.2.0/DiskLens_0.2.0_x64_zh-CN.msi"},
              {"name":"DiskLens_0.2.0_x64-setup.exe","size":2284120,"digest":"sha256:BF00","browser_download_url":"https://github.com/calabli/DiskLens-Win/releases/download/v0.2.0/DiskLens_0.2.0_x64-setup.exe"}
            ]}"#;
        let u = parse("0.1.0", body).unwrap();
        assert!(u.newer);
        assert_eq!(u.latest, "0.2.0");
        let i = u.installer.unwrap();
        assert_eq!(i.name, "DiskLens_0.2.0_x64-setup.exe");
        assert_eq!(i.size, 2_284_120);
        assert_eq!(i.sha256.as_deref(), Some("bf00"));

        assert!(!parse("0.2.0", body).unwrap().newer);

        // 不是这个仓库的地址：发布页改用首页，安装程序不认。
        let odd = r#"{"tag_name":"v9","html_url":"https://example.com/x","assets":[
            {"name":"x_x64-setup.exe","size":1,"browser_download_url":"https://example.com/x_x64-setup.exe"}]}"#;
        let o = parse("0.1.0", odd).unwrap();
        assert_eq!(o.url, RELEASES_PAGE);
        assert!(o.installer.is_none());
        assert!(parse("0.1.0", "not json").is_err());
    }

    #[test]
    fn splits_urls() {
        assert_eq!(
            split_url("https://api.github.com/repos/x"),
            Some(("api.github.com", "/repos/x"))
        );
        assert_eq!(split_url("https://example.com"), Some(("example.com", "/")));
        assert_eq!(split_url("http://example.com/x"), None);
    }
}
