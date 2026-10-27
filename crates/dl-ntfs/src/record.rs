//! 单条 MFT 文件记录（FILE record）。
//!
//! 记录在磁盘上受 update sequence 保护：每 512 字节的最后两个字节被替换成序列号，
//! 原值存放在记录头后的数组里。读出后必须先做 fixup 还原，再解析属性。

use crate::runs::{Run, allocated_clusters, parse_runs};
use crate::{Error, Result, le_u16, le_u32, le_u64};

pub const ATTR_STANDARD_INFORMATION: u32 = 0x10;
pub const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
pub const ATTR_FILE_NAME: u32 = 0x30;
pub const ATTR_DATA: u32 = 0x80;
const ATTR_END: u32 = 0xFFFF_FFFF;

const RECORD_IN_USE: u16 = 0x0001;
const RECORD_IS_DIRECTORY: u16 = 0x0002;

/// 只有 8.3 短名的命名空间。同一文件总还有一个长名，短名一律跳过。
pub const NAMESPACE_DOS: u8 = 2;

/// update sequence 的步长固定为 512 字节，与物理扇区大小无关。
const FIXUP_STRIDE: usize = 512;

/// 文件引用的低 48 位是记录号，高 16 位是序列号。
pub const REF_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileName {
    /// 父目录的记录号。
    pub parent: u64,
    pub namespace: u8,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataStream {
    /// 有名称的是备用数据流（ADS）。
    pub named: bool,
    /// 逻辑大小。属性被拆到多条记录时，只有起始 VCN 为 0 的那一段带有大小，其余为 `None`。
    pub size: Option<u64>,
    /// 这一段实际分配的簇数（稀疏部分不计）；常驻属性为 0。
    pub clusters: u64,
    /// 仅在要求时解析，目前只有 `$MFT` 自身需要。
    pub runs: Option<Vec<Run>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    pub is_dir: bool,
    /// 扩展记录指向的基记录号；基记录本身为 0。
    pub base: u64,
    pub names: Vec<FileName>,
    pub streams: Vec<DataStream>,
    /// 属性分散在多条记录里。其他记录都会被顺序扫到，只有 `$MFT` 自身需要关心。
    pub has_attribute_list: bool,
    /// `$STANDARD_INFORMATION` 里的修改时间和访问时间（Unix 秒，0 表示没有或早于 1970 年）。
    pub modified: u32,
    pub accessed: u32,
}

/// Windows FILETIME（1601 年起的 100 纳秒）换算成 Unix 秒；超出 u32 范围的取边界。
pub fn filetime_to_unix(ft: u64) -> u32 {
    const EPOCH_DIFF: u64 = 11_644_473_600;
    let secs = ft / 10_000_000;
    u32::try_from(secs.saturating_sub(EPOCH_DIFF)).unwrap_or(u32::MAX)
}

/// 还原 update sequence。记录损坏（撕裂写入）时返回错误。
pub fn apply_fixup(rec: &mut [u8]) -> Result<()> {
    let usa_off = usize::from(le_u16(rec, 4)?);
    let usa_count = usize::from(le_u16(rec, 6)?);
    if usa_count < 2 {
        return Err(Error::Corrupt("update sequence array"));
    }
    let strides = usa_count - 1;
    if strides * FIXUP_STRIDE > rec.len() || usa_off + usa_count * 2 > rec.len() {
        return Err(Error::Corrupt("update sequence array"));
    }
    let usn = [rec[usa_off], rec[usa_off + 1]];
    for i in 0..strides {
        let end = (i + 1) * FIXUP_STRIDE;
        if rec[end - 2..end] != usn {
            return Err(Error::Corrupt("update sequence mismatch"));
        }
        let saved = usa_off + 2 * (i + 1);
        rec[end - 2] = rec[saved];
        rec[end - 1] = rec[saved + 1];
    }
    Ok(())
}

/// 解析一条记录（会就地做 fixup）。空白记录和未使用的记录返回 `None`。
pub fn parse_record(rec: &mut [u8], want_runs: bool) -> Result<Option<Record>> {
    if rec.len() < 0x30 || &rec[0..4] != b"FILE" {
        return Ok(None);
    }
    apply_fixup(rec)?;
    let flags = le_u16(rec, 0x16)?;
    if flags & RECORD_IN_USE == 0 {
        return Ok(None);
    }
    let used = (le_u32(rec, 0x18)? as usize).min(rec.len());
    let mut out = Record {
        is_dir: flags & RECORD_IS_DIRECTORY != 0,
        base: le_u64(rec, 0x20)? & REF_MASK,
        ..Record::default()
    };

    let mut off = usize::from(le_u16(rec, 0x14)?);
    while off + 8 <= used {
        let ty = le_u32(rec, off)?;
        if ty == ATTR_END {
            break;
        }
        let len = le_u32(rec, off + 4)? as usize;
        if len < 0x18 || off + len > used {
            return Err(Error::Corrupt("attribute length"));
        }
        let attr = &rec[off..off + len];
        match ty {
            ATTR_ATTRIBUTE_LIST => out.has_attribute_list = true,
            ATTR_FILE_NAME => out.names.push(parse_file_name(attr)?),
            ATTR_STANDARD_INFORMATION => {
                // 依次是创建、修改、MFT 变更、访问时间，各 8 字节。
                let v = resident_value(attr)?;
                out.modified = filetime_to_unix(le_u64(v, 0x08)?);
                out.accessed = filetime_to_unix(le_u64(v, 0x18)?);
            }
            ATTR_DATA => out.streams.push(parse_data(attr, want_runs)?),
            _ => {}
        }
        off += len;
    }
    Ok(Some(out))
}

fn resident_value(attr: &[u8]) -> Result<&[u8]> {
    if attr[8] != 0 {
        return Err(Error::Corrupt("expected resident attribute"));
    }
    let len = le_u32(attr, 0x10)? as usize;
    let off = usize::from(le_u16(attr, 0x14)?);
    attr.get(off..off + len)
        .ok_or(Error::Corrupt("resident value"))
}

fn parse_file_name(attr: &[u8]) -> Result<FileName> {
    let v = resident_value(attr)?;
    let name_len = usize::from(*v.get(0x40).ok_or(Error::Corrupt("file name"))?);
    let namespace = v[0x41];
    let raw = v
        .get(0x42..0x42 + 2 * name_len)
        .ok_or(Error::Corrupt("file name"))?;
    let (pairs, _) = raw.as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().map(|&c| u16::from_le_bytes(c)).collect();
    Ok(FileName {
        parent: le_u64(v, 0)? & REF_MASK,
        namespace,
        name: String::from_utf16_lossy(&units),
    })
}

fn parse_data(attr: &[u8], want_runs: bool) -> Result<DataStream> {
    let named = attr[9] != 0;
    if attr[8] == 0 {
        // 常驻：内容就在记录里，不另占簇。
        let size = resident_value(attr)?.len() as u64;
        return Ok(DataStream {
            named,
            size: Some(size),
            clusters: 0,
            runs: None,
        });
    }
    let first_segment = le_u64(attr, 0x10)? == 0;
    let off = usize::from(le_u16(attr, 0x20)?);
    let run_bytes = attr.get(off..).ok_or(Error::Corrupt("data runs"))?;
    // 占用空间按 runs 实际数簇：压缩、稀疏以及没带标记的稀疏属性都能算对。
    Ok(DataStream {
        named,
        size: if first_segment {
            Some(le_u64(attr, 0x30)?)
        } else {
            None
        },
        clusters: allocated_clusters(run_bytes)?,
        runs: if want_runs {
            Some(parse_runs(run_bytes)?)
        } else {
            None
        },
    })
}

#[cfg(test)]
pub(crate) mod build {
    //! 测试用：按 NTFS 布局拼出文件记录。

    pub const RECORD_SIZE: usize = 1024;

    pub fn file_name(parent: u64, name: &str, namespace: u8) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut v = vec![0u8; 0x42 + units.len() * 2];
        // 高 16 位放一个序列号，验证解析时会被屏蔽掉。
        v[0..8].copy_from_slice(&(parent | (3 << 48)).to_le_bytes());
        v[0x40] = units.len() as u8;
        v[0x41] = namespace;
        for (i, u) in units.iter().enumerate() {
            v[0x42 + i * 2..0x44 + i * 2].copy_from_slice(&u.to_le_bytes());
        }
        resident(super::ATTR_FILE_NAME, None, &v)
    }

    pub fn resident(ty: u32, name: Option<&str>, value: &[u8]) -> Vec<u8> {
        let name_units: Vec<u16> = name.map(|n| n.encode_utf16().collect()).unwrap_or_default();
        let name_off = 0x18;
        let value_off = align8(name_off + name_units.len() * 2);
        let len = align8(value_off + value.len());
        let mut a = vec![0u8; len];
        a[0..4].copy_from_slice(&ty.to_le_bytes());
        a[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        a[9] = name_units.len() as u8;
        a[0x0A..0x0C].copy_from_slice(&(name_off as u16).to_le_bytes());
        for (i, u) in name_units.iter().enumerate() {
            a[name_off + i * 2..name_off + i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        a[0x10..0x14].copy_from_slice(&(value.len() as u32).to_le_bytes());
        a[0x14..0x16].copy_from_slice(&(value_off as u16).to_le_bytes());
        a[value_off..value_off + value.len()].copy_from_slice(value);
        a
    }

    pub struct NonResident<'a> {
        pub name: Option<&'a str>,
        pub size: u64,
        pub allocated: u64,
        pub flags: u16,
        pub total_allocated: Option<u64>,
        pub runs: &'a [u8],
    }

    pub fn non_resident(d: &NonResident<'_>) -> Vec<u8> {
        let name_units: Vec<u16> = d
            .name
            .map(|n| n.encode_utf16().collect())
            .unwrap_or_default();
        let header = if d.total_allocated.is_some() {
            0x48
        } else {
            0x40
        };
        let name_off = header;
        let runs_off = align8(name_off + name_units.len() * 2);
        let len = align8(runs_off + d.runs.len());
        let mut a = vec![0u8; len];
        a[0..4].copy_from_slice(&super::ATTR_DATA.to_le_bytes());
        a[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        a[8] = 1;
        a[9] = name_units.len() as u8;
        a[0x0A..0x0C].copy_from_slice(&(name_off as u16).to_le_bytes());
        a[0x0C..0x0E].copy_from_slice(&d.flags.to_le_bytes());
        for (i, u) in name_units.iter().enumerate() {
            a[name_off + i * 2..name_off + i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        a[0x20..0x22].copy_from_slice(&(runs_off as u16).to_le_bytes());
        a[0x28..0x30].copy_from_slice(&d.allocated.to_le_bytes());
        a[0x30..0x38].copy_from_slice(&d.size.to_le_bytes());
        a[0x38..0x40].copy_from_slice(&d.size.to_le_bytes());
        if let Some(t) = d.total_allocated {
            a[0x40..0x48].copy_from_slice(&t.to_le_bytes());
        }
        a[runs_off..runs_off + d.runs.len()].copy_from_slice(d.runs);
        a
    }

    /// 拼出一条完整记录并施加 update sequence 保护，就像它在磁盘上的样子。
    pub fn record(number: u32, flags: u16, base: u64, attrs: &[Vec<u8>]) -> Vec<u8> {
        let mut r = vec![0u8; RECORD_SIZE];
        let usa_off = 0x30usize;
        let usa_count = RECORD_SIZE / 512 + 1;
        let first_attr = align8(usa_off + usa_count * 2);
        r[0..4].copy_from_slice(b"FILE");
        r[4..6].copy_from_slice(&(usa_off as u16).to_le_bytes());
        r[6..8].copy_from_slice(&(usa_count as u16).to_le_bytes());
        r[0x14..0x16].copy_from_slice(&(first_attr as u16).to_le_bytes());
        r[0x16..0x18].copy_from_slice(&flags.to_le_bytes());
        r[0x20..0x28].copy_from_slice(&base.to_le_bytes());
        r[0x2C..0x30].copy_from_slice(&number.to_le_bytes());
        let mut off = first_attr;
        for a in attrs {
            r[off..off + a.len()].copy_from_slice(a);
            off += a.len();
        }
        r[off..off + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        let used = align8(off + 8);
        r[0x18..0x1C].copy_from_slice(&(used as u32).to_le_bytes());
        r[0x1C..0x20].copy_from_slice(&(RECORD_SIZE as u32).to_le_bytes());

        let usn = [0x07, 0x00];
        r[usa_off..usa_off + 2].copy_from_slice(&usn);
        for i in 0..usa_count - 1 {
            let end = (i + 1) * 512;
            let saved = usa_off + 2 * (i + 1);
            r[saved] = r[end - 2];
            r[saved + 1] = r[end - 1];
            r[end - 2..end].copy_from_slice(&usn);
        }
        r
    }

    fn align8(n: usize) -> usize {
        n.div_ceil(8) * 8
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    const IN_USE: u16 = RECORD_IN_USE;

    #[test]
    fn parses_names_and_streams() {
        // 2024-01-02 03:04:05 UTC 和 2025-06-07 08:09:10 UTC。
        let ft = |unix: u64| (unix + 11_644_473_600) * 10_000_000;
        let mut si = vec![0u8; 0x48];
        si[0x08..0x10].copy_from_slice(&ft(1_704_164_645).to_le_bytes());
        si[0x18..0x20].copy_from_slice(&ft(1_749_283_750).to_le_bytes());
        let mut rec = record(
            42,
            IN_USE,
            0,
            &[
                resident(ATTR_STANDARD_INFORMATION, None, &si),
                file_name(5, "PROGRA~1", NAMESPACE_DOS),
                file_name(5, "报告 2026.docx", 1),
                resident(ATTR_DATA, None, b"hello"),
                non_resident(&NonResident {
                    name: Some("Zone.Identifier"),
                    size: 3_000,
                    allocated: 4_096,
                    flags: 0,
                    total_allocated: None,
                    runs: &[0x11, 0x01, 0x20, 0x00],
                }),
            ],
        );
        let r = parse_record(&mut rec, false).unwrap().unwrap();
        assert!(!r.is_dir);
        assert_eq!(r.base, 0);
        assert_eq!((r.modified, r.accessed), (1_704_164_645, 1_749_283_750));
        assert_eq!(filetime_to_unix(0), 0, "before 1970");
        assert_eq!(r.names.len(), 2);
        assert_eq!(
            r.names[1],
            FileName {
                parent: 5,
                namespace: 1,
                name: "报告 2026.docx".into()
            }
        );
        assert_eq!(
            r.streams[0],
            DataStream {
                named: false,
                size: Some(5),
                clusters: 0,
                runs: None
            }
        );
        assert_eq!(r.streams[1].clusters, 1);
        assert!(r.streams[1].named);
    }

    #[test]
    fn sparse_counts_only_allocated_runs() {
        let mut rec = record(
            7,
            IN_USE,
            0,
            &[non_resident(&NonResident {
                name: None,
                size: 1_000_000,
                allocated: 1_048_576,
                flags: 0x8000,
                total_allocated: Some(4_096),
                runs: &[0x11, 0x01, 0x20, 0x01, 0xFF, 0x00],
            })],
        );
        let r = parse_record(&mut rec, true).unwrap().unwrap();
        assert_eq!(r.streams[0].size, Some(1_000_000));
        assert_eq!(r.streams[0].clusters, 1);
        assert_eq!(r.streams[0].runs.as_ref().unwrap().len(), 2);
    }

    /// `$BadClus:$Bad`：属性头声称占满整个卷，没有稀疏标记，但只有一个稀疏 run。
    #[test]
    fn unflagged_sparse_stream_allocates_nothing() {
        let volume = 200u64 << 30;
        let mut rec = record(
            8,
            IN_USE,
            0,
            &[non_resident(&NonResident {
                name: Some("$Bad"),
                size: volume,
                allocated: volume,
                flags: 0,
                total_allocated: None,
                runs: &[0x04, 0x00, 0x00, 0x40, 0x03, 0x00],
            })],
        );
        let r = parse_record(&mut rec, false).unwrap().unwrap();
        assert_eq!(r.streams[0].clusters, 0);
    }

    /// 被拆到扩展记录里的后续段：没有逻辑大小，但它的簇要计入占用空间。
    #[test]
    fn continuation_segment_has_clusters_but_no_size() {
        let mut attr = non_resident(&NonResident {
            name: None,
            size: 0,
            allocated: 0,
            flags: 0,
            total_allocated: None,
            runs: &[0x11, 0x05, 0x40, 0x00],
        });
        attr[0x10..0x18].copy_from_slice(&100u64.to_le_bytes()); // lowest VCN = 100
        let mut rec = record(30, IN_USE, 12, &[attr]);
        let r = parse_record(&mut rec, false).unwrap().unwrap();
        assert_eq!(r.streams[0].size, None);
        assert_eq!(r.streams[0].clusters, 5);
    }

    #[test]
    fn skips_unused_and_blank_records() {
        let mut unused = record(9, 0, 0, &[file_name(5, "deleted.txt", 1)]);
        assert_eq!(parse_record(&mut unused, false).unwrap(), None);
        let mut blank = vec![0u8; RECORD_SIZE];
        assert_eq!(parse_record(&mut blank, false).unwrap(), None);
    }

    #[test]
    fn detects_torn_write() {
        let mut rec = record(3, IN_USE, 0, &[file_name(5, "a", 1)]);
        rec[1022] ^= 0xFF; // 第二个 512 字节块末尾的序列号被破坏
        assert!(matches!(
            parse_record(&mut rec, false),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn extension_record_points_to_base() {
        let mut rec = record(
            20,
            IN_USE,
            17 | (2 << 48),
            &[resident(ATTR_DATA, Some("ads"), b"xy")],
        );
        let r = parse_record(&mut rec, false).unwrap().unwrap();
        assert_eq!(r.base, 17);
    }
}
