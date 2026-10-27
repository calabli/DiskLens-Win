//! 受保护路径。
//!
//! 三类规则：
//! - **整棵子树**：Windows、Program Files、ProgramData 及其下的一切都不能直接删除；
//! - **只保护自身**：用户文件夹、AppData、常用文件夹等，本身不能删，里面的内容可以；
//! - **卷级**：整个磁盘、`$` 开头的 NTFS 元文件和回收站、System Volume Information、
//!   页面文件和休眠文件等。
//!
//! 另外，包含任何受保护路径的上级文件夹也不能删除（例如 `C:\Users`）。

use std::path::Path;

/// 卷根目录下的这些名称（不区分大小写）不能删除。
const VOLUME_RESERVED: &[&str] = &[
    "system volume information",
    "recovery",
    "boot",
    "efi",
    "bootmgr",
    "bootnxt",
    "pagefile.sys",
    "hiberfil.sys",
    "swapfile.sys",
];

/// 一条规则：`key` 用于比较（规范化后的路径），`display` 是给用户看的原始写法。
#[derive(Debug, Clone)]
struct Rule {
    key: String,
    display: String,
}

impl Rule {
    fn parse(path: &str) -> Option<Self> {
        Some(Self {
            key: normalize(path)?,
            display: path.trim_end_matches(['\\', '/']).to_string(),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Protector {
    subtree: Vec<Rule>,
    exact: Vec<Rule>,
}

impl Protector {
    /// 用给定的规则构造（测试用）。
    pub fn new(subtree: &[&str], exact: &[&str]) -> Self {
        Self {
            subtree: subtree.iter().filter_map(|p| Rule::parse(p)).collect(),
            exact: exact.iter().filter_map(|p| Rule::parse(p)).collect(),
        }
    }

    /// 按当前系统的环境变量构造：系统目录、程序目录、当前用户的文件夹。
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var(k).ok();
        let mut subtree: Vec<String> = [
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
            "ProgramData",
        ]
        .iter()
        .filter_map(|k| var(k))
        .collect();
        subtree.dedup();

        let mut exact = Vec::new();
        if let Some(profile) = var("USERPROFILE") {
            if let Some(users) = Path::new(&profile).parent() {
                exact.push(users.to_string_lossy().into_owned());
            }
            for sub in [
                "",
                "AppData",
                "AppData\\Local",
                "AppData\\LocalLow",
                "AppData\\Roaming",
                "Desktop",
                "Documents",
                "Downloads",
                "Pictures",
                "Music",
                "Videos",
            ] {
                exact.push(if sub.is_empty() {
                    profile.clone()
                } else {
                    format!("{profile}\\{sub}")
                });
            }
        }
        if let Some(public) = var("PUBLIC") {
            exact.push(public);
        }

        let subtree: Vec<&str> = subtree.iter().map(String::as_str).collect();
        let exact: Vec<&str> = exact.iter().map(String::as_str).collect();
        Self::new(&subtree, &exact)
    }

    /// 不能删除时返回原因。
    pub fn check(&self, path: &Path) -> Option<String> {
        self.check_allowing(path, &[])
    }

    /// 同 [`check`](Self::check)，但位于 `allowed` 中某个位置（含其本身）的路径不受“整棵子树”
    /// 保护的限制。清理规则用它处理受保护文件夹里的特定位置，例如 `C:\Windows\Temp`。
    /// 卷级保留项目、被保护的文件夹本身和包含受保护文件夹的上级仍然不能删除。
    pub fn check_allowing(&self, path: &Path, allowed: &[String]) -> Option<String> {
        let Some(p) = normalize(&path.to_string_lossy()) else {
            return Some("不是完整路径".into());
        };
        let exempt = allowed
            .iter()
            .filter_map(|a| normalize(a))
            .any(|a| p == a || is_under(&p, &a));
        let (_, rest) = split_root(&p);
        if rest.is_empty() {
            return Some("不能删除整个磁盘".into());
        }
        let first = rest.split('\\').next().unwrap_or_default();
        if first.starts_with('$') || VOLUME_RESERVED.contains(&first) {
            return Some("这是系统保留项目，只能通过系统设置处理".into());
        }
        if let Some(dir) = self
            .subtree
            .iter()
            .find(|d| !exempt && (p == d.key || is_under(&p, &d.key)))
        {
            return Some(format!("位于受保护的系统文件夹 {} 中", dir.display));
        }
        if self.exact.iter().any(|d| d.key == p) {
            return Some("这是受保护的文件夹，只能删除其中的内容".into());
        }
        if let Some(inner) = self
            .subtree
            .iter()
            .chain(&self.exact)
            .find(|d| is_under(&d.key, &p))
        {
            return Some(format!("其中包含受保护的文件夹 {}", inner.display));
        }
        None
    }
}

/// `path` 是否严格位于 `dir` 之下。
fn is_under(path: &str, dir: &str) -> bool {
    path.len() > dir.len() && path.starts_with(dir) && path.as_bytes()[dir.len()] == b'\\'
}

/// 拆出根部分（`c:` 或 `\\server\share`）和其余部分。
fn split_root(p: &str) -> (&str, &str) {
    if let Some(unc) = p.strip_prefix("\\\\") {
        // \\server\share\rest
        let mut parts = unc.splitn(3, '\\');
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        let root_len = 2 + server.len() + 1 + share.len();
        let root = &p[..root_len.min(p.len())];
        let rest = p.get(root_len + 1..).unwrap_or("");
        (root, rest)
    } else {
        (&p[..2], p.get(3..).unwrap_or(""))
    }
}

/// 规范化：去掉 `\\?\` 前缀，`/` 换成 `\`，去掉末尾的 `\`，转小写。
/// 不是盘符或 UNC 开头的完整路径返回 `None`。
pub fn normalize(path: &str) -> Option<String> {
    let mut s = path.trim().replace('/', "\\");
    if let Some(rest) = s.strip_prefix("\\\\?\\UNC\\") {
        s = format!("\\\\{rest}");
    } else if let Some(rest) = s.strip_prefix("\\\\?\\") {
        s = rest.to_string();
    }
    while s.ends_with('\\') && s.len() > 3 && !s.ends_with(":\\") {
        s.pop();
    }
    let s = s.to_lowercase();
    let b = s.as_bytes();
    let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    let drive_ok = drive && (b.len() == 2 || b[2] == b'\\');
    let unc = s.starts_with("\\\\") && s[2..].contains('\\');
    if !(drive_ok || unc) {
        return None;
    }
    // "c:\" 与 "c:" 统一成 "c:"。
    Some(if drive && s.len() == 3 {
        s[..2].to_string()
    } else {
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protector() -> Protector {
        Protector::new(
            &[r"C:\Windows", r"C:\Program Files", r"C:\ProgramData"],
            &[
                r"C:\Users",
                r"C:\Users\alice",
                r"C:\Users\alice\AppData",
                r"C:\Users\alice\Downloads",
            ],
        )
    }

    fn blocked(p: &str) -> bool {
        protector().check(Path::new(p)).is_some()
    }

    #[test]
    fn system_trees_are_protected() {
        assert!(blocked(r"C:\Windows"));
        assert!(blocked(r"c:\windows\system32\drivers"));
        assert!(blocked(r"C:/Program Files/App/app.exe"));
        assert!(blocked(r"\\?\C:\ProgramData\x"));
        assert!(
            !blocked(r"C:\WindowsApps-backup"),
            "prefix of a name is not a match"
        );
    }

    #[test]
    fn profile_folders_protect_themselves_only() {
        assert!(blocked(r"C:\Users\alice"));
        assert!(blocked(r"C:\Users\alice\AppData\"));
        assert!(blocked(r"C:\Users\alice\Downloads"));
        assert!(!blocked(r"C:\Users\alice\Downloads\setup.iso"));
        assert!(!blocked(r"C:\Users\alice\AppData\Local\Temp\x"));
    }

    #[test]
    fn ancestors_of_protected_paths_are_protected() {
        assert!(blocked(r"C:\Users"));
        let p = Protector::new(&[r"D:\keep\inner"], &[]);
        assert!(p.check(Path::new(r"D:\keep")).is_some());
        assert!(p.check(Path::new(r"D:\keeper")).is_none());
    }

    #[test]
    fn volume_level_reservations() {
        assert!(blocked(r"D:\"));
        assert!(blocked("D:"));
        assert!(blocked(r"D:\$Recycle.Bin\S-1-5"));
        assert!(blocked(r"D:\$MFT"));
        assert!(blocked(r"D:\pagefile.sys"));
        assert!(blocked(r"T:\System Volume Information"));
        assert!(!blocked(r"D:\ai\cc\old-project"));
        assert!(!blocked(r"D:\boot-camp-notes.txt"));
        assert!(blocked(r"\\nas\share"));
        assert!(!blocked(r"\\nas\share\movies\a.mkv"));
    }

    #[test]
    fn reasons_show_original_spelling() {
        let reason = protector().check(Path::new(r"c:\windows\x")).unwrap();
        assert!(reason.contains(r"C:\Windows"), "{reason}");
        let p = Protector::new(&[r"D:\Keep\Inner"], &[]);
        let reason = p.check(Path::new(r"d:\keep")).unwrap();
        assert!(reason.contains(r"D:\Keep\Inner"), "{reason}");
    }

    #[test]
    fn rules_can_open_specific_places_only() {
        let p = protector();
        let allow = [r"C:\WINDOWS\Temp".to_string()];
        let ok = |path: &str| p.check_allowing(Path::new(path), &allow).is_none();
        assert!(ok(r"C:\Windows\Temp\x.log"));
        assert!(ok(r"C:\Windows\Temp\sub\y"));
        assert!(!ok(r"C:\Windows\System32\x.dll"), "only the allowed place");
        assert!(!ok(r"C:\Windows"), "not the protected folder itself");
        assert!(
            !ok(r"C:\Windows\TempX\y"),
            "prefix of a name is not a match"
        );
        let reserved = [r"D:\$Recycle.Bin".to_string()];
        assert!(
            p.check_allowing(Path::new(r"D:\$Recycle.Bin\x"), &reserved)
                .is_some()
        );
        assert!(
            p.check(Path::new(r"C:\Windows\Temp\x.log")).is_some(),
            "plain check unchanged"
        );
    }

    #[test]
    fn relative_paths_are_rejected() {
        assert!(blocked(r"Windows\x"));
        assert!(blocked(""));
    }

    #[test]
    fn normalize_cases() {
        assert_eq!(normalize(r"C:\").as_deref(), Some("c:"));
        assert_eq!(normalize(r"C:\A\B\\").as_deref(), Some(r"c:\a\b"));
        assert_eq!(
            normalize(r"\\?\UNC\nas\share\x").as_deref(),
            Some(r"\\nas\share\x")
        );
        assert_eq!(normalize("relative"), None);
        assert_eq!(normalize("C:relative"), None);
    }
}
