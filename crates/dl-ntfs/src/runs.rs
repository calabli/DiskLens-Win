//! 非常驻属性的 data runs（mapping pairs）。
//!
//! 每个 run 以一个头字节开始：低 4 位是长度字段的字节数，高 4 位是偏移字段的字节数。
//! 偏移是相对上一个 run 起始簇号的有符号增量；偏移字节数为 0 表示稀疏 run。

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// 起始逻辑簇号；`None` 表示稀疏（未分配）。
    pub lcn: Option<u64>,
    pub clusters: u64,
}

/// 解析出全部 run。
pub fn parse_runs(b: &[u8]) -> Result<Vec<Run>> {
    Runs::new(b).collect()
}

/// 实际分配的簇数，即所有非稀疏 run 的长度之和。
///
/// 不能信任属性头里的“已分配大小”：例如 `$BadClus:$Bad` 名义上覆盖整个卷，
/// 但只有一个稀疏 run，且属性不一定带稀疏标记。
pub fn allocated_clusters(b: &[u8]) -> Result<u64> {
    Runs::new(b).try_fold(0u64, |acc, run| {
        let run = run?;
        Ok(acc + if run.lcn.is_some() { run.clusters } else { 0 })
    })
}

/// 逐个产出 run。遇到错误时产出一次 `Err` 后结束。
struct Runs<'a> {
    b: &'a [u8],
    i: usize,
    lcn: i64,
    done: bool,
}

impl<'a> Runs<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self {
            b,
            i: 0,
            lcn: 0,
            done: false,
        }
    }

    fn step(&mut self) -> Result<Option<Run>> {
        let b = self.b;
        let header = *b
            .get(self.i)
            .ok_or(Error::Corrupt("unterminated data runs"))?;
        if header == 0 {
            return Ok(None);
        }
        self.i += 1;
        let len_n = usize::from(header & 0x0F);
        let off_n = usize::from(header >> 4);
        if len_n == 0 || len_n > 8 || off_n > 8 {
            return Err(Error::Corrupt("data run header"));
        }

        let len_bytes = b
            .get(self.i..self.i + len_n)
            .ok_or(Error::Corrupt("data run length"))?;
        self.i += len_n;
        let clusters = len_bytes
            .iter()
            .rev()
            .fold(0u64, |acc, &x| (acc << 8) | u64::from(x));

        if off_n == 0 {
            return Ok(Some(Run {
                lcn: None,
                clusters,
            }));
        }

        let off_bytes = b
            .get(self.i..self.i + off_n)
            .ok_or(Error::Corrupt("data run offset"))?;
        self.i += off_n;
        let raw = off_bytes
            .iter()
            .rev()
            .fold(0u64, |acc, &x| (acc << 8) | u64::from(x));
        // 按字段宽度做符号扩展。
        let shift = 64 - 8 * off_n as u32;
        let delta = ((raw << shift) as i64) >> shift;

        self.lcn = self
            .lcn
            .checked_add(delta)
            .ok_or(Error::Corrupt("data run offset overflow"))?;
        let start =
            u64::try_from(self.lcn).map_err(|_| Error::Corrupt("negative cluster number"))?;
        Ok(Some(Run {
            lcn: Some(start),
            clusters,
        }))
    }
}

impl Iterator for Runs<'_> {
    type Item = Result<Run>;

    fn next(&mut self) -> Option<Result<Run>> {
        if self.done {
            return None;
        }
        let item = self.step().transpose();
        if !matches!(item, Some(Ok(_))) {
            self.done = true;
        }
        item
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_relative_offsets() {
        // 18 个簇 @ 0x5634，再往回 0x56 个簇取 0x20 个，然后结束。
        let b = [0x21, 0x18, 0x34, 0x56, 0x11, 0x20, 0xAA, 0x00];
        let runs = parse_runs(&b).unwrap();
        assert_eq!(
            runs,
            [
                Run {
                    lcn: Some(0x5634),
                    clusters: 0x18
                },
                Run {
                    lcn: Some(0x5634 - 0x56),
                    clusters: 0x20
                },
            ]
        );
        assert_eq!(allocated_clusters(&b).unwrap(), 0x18 + 0x20);
    }

    #[test]
    fn parses_sparse_runs() {
        let b = [0x11, 0x04, 0x10, 0x01, 0x08, 0x11, 0x02, 0x04, 0x00];
        let runs = parse_runs(&b).unwrap();
        assert_eq!(
            runs[1],
            Run {
                lcn: None,
                clusters: 8
            }
        );
        assert_eq!(
            runs[2],
            Run {
                lcn: Some(0x14),
                clusters: 2
            }
        );
        // 稀疏的 8 个簇不计入实际分配。
        assert_eq!(allocated_clusters(&b).unwrap(), 4 + 2);
    }

    #[test]
    fn rejects_unterminated_and_negative() {
        assert!(parse_runs(&[0x11, 0x04]).is_err());
        assert!(parse_runs(&[0x11, 0x04, 0xF0, 0x00]).is_err());
        assert!(allocated_clusters(&[0x11, 0x04]).is_err());
    }
}
