//! 搜索语法（Everything 风格的子集）。
//!
//! 以空格分隔的多个条件同时满足（AND）。支持：
//! - 普通文字：名称包含该文字（不区分大小写）
//! - `!文字`：名称不包含该文字
//! - `size:>1gb`、`size:<=500mb`：大小比较，单位 b / kb / mb / gb / tb（1024 进制），默认字节
//! - `ext:iso;zip`：扩展名是其中之一
//! - `file:` / `folder:`：只要文件 / 只要目录
//! - `type:音视频`、`type:video`：按类别（见 [`Category::from_name`]）
//! - `dm:` / `da:`：按修改时间 / 访问时间，例如 `dm:>2024-01-01`、`dm:2024`（这一年内）、
//!   `da:<1y`（一年以上没访问）、`dm:>7d`（最近 7 天内修改过）。单位 d / w / m / y。
//!   时间未知的条目不满足任何时间条件。
//! - 用双引号括起来的文字可以包含空格，例如 `"program files"`

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::Category;

/// 一个条目中搜索要用到的信息。
#[derive(Debug, Clone, Copy)]
pub struct Item<'a> {
    pub name: &'a str,
    pub is_dir: bool,
    pub size: u64,
    /// 修改时间、访问时间（Unix 秒，0 表示未知）。
    pub modified: u32,
    pub accessed: u32,
    /// 类别；不知道时为 `None`（此时不满足 `type:` 条件）。
    pub category: Option<Category>,
}

