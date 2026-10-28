//! 存储详情：整个磁盘的用量，以及扫描结果里 Windows 自己占用的那些位置（页面文件、休眠文件、
//! 还原点、组件存储等）。说明文字在前端，这里只按 `key` 给出找到的位置和大小。

use dl_tree::{FileTree, NodeId};
use serde::Serialize;
use tauri::State;

use crate::scan::AppState;
use crate::volumes::{self, Volume};

/// 系统占用的各类位置，路径相对于卷根目录。同一类可以有多个位置。
const SYSTEM_ITEMS: &[(&str, &[&str])] = &[
    ("pagefile", &["pagefile.sys", "swapfile.sys"]),
    ("hiberfil", &["hiberfil.sys"]),
    ("restore", &["System Volume Information"]),
    ("winsxs", &[r"Windows\WinSxS"]),
    ("installer", &[r"Windows\Installer"]),
    (
        "update",
        &[
            r"Windows\SoftwareDistribution\Download",
            r"Windows\ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache",
        ],
    ),
    (
        "windowsOld",
        &["Windows.old", "$Windows.~BT", "$Windows.~WS"],
    ),
    (
        "dumps",
        &[
            r"Windows\MEMORY.DMP",
            r"Windows\Minidump",
            r"Windows\LiveKernelReports",
        ],
    ),
    ("temp", &[r"Windows\Temp"]),
    ("recycle", &["$Recycle.Bin"]),
    (
        "ntfs",
        &[
            "$MFT", "$MFTMirr", "$LogFile", "$Volume", "$AttrDef", "$Bitmap", "$Boot", "$BadClus",
            "$Secure", "$UpCase", "$Extend",
        ],
    ),
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDetails {
    /// 扫描范围所在的卷；读不到卷信息时为空。
    pub volume: Option<Volume>,
    /// 扫描范围的占用空间。
    pub scanned: u64,
    /// 扫描的是整个卷（根目录），这时“已用 − 扫描到的”就是没算进去的部分。
    pub whole_volume: bool,
    /// 扫描结果里找到的系统位置，按占用从大到小。
    pub items: Vec<SystemItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemItem {
    pub key: &'static str,
    pub allocated: u64,
    pub places: Vec<Place>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub id: NodeId,
    pub path: String,
    pub allocated: u64,
}

#[tauri::command]
pub fn storage_details(state: State<'_, AppState>) -> Result<StorageDetails, String> {
    let scan = state.current()?;
    let t = scan.read();
    let root = t.root();
    let base = t.path(root);
    let volume_root = volume_root(&base);
    let whole_volume = volume_root.as_deref().is_some_and(|v| {
        base.trim_end_matches('\\')
            .eq_ignore_ascii_case(v.trim_end_matches('\\'))
    });
    let volume = volume_root.and_then(|v| volumes::find(&v));
    let items = if whole_volume {
        find_items(&t)
    } else {
        Vec::new()
    };
    Ok(StorageDetails {
        volume,
        scanned: t.allocated(root),
        whole_volume,
        items,
    })
}

/// 在 Windows 存储设置里打开“存储”页面。
#[tauri::command]
pub fn open_storage_settings() -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg("ms-settings:storagesense")
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("无法打开设置：{e}"))
    }
    #[cfg(not(windows))]
    Err("只在 Windows 上支持".into())
}

/// `D:\foo` → `D:\`；不是盘符路径（例如网络路径）时为 `None`。
fn volume_root(path: &str) -> Option<String> {
    let b = path.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        .then(|| format!("{}:\\", char::from(b[0]).to_ascii_uppercase()))
}

/// 树根就是卷根目录时，找出各类系统位置。
fn find_items(t: &FileTree) -> Vec<SystemItem> {
    let mut items: Vec<SystemItem> = SYSTEM_ITEMS
        .iter()
        .filter_map(|&(key, paths)| {
            let places: Vec<Place> = paths
                .iter()
                .filter_map(|p| find_path(t, t.root(), p))
                .map(|id| Place {
                    id,
                    path: t.path(id),
                    allocated: t.allocated(id),
                })
                // 还原点文件夹不以管理员身份运行时读不到，大小是 0，也要列出来提醒。
                .filter(|p| p.allocated > 0 || key == "restore")
                .collect();
            (!places.is_empty()).then(|| SystemItem {
                key,
                allocated: places.iter().map(|p| p.allocated).sum(),
                places,
            })
        })
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.allocated));
    items
}

/// 从 `from` 出发按相对路径逐级查找，名称不区分大小写。
pub fn find_path(t: &FileTree, from: NodeId, rel: &str) -> Option<NodeId> {
    rel.split('\\')
        .filter(|p| !p.is_empty())
        .try_fold(from, |id, part| {
            t.children(id)
                .find(|&c| t.name(c).eq_ignore_ascii_case(part))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_system_places_case_insensitively() {
        let mut t = FileTree::new(r"C:\");
        t.add_file(0, "pagefile.sys", 4 << 30, 4 << 30);
        t.add_file(0, "swapfile.sys", 16 << 20, 16 << 20);
        let win = t.add_dir(0, "WINDOWS");
        let sxs = t.add_dir(win, "WinSxS");
        t.add_file(sxs, "a.dll", 1000, 4096);
        t.add_dir(0, "Windows.old"); // 空的，不列出
        t.add_dir(0, "System Volume Information"); // 读不到，仍然列出
        t.finalize();

        let items = find_items(&t);
        let keys: Vec<_> = items.iter().map(|i| i.key).collect();
        assert_eq!(keys, ["pagefile", "winsxs", "restore"]);
        assert_eq!(items[0].places.len(), 2);
        assert_eq!(items[0].allocated, (4 << 30) + (16 << 20));
        assert_eq!(items[1].places[0].path, r"C:\WINDOWS\WinSxS");
    }

    #[test]
    fn volume_root_of_paths() {
        assert_eq!(volume_root(r"d:\ai").as_deref(), Some(r"D:\"));
        assert_eq!(volume_root(r"\\server\share"), None);
    }
}
