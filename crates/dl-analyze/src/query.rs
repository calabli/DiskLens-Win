//! 搜索语法（Everything 风格的子集）。
//!
//! 以空格分隔的多个条件同时满足（AND）。支持：
//! - 普通文字：名称包含该文字（不区分大小写）
//! - `!文字`：名称不包含该文字
//! - `size:>1gb`、`size:<=500mb`：大小比较，单位 b / kb / mb / gb / tb（1024 进制），默认字节
//! - `ext:iso;zip`：扩展名是其中之一
//! - `file:` / `folder:`：只要文件 / 只要目录
//! - 用双引号括起来的文字可以包含空格，例如 `"program files"`

use std::fmt;

use crate::category::extension;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cond {
    Contains(String),
    NotContains(String),
    Size(Cmp, u64),
    Ext(Vec<String>),
    FilesOnly,
    FoldersOnly,
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
        let mut conds = Vec::new();
        for token in tokenize(input) {
            let lower = token.to_lowercase();
            let cond = if let Some(rest) = lower.strip_prefix("size:") {
                parse_size(rest)?
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
        Ok(Self { conds })
    }

    pub fn is_empty(&self) -> bool {
        self.conds.is_empty()
    }

    /// 该条目是否满足全部条件。
    pub fn matches(&self, name: &str, is_dir: bool, size: u64) -> bool {
        let lower = name.to_lowercase();
        self.conds.iter().all(|c| match c {
            Cond::Contains(s) => lower.contains(s.as_str()),
            Cond::NotContains(s) => !lower.contains(s.as_str()),
            Cond::Size(cmp, n) => match cmp {
                Cmp::Gt => size > *n,
                Cmp::Ge => size >= *n,
                Cmp::Lt => size < *n,
                Cmp::Le => size <= *n,
                Cmp::Eq => size == *n,
            },
            Cond::Ext(exts) => !is_dir && exts.iter().any(|e| *e == extension(name)),
            Cond::FilesOnly => !is_dir,
            Cond::FoldersOnly => is_dir,
        })
    }
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

    #[test]
    fn plain_text_is_case_insensitive_and() {
        let q = Query::parse("Win iso").unwrap();
        assert!(q.matches("Win11_24H2.ISO", false, 1));
        assert!(!q.matches("Win11.zip", false, 1));
    }

    #[test]
    fn size_and_ext() {
        let q = Query::parse("size:>1gb ext:iso;.zip").unwrap();
        assert!(q.matches("a.iso", false, 2 * GB));
        assert!(!q.matches("a.iso", false, GB));
        assert!(!q.matches("a.mkv", false, 2 * GB));
        assert!(!q.matches("folder.iso", true, 2 * GB));
        assert!(
            Query::parse("size:<=1.5gb")
                .unwrap()
                .matches("x", false, GB + GB / 2)
        );
        assert!(Query::parse("size:100").unwrap().matches("x", false, 100));
    }

    #[test]
    fn negation_quotes_and_kind_filters() {
        let q = Query::parse("\"program files\" !x86 folder:").unwrap();
        assert!(q.matches("Program Files", true, 0));
        assert!(!q.matches("Program Files (x86)", true, 0));
        assert!(!q.matches("Program Files", false, 0));
        assert!(Query::parse("file:").unwrap().matches("a", false, 0));
    }

    #[test]
    fn reports_bad_syntax() {
        assert!(Query::parse("size:>lots").is_err());
        assert!(Query::parse("size:>5pb").is_err());
        assert!(Query::parse("ext:").is_err());
        assert!(Query::parse("   ").unwrap().is_empty());
    }
}
