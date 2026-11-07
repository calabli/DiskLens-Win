//! 操作日志：每次删除、硬链接替换、移动、从回收站放回（包括被拦截和失败的）追加一行 JSON
//! 到按月分的文件里。清理记录页面再把它读回来。

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{Mode, Outcome};

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// UTC 时间，RFC 3339 格式。
    pub time: String,
    pub action: &'static str,
    pub path: String,
    pub size: u64,
    pub result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 硬链接替换时链接到的文件；移动时的新位置。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_to: Option<String>,
}

impl LogEntry {
    pub fn new(path: &Path, size: u64, mode: Mode, outcome: &Outcome) -> Self {
        let action = match mode {
            Mode::Recycle => "recycle",
            Mode::Permanent => "delete",
        };
        Self::build(action, path, size, outcome, None)
    }

    /// 把文件夹 `path` 移到 `to`，原位置留下连接点。
    pub fn moved(path: &Path, to: &Path, size: u64, outcome: &Outcome) -> Self {
        let to = Some(to.to_string_lossy().into_owned());
        Self::build("move", path, size, outcome, to)
    }

    /// 把回收站里的一项放回原处 `path`。
    pub fn restored(path: &Path, size: u64, outcome: &Outcome) -> Self {
        Self::build("restore", path, size, outcome, None)
    }

    /// 把 `path` 替换为指向 `to` 的硬链接。
    pub fn link(path: &Path, to: &Path, size: u64, outcome: &Outcome) -> Self {
        let to = Some(to.to_string_lossy().into_owned());
        Self::build("hardlink", path, size, outcome, to)
    }

    fn build(
        action: &'static str,
        path: &Path,
        size: u64,
        outcome: &Outcome,
        link_to: Option<String>,
    ) -> Self {
        let (result, reason) = match outcome {
            Outcome::Deleted => ("deleted", None),
            Outcome::Linked => ("linked", None),
            Outcome::Moved => ("moved", None),
            Outcome::Restored => ("restored", None),
            Outcome::Blocked(r) => ("blocked", Some(r.clone())),
            Outcome::Failed(r) => ("failed", Some(r.clone())),
        };
        Self {
            time: rfc3339(SystemTime::now()),
            action,
            path: path.to_string_lossy().into_owned(),
            size,
            result,
            reason,
            link_to,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpLog {
    dir: PathBuf,
}

impl OpLog {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// 默认位置：`%LOCALAPPDATA%\DiskLens\logs`。
    pub fn default_dir() -> PathBuf {
        let base = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
        base.join("DiskLens").join("logs")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn append(&self, entry: &LogEntry) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        // 按月分文件：operations-2026-10.jsonl
        let file = self
            .dir
            .join(format!("operations-{}.jsonl", &entry.time[..7]));
        let mut line = serde_json::to_string(entry).map_err(io::Error::other)?;
        line.push('\n');
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)?
            .write_all(line.as_bytes())
    }
}

/// 从日志读回的一行。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Record {
    pub time: String,
    /// `recycle`、`delete`、`hardlink`、`move`、`restore`。
    pub action: String,
    pub path: String,
    pub size: u64,
    /// `deleted`、`linked`、`moved`、`restored`、`blocked`、`failed`。
    pub result: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub link_to: Option<String>,
}

impl OpLog {
    /// 最近的至多 `limit` 条记录，从新到旧。读不了的文件和格式不对的行跳过。
    pub fn read(&self, limit: usize) -> Vec<Record> {
        let mut files: Vec<PathBuf> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("operations-") && n.ends_with(".jsonl"))
            })
            .collect();
        // 文件名里的年月按字典序就是时间顺序。
        files.sort();
        let mut out = Vec::new();
        for file in files.iter().rev() {
            let Ok(text) = fs::read_to_string(file) else {
                continue;
            };
            for line in text.lines().rev() {
                if let Ok(r) = serde_json::from_str::<Record>(line) {
                    out.push(r);
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        out
    }
}

/// 解析 [`rfc3339`] 写出的时间，得到 Unix 秒。
pub fn parse_time(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let days = days_from_civil(y, m as u32, d as u32);
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

/// 公历日期换算成 1970-01-01 起的天数，[`civil_from_days`] 的逆运算。
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// UTC 时间格式化为 `2026-10-26T13:45:07Z`，不依赖时间库。
pub fn rfc3339(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// 1970-01-01 起的天数换算成公历日期（Howard Hinnant 的算法）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn formats_utc_times() {
        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let t = UNIX_EPOCH + Duration::from_secs(1_793_000_000);
        // 1_793_000_000 = 20752 天 + 27200 秒（7:33:20）
        assert_eq!(rfc3339(t), "2026-10-26T07:33:20Z");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400); // 2000-02-29
        assert_eq!(rfc3339(leap), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn parses_its_own_times() {
        for secs in [0u64, 951_782_400, 1_793_000_000, 1_793_993_593] {
            let s = rfc3339(UNIX_EPOCH + Duration::from_secs(secs));
            assert_eq!(parse_time(&s), Some(secs), "{s}");
        }
        assert_eq!(parse_time("2026-10-26 07:33:20"), None);
    }

    #[test]
    fn reads_back_newest_first_across_months() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .join("target")
            .join("test-tmp")
            .join(format!("dl-clean-log-read-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let line = |t: &str, p: &str| {
            format!(
                r#"{{"time":"{t}","action":"recycle","path":"{p}","size":1,"result":"deleted"}}"#
            )
        };
        fs::write(
            dir.join("operations-2026-10.jsonl"),
            format!(
                "{}\n{}\n",
                line("2026-10-01T00:00:00Z", "a"),
                line("2026-10-02T00:00:00Z", "b")
            ),
        )
        .unwrap();
        fs::write(
            dir.join("operations-2026-11.jsonl"),
            format!("{}\nnot json\n", line("2026-11-01T00:00:00Z", "c")),
        )
        .unwrap();
        let log = OpLog::new(&dir);
        let paths: Vec<String> = log.read(10).into_iter().map(|r| r.path).collect();
        assert_eq!(paths, ["c", "b", "a"]);
        assert_eq!(log.read(2).len(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn appends_json_lines_by_month() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .join("target")
            .join("test-tmp")
            .join(format!("dl-clean-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let log = OpLog::new(&dir);
        let blocked = Outcome::Blocked("系统文件夹".into());
        log.append(&LogEntry::new(
            Path::new(r"D:\a.iso"),
            7,
            Mode::Recycle,
            &Outcome::Deleted,
        ))
        .unwrap();
        log.append(&LogEntry::new(
            Path::new(r"C:\Windows"),
            0,
            Mode::Permanent,
            &blocked,
        ))
        .unwrap();

        let files: Vec<_> = fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);
        let text = fs::read_to_string(files[0].path()).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["action"], "recycle");
        assert_eq!(lines[0]["result"], "deleted");
        assert!(lines[0].get("reason").is_none());
        assert_eq!(lines[1]["result"], "blocked");
        assert_eq!(lines[1]["reason"], "系统文件夹");
        fs::remove_dir_all(&dir).unwrap();
    }
}
