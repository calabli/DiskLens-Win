//! NTFS 引导扇区。

use crate::{Error, Result, le_u16, le_u64};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootSector {
    pub bytes_per_sector: u32,
    pub cluster_size: u64,
    pub total_sectors: u64,
    /// `$MFT` 第一个簇的逻辑簇号。
    pub mft_lcn: u64,
    /// 每条文件记录的字节数，通常是 1024，4K 原生扇区的盘上是 4096。
    pub record_size: u32,
}

impl BootSector {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.get(3..11) != Some(b"NTFS    ") {
            return Err(Error::NotNtfs);
        }
        let bytes_per_sector = u32::from(le_u16(b, 0x0B)?);
        if !bytes_per_sector.is_power_of_two() || !(256..=4096).contains(&bytes_per_sector) {
            return Err(Error::Corrupt("bytes per sector"));
        }

        // 每簇扇区数大于 128 时按 2 的负指数编码，例如 0xF4 表示 2^12。
        let spc_raw = b[0x0D];
        let sectors_per_cluster = match spc_raw {
            0 => return Err(Error::Corrupt("sectors per cluster")),
            1..=0x80 => u64::from(spc_raw),
            _ => 1u64 << (256 - u32::from(spc_raw)),
        };
        let cluster_size = u64::from(bytes_per_sector) * sectors_per_cluster;

        // 每条记录的簇数：正数是簇数，负数 n 表示 2^(-n) 字节。
        let cpr = b[0x40] as i8;
        let record_size = if cpr > 0 {
            u64::from(cpr.unsigned_abs()) * cluster_size
        } else {
            1u64 << u32::from(cpr.unsigned_abs())
        };
        if !record_size.is_power_of_two() || !(256..=65_536).contains(&record_size) {
            return Err(Error::Corrupt("file record size"));
        }

        Ok(Self {
            bytes_per_sector,
            cluster_size,
            total_sectors: le_u64(b, 0x28)?,
            mft_lcn: le_u64(b, 0x30)?,
            record_size: record_size as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boot(bps: u16, spc: u8, cpr: u8) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[3..11].copy_from_slice(b"NTFS    ");
        b[0x0B..0x0D].copy_from_slice(&bps.to_le_bytes());
        b[0x0D] = spc;
        b[0x28..0x30].copy_from_slice(&1_000_000u64.to_le_bytes());
        b[0x30..0x38].copy_from_slice(&786_432u64.to_le_bytes());
        b[0x40] = cpr;
        b
    }

    #[test]
    fn parses_common_layout() {
        let s = BootSector::parse(&boot(512, 8, 0xF6)).unwrap();
        assert_eq!(s.cluster_size, 4096);
        assert_eq!(s.record_size, 1024);
        assert_eq!(s.mft_lcn, 786_432);
    }

    #[test]
    fn parses_4k_native_and_large_clusters() {
        let s = BootSector::parse(&boot(4096, 1, 0xF4)).unwrap();
        assert_eq!(s.cluster_size, 4096);
        assert_eq!(s.record_size, 4096);
        // 0xF4：每簇 2^12 个扇区，即 2 MiB 簇
        let s = BootSector::parse(&boot(512, 0xF4, 0xF6)).unwrap();
        assert_eq!(s.cluster_size, 2 * 1024 * 1024);
    }

    #[test]
    fn rejects_non_ntfs() {
        let mut b = boot(512, 8, 0xF6);
        b[3..11].copy_from_slice(b"EXFAT   ");
        assert!(matches!(BootSector::parse(&b), Err(Error::NotNtfs)));
    }
}
