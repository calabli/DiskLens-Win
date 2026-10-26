//! 快照：把扫描得到的文件树连同一段描述信息存成一个文件，以及读回来。
//!
//! 文件格式：`DLSNAP01`（8 字节）、描述信息字节数（u32 小端）、描述信息（JSON）、
//! zstd 压缩的文件树（见 [`FileTree::write_to`]，带校验和）。
//! 描述信息放在压缩数据前面，列出快照时只读文件头，不用解压整棵树。

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

use dl_tree::FileTree;
use serde::Serialize;
use serde::de::DeserializeOwned;

const MAGIC: &[u8; 8] = b"DLSNAP01";
/// 描述信息的长度上限，防止损坏的文件让我们分配巨量内存。
const MAX_META: u32 = 1 << 20;
/// 速度优先：百万级节点的树压缩只需零点几秒，体积约为原来的三分之一。
const LEVEL: i32 = 1;

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("快照文件损坏：{msg}"))
}

/// 把 `meta` 和 `tree` 写入 `path`。先写到临时文件再改名，写到一半失败不会破坏原有快照。
pub fn save<M: Serialize>(path: &Path, meta: &M, tree: &FileTree) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let result = write(&tmp, meta, tree).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write<M: Serialize>(path: &Path, meta: &M, tree: &FileTree) -> io::Result<()> {
    let meta = serde_json::to_vec(meta)?;
    let mut file = BufWriter::new(File::create(path)?);
    file.write_all(MAGIC)?;
    file.write_all(&(meta.len() as u32).to_le_bytes())?;
    file.write_all(&meta)?;

    let mut encoder = zstd::Encoder::new(file, LEVEL)?;
    encoder.include_checksum(true)?;
    let mut body = BufWriter::new(encoder);
    tree.write_to(&mut body)?;
    let file = body.into_inner().map_err(|e| e.into_error())?.finish()?;
    file.into_inner().map_err(|e| e.into_error())?.sync_all()
}

fn read_header<M: DeserializeOwned>(r: &mut impl Read) -> io::Result<M> {
    let mut magic = [0; 8];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(invalid("文件头不对"));
    }
    let mut len = [0; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len);
    if len > MAX_META {
        return Err(invalid("描述信息过长"));
    }
    let mut meta = vec![0; len as usize];
    r.read_exact(&mut meta)?;
    serde_json::from_slice(&meta).map_err(|_| invalid("描述信息不对"))
}

/// 只读快照的描述信息。
pub fn read_meta<M: DeserializeOwned>(path: &Path) -> io::Result<M> {
    read_header(&mut BufReader::new(File::open(path)?))
}

/// 读回整个快照。
pub fn load<M: DeserializeOwned>(path: &Path) -> io::Result<(M, FileTree)> {
    let mut file = BufReader::new(File::open(path)?);
    let meta = read_header(&mut file)?;
    let mut decoder = zstd::Decoder::with_buffer(file)?;
    let tree = FileTree::read_from(&mut decoder)?;
    // 读到压缩流末尾，zstd 才会核对校验和。
    if io::copy(&mut decoder, &mut io::sink())? != 0 {
        return Err(invalid("文件树后面有多余数据"));
    }
    Ok((meta, tree))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::path::PathBuf;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Meta {
        root: String,
        at: u64,
    }

    fn tree() -> FileTree {
        let mut t = FileTree::new(r"D:\");
        let d = t.add_dir(0, "照片");
        for i in 0..1_000 {
            t.add_file(d, &format!("IMG_{i:04}.jpg"), 3_000_000 + i, 3_002_368);
        }
        t.finalize();
        t
    }

    /// 测试文件放在仓库的 target 目录下（D 盘），不碰系统盘。
    fn tmp(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp/snapshot");
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn save_and_load() {
        let path = tmp("roundtrip.dlscan");
        let meta = Meta {
            root: r"D:\".into(),
            at: 42,
        };
        let t = tree();
        save(&path, &meta, &t).unwrap();

        assert_eq!(read_meta::<Meta>(&path).unwrap(), meta);
        let (m, back) = load::<Meta>(&path).unwrap();
        assert_eq!(m, meta);
        assert_eq!(back.len(), t.len());
        assert_eq!(back.size(0), t.size(0));
        assert_eq!(back.path(back.len() as u32 - 1), t.path(t.len() as u32 - 1));
        assert!(!path.with_extension("tmp").exists());
        // 名称高度重复，压缩后应明显变小。
        let mut raw = Vec::new();
        t.write_to(&mut raw).unwrap();
        assert!(fs::metadata(&path).unwrap().len() < raw.len() as u64 / 2);
    }

    #[test]
    fn detects_corruption() {
        let path = tmp("corrupt.dlscan");
        save(
            &path,
            &Meta {
                root: "x".into(),
                at: 1,
            },
            &tree(),
        )
        .unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        fs::write(&path, &bytes).unwrap();
        assert!(load::<Meta>(&path).is_err());

        fs::write(&path, b"not a snapshot").unwrap();
        assert!(read_meta::<Meta>(&path).is_err());
    }
}
