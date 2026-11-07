//! 检查更新：只在用户点“检查更新”时访问 GitHub，查询最新发布的版本，不在后台自动联网，
//! 也不自动下载安装。有新版本时由用户在浏览器里打开下载页。
//!
//! 网络请求用系统自带的 WinHTTP，自动使用系统的代理设置。

use serde::Serialize;
use tauri::AppHandle;

const HOST: &str = "api.github.com";
const LATEST: &str = "/repos/calabli/DiskLens-Win/releases/latest";
/// 只允许打开这个仓库的发布页。
const RELEASES_PAGE: &str = "https://github.com/calabli/DiskLens-Win/releases";

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
}

/// 查询最新发布的版本。
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let body = sys::get(HOST, LATEST).map_err(|e| format!("无法连接 GitHub：{e}"))?;
        parse(&current, &body)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 在浏览器里打开发布页。只接受这个仓库的发布页地址。
#[tauri::command]
pub fn open_release_page(url: String) -> Result<(), String> {
    if !url.starts_with(RELEASES_PAGE) {
        return Err("只能打开 DiskLens 的发布页".into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(&url)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("无法打开浏览器：{e}"))
    }
    #[cfg(not(windows))]
    Err("只在 Windows 上支持".into())
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
}

fn parse(current: &str, body: &str) -> Result<UpdateInfo, String> {
    let r: Release = serde_json::from_str(body)
        .map_err(|_| "GitHub 返回的内容看不懂，可能暂时不可用".to_string())?;
    let latest = r.tag_name.trim_start_matches(['v', 'V']).to_string();
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

#[cfg(windows)]
mod sys {
    use windows_sys::Win32::Networking::WinHttp::{
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
        WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders,
        WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    };

    /// 返回体最多读这么多字节。
    const MAX_BODY: usize = 4 << 20;

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

    fn check(h: *mut core::ffi::c_void) -> std::io::Result<Handle> {
        if h.is_null() {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(Handle(h))
        }
    }

    /// HTTPS GET `https://{host}{path}`，返回 200 时的响应正文。
    pub fn get(host: &str, path: &str) -> Result<String, String> {
        let io = |e: std::io::Error| e.to_string();
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
            ))
            .map_err(io)?;
            let connect = check(WinHttpConnect(
                session.0,
                host.as_ptr(),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ))
            .map_err(io)?;
            let request = check(WinHttpOpenRequest(
                connect.0,
                verb.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ))
            .map_err(io)?;
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
                return Err(std::io::Error::last_os_error().to_string());
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
            let mut body = Vec::new();
            loop {
                let mut available = 0u32;
                if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                if available == 0 || body.len() > MAX_BODY {
                    break;
                }
                let start = body.len();
                body.resize(start + available as usize, 0);
                let mut read = 0u32;
                if WinHttpReadData(
                    request.0,
                    body[start..].as_mut_ptr().cast(),
                    available,
                    &mut read,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                body.truncate(start + read as usize);
            }
            match status {
                200 => Ok(String::from_utf8_lossy(&body).into_owned()),
                404 => Err("还没有发布过任何版本".into()),
                403 | 429 => Err("GitHub 暂时限制了访问次数，请过一会儿再试".into()),
                s => Err(format!("GitHub 返回了错误（HTTP {s}）")),
            }
        }
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn get(_: &str, _: &str) -> Result<String, String> {
        Err("只在 Windows 上支持".into())
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
    fn reads_the_latest_release() {
        let body = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/calabli/DiskLens-Win/releases/tag/v0.2.0","body":"新功能","published_at":"2026-12-01T00:00:00Z"}"#;
        let u = parse("0.1.0", body).unwrap();
        assert!(u.newer);
        assert_eq!(u.latest, "0.2.0");
        assert_eq!(u.notes, "新功能");

        let same = parse("0.2.0", body).unwrap();
        assert!(!same.newer);

        // 地址不是这个仓库的发布页时，改用发布页首页。
        let odd = r#"{"tag_name":"v9","html_url":"https://example.com/x"}"#;
        assert_eq!(parse("0.1.0", odd).unwrap().url, RELEASES_PAGE);
        assert!(parse("0.1.0", "not json").is_err());
    }
}