/// `name` 的扩展名是否为 `ext`（`ext` 已是小写、不含点），不分配内存。
fn has_extension(name: &str, ext: &str) -> bool {
    let Some(dot) = name.len().checked_sub(ext.len() + 1) else {
        return false;
    };
    dot > 0
        && name.as_bytes()[dot] == b'.'
        && name.is_char_boundary(dot + 1)
        && name[dot + 1..].eq_ignore_ascii_case(ext)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cond {
    Contains(String),
    NotContains(String),
    Size(Cmp, u64),
    Ext(Vec<String>),
    FilesOnly,
    FoldersOnly,
    Type(Category),
    Modified(Cmp, u32),
    Accessed(Cmp, u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    conds: Vec<Cond>,
    /// 是否有按名称包含 / 不包含的条件（需要把名称转成小写）。
    has_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

impl Query {
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self::parse_at(input, now)
    }

    /// 同 [`parse`](Self::parse)，相对时间（如 `dm:<1y`）以 `now`（Unix 秒）为准。
    pub fn parse_at(input: &str, now: u64) -> Result<Self, ParseError> {
        let mut conds = Vec::new();
        for token in tokenize(input) {
            let lower = token.to_lowercase();
            let cond = if let Some(rest) = lower.strip_prefix("size:") {
                parse_size(rest)?
            } else if let Some(rest) = lower.strip_prefix("type:") {
                Cond::Type(Category::from_name(rest).ok_or_else(|| {
                    ParseError(format!(
                        "不认识的类别：type:{rest}，例如 type:视频、type:文档"
                    ))
                })?)
            } else if let Some(rest) = lower.strip_prefix("dm:") {
                for (cmp, t) in parse_time(rest, now, "dm")? {
                    conds.push(Cond::Modified(cmp, t));
                }
                continue;
            } else if let Some(rest) = lower.strip_prefix("da:") {
                for (cmp, t) in parse_time(rest, now, "da")? {
                    conds.push(Cond::Accessed(cmp, t));
                }
                continue;
            } else if let Some(rest) = lower.strip_prefix("ext:") {
                let exts: Vec<String> = rest
                    .split([';', ','])
                    .map(|e| e.trim_start_matches('.').to_string())
                    .filter(|e| !e.is_empty())
                    .collect();
                if exts.is_empty() {
                    return Err(ParseError("ext: 后面需要扩展名，例如 ext:iso;zip".into()));
                }
                Cond::Ext(exts)
            } else if lower == "file:" {
                Cond::FilesOnly
            } else if lower == "folder:" || lower == "dir:" {
                Cond::FoldersOnly
            } else if let Some(rest) = lower.strip_prefix('!') {
                if rest.is_empty() {
                    continue;
                }
                Cond::NotContains(rest.to_string())
            } else {
                Cond::Contains(lower)
            };
            conds.push(cond);
        }
        let has_text = conds
            .iter()
            .any(|c| matches!(c, Cond::Contains(_) | Cond::NotContains(_)));
        Ok(Self { conds, has_text })
    }

    pub fn is_empty(&self) -> bool {
        self.conds.is_empty()
    }

    /// 该条目是否满足全部条件。
    ///
    /// 搜索会对每个节点调用一次：先判断不需要分配内存的条件，
    /// 只有存在文字条件、且其他条件都满足时，才把名称转成小写。
    pub fn matches(&self, item: &Item<'_>) -> bool {
        let cheap = self.conds.iter().all(|c| match c {
            Cond::Contains(_) | Cond::NotContains(_) => true,
            Cond::Size(cmp, n) => cmp.test(item.size, *n),
            Cond::Ext(exts) => !item.is_dir && exts.iter().any(|e| has_extension(item.name, e)),
            Cond::FilesOnly => !item.is_dir,
            Cond::FoldersOnly => item.is_dir,
            Cond::Type(c) => item.category == Some(*c),
            Cond::Modified(cmp, t) => item.modified > 0 && cmp.test(item.modified, *t),
            Cond::Accessed(cmp, t) => item.accessed > 0 && cmp.test(item.accessed, *t),
        });
        if !cheap || !self.has_text {
            return cheap;
        }
        let lower = item.name.to_lowercase();
        self.conds.iter().all(|c| match c {
            Cond::Contains(s) => lower.contains(s.as_str()),
            Cond::NotContains(s) => !lower.contains(s.as_str()),
            _ => true,
        })
    }
}

impl Cmp {
    fn test<T: PartialOrd>(self, a: T, b: T) -> bool {
        match self {
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Eq => a == b,
        }
    }
}

/// 解析 `dm:` / `da:` 后面的部分，得到一到两个比较条件（Unix 秒）。
fn parse_time(spec: &str, now: u64, key: &str) -> Result<Vec<(Cmp, u32)>, ParseError> {
    let bad = || {
        ParseError(format!(
            "无法识别的时间：{key}:{spec}，例如 {key}:>2024-01-01、{key}:2024、{key}:<1y"
        ))
    };
    let (cmp, rest) = if let Some(r) = spec.strip_prefix(">=") {
        (Some(Cmp::Ge), r)
    } else if let Some(r) = spec.strip_prefix("<=") {
        (Some(Cmp::Le), r)
    } else if let Some(r) = spec.strip_prefix('>') {
        (Some(Cmp::Gt), r)
    } else if let Some(r) = spec.strip_prefix('<') {
        (Some(Cmp::Lt), r)
    } else {
        (None, spec.strip_prefix('=').unwrap_or(spec))
    };
    let clamp = |t: i64| u32::try_from(t.max(0)).unwrap_or(u32::MAX);

    // 相对时间：N 天 / 周 / 月 / 年之前。
    if let Some(unit) = rest.chars().last().filter(|c| "dwmy".contains(*c))
        && let Ok(n) = rest[..rest.len() - 1].parse::<i64>()
    {
        let secs = n * match unit {
            'd' => 86_400,
            'w' => 7 * 86_400,
            'm' => 30 * 86_400,
            _ => 365 * 86_400,
        };
        let at = clamp(now as i64 - secs);
        return Ok(vec![(cmp.ok_or_else(bad)?, at)]);
    }

    // 绝对日期：YYYY、YYYY-MM、YYYY-MM-DD（按 UTC）；不带比较符时表示这一整段时间。
    let parts: Vec<&str> = rest.split(['-', '/', '.']).collect();
    let num = |i: usize, default: i64| -> Result<i64, ParseError> {
        parts
            .get(i)
            .map_or(Ok(default), |p| p.parse().map_err(|_| bad()))
    };
    if parts.is_empty() || parts.len() > 3 || parts[0].len() != 4 {
        return Err(bad());
    }
    let (y, m, d) = (num(0, 0)?, num(1, 1)?, num(2, 1)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err(bad());
    }
    let start = days_from_civil(y, m, d) * 86_400;
    let end = match parts.len() {
        1 => days_from_civil(y + 1, 1, 1),
        2 if m == 12 => days_from_civil(y + 1, 1, 1),
        2 => days_from_civil(y, m + 1, 1),
        _ => days_from_civil(y, m, d) + 1,
    } * 86_400;
    Ok(match cmp {
        None => vec![(Cmp::Ge, clamp(start)), (Cmp::Lt, clamp(end))],
        // “晚于 2024” 指 2024 年之后；“早于 2024” 指 2024 年之前。
        Some(Cmp::Gt) => vec![(Cmp::Ge, clamp(end))],
        Some(Cmp::Le) => vec![(Cmp::Lt, clamp(end))],
        Some(c) => vec![(c, clamp(start))],
    })
}

/// 公历日期到 1970-01-01 起的天数（Howard Hinnant 的算法）。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn tokenize(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in input.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_size(spec: &str) -> Result<Cond, ParseError> {
    let (cmp, rest) = if let Some(r) = spec.strip_prefix(">=") {
        (Cmp::Ge, r)
    } else if let Some(r) = spec.strip_prefix("<=") {
        (Cmp::Le, r)
    } else if let Some(r) = spec.strip_prefix('>') {
        (Cmp::Gt, r)
    } else if let Some(r) = spec.strip_prefix('<') {
        (Cmp::Lt, r)
    } else if let Some(r) = spec.strip_prefix('=') {
        (Cmp::Eq, r)
    } else {
        (Cmp::Eq, spec)
    };
    let bad = || ParseError(format!("无法识别的大小：size:{spec}，例如 size:>1gb"));
    let digits_end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    let value: f64 = rest[..digits_end].parse().map_err(|_| bad())?;
    let unit: u64 = match rest[digits_end..].trim() {
        "" | "b" => 1,
        "k" | "kb" => 1 << 10,
        "m" | "mb" => 1 << 20,
        "g" | "gb" => 1 << 30,
        "t" | "tb" => 1 << 40,
        _ => return Err(bad()),
    };
    Ok(Cond::Size(cmp, (value * unit as f64) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn item(name: &str, is_dir: bool, size: u64) -> Item<'_> {
        Item {
            name,
            is_dir,
            size,
            modified: 0,
            accessed: 0,
            category: None,
        }
    }

    trait Matches {
        fn hit(&self, name: &str, is_dir: bool, size: u64) -> bool;
    }

    impl Matches for Query {
        fn hit(&self, name: &str, is_dir: bool, size: u64) -> bool {
            self.matches(&item(name, is_dir, size))
        }
    }

    #[test]
    fn plain_text_is_case_insensitive_and() {
        let q = Query::parse("Win iso").unwrap();
        assert!(q.hit("Win11_24H2.ISO", false, 1));
        assert!(!q.hit("Win11.zip", false, 1));
    }

    #[test]
    fn dates_and_types() {
        // 2026-10-27 00:00 UTC
        let now = days_from_civil(2026, 10, 27) as u64 * 86_400;
        let at = |y, m, d| (days_from_civil(y, m, d) * 86_400) as u32;
        let file = |modified, accessed| Item {
            modified,
            accessed,
            ..item("a.mkv", false, 1)
        };
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(
            days_from_civil(2024, 3, 1) - days_from_civil(2024, 2, 1),
            29
        );

        let q = Query::parse_at("dm:2024", now).unwrap();
        assert!(q.matches(&file(at(2024, 6, 1), 0)));
        assert!(!q.matches(&file(at(2025, 1, 1), 0)));
        assert!(!q.matches(&file(at(2023, 12, 31), 0)));

        let newer = Query::parse_at("dm:>2024-03", now).unwrap();
        assert!(newer.matches(&file(at(2024, 4, 1), 0)));
        assert!(!newer.matches(&file(at(2024, 3, 15), 0)));
        assert!(
            Query::parse_at("dm:<2024-03-02", now)
                .unwrap()
                .matches(&file(at(2024, 3, 1), 0))
        );

        let stale = Query::parse_at("da:<1y", now).unwrap();
        assert!(stale.matches(&file(0, at(2025, 1, 1))));
        assert!(!stale.matches(&file(0, at(2026, 9, 1))));
        assert!(!stale.matches(&file(0, 0)), "unknown time never matches");
        assert!(
            Query::parse_at("dm:>7d", now)
                .unwrap()
                .matches(&file(at(2026, 10, 25), 0))
        );

        let video = Query::parse("type:视频").unwrap();
        assert!(video.matches(&Item {
            category: Some(Category::Media),
            ..item("a.mkv", false, 1)
        }));
        assert!(!video.matches(&item("a.mkv", false, 1)), "category unknown");
        assert!(Query::parse("type:音视频").is_ok());
        assert!(Query::parse("type:media").is_ok());
        assert!(Query::parse("type:什么").is_err());
        assert!(Query::parse("dm:yesterday").is_err());
        assert!(Query::parse("dm:2024-13").is_err());
        assert!(
            Query::parse("dm:7d").is_err(),
            "relative time needs a comparison"
        );
    }

    #[test]
    fn size_and_ext() {
        let q = Query::parse("size:>1gb ext:iso;.zip").unwrap();
        assert!(q.hit("a.iso", false, 2 * GB));
        assert!(!q.hit("a.iso", false, GB));
        assert!(!q.hit("a.mkv", false, 2 * GB));
        assert!(!q.hit("folder.iso", true, 2 * GB));
        assert!(
            Query::parse("size:<=1.5gb")
                .unwrap()
                .hit("x", false, GB + GB / 2)
        );
        assert!(Query::parse("size:100").unwrap().hit("x", false, 100));
    }

    #[test]
    fn negation_quotes_and_kind_filters() {
        let q = Query::parse("\"program files\" !x86 folder:").unwrap();
        assert!(q.hit("Program Files", true, 0));
        assert!(!q.hit("Program Files (x86)", true, 0));
        assert!(!q.hit("Program Files", false, 0));
        assert!(Query::parse("file:").unwrap().hit("a", false, 0));
    }

    #[test]
    fn extension_matching() {
        assert!(has_extension("a.ISO", "iso"));
        assert!(has_extension("a.tar.gz", "gz"));
        assert!(!has_extension("iso", "iso"));
        assert!(!has_extension(".iso", "iso"));
        assert!(!has_extension("a.xiso", "iso"));
        assert!(!has_extension("报告.iso", "so"));
    }

    #[test]
    fn reports_bad_syntax() {
        assert!(Query::parse("size:>lots").is_err());
        assert!(Query::parse("size:>5pb").is_err());
        assert!(Query::parse("ext:").is_err());
        assert!(Query::parse("   ").unwrap().is_empty());
    }
}
