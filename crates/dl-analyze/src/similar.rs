//! 相似图片：感知哈希（pHash）。
//!
//! 每张图缩成 32×32 灰度，做二维 DCT，取左上角 8×8 的低频系数，与其中位数比较得到 64 位指纹。
//! 缩放、另存、重新压缩、轻微调色后的同一张图指纹几乎不变；两张图指纹不同的位数（汉明距离）
//! 不超过阈值就算相似。相似关系可以传递（A 像 B、B 像 C），合并成一组。
//!
//! 指纹按路径、大小和修改时间缓存在 [`PhashCache`] 里，图片没变就不必再解码。

use std::collections::HashMap;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::UNIX_EPOCH;

use dl_tree::{FileTree, NodeId, NodeKind};
use image::{DynamicImage, ImageFormat, ImageReader, Limits};

/// 支持的图片扩展名（小写）。
pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "bmp", "gif", "webp"];
/// 超过这个大小的文件不解码，避免占用过多内存。
const MAX_FILE: u64 = 256 << 20;
/// 解码时最多分配的内存。
const MAX_ALLOC: u64 = 1 << 30;
/// DCT 的输入边长。
const N: usize = 32;

#[derive(Debug, Clone)]
pub struct ImageCandidate {
    pub id: NodeId,
    pub path: PathBuf,
    pub size: u64,
}

/// `root` 子树中不小于 `min_size` 的图片。
pub fn candidates(tree: &FileTree, root: NodeId, min_size: u64) -> Vec<ImageCandidate> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        match tree.kind(n) {
            NodeKind::Dir => stack.extend(tree.children(n)),
            NodeKind::File => {
                let size = tree.size(n);
                if size < min_size.max(1) || size > MAX_FILE {
                    continue;
                }
                let name = tree.name(n).to_ascii_lowercase();
                let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
                if EXTENSIONS.contains(&ext) {
                    out.push(ImageCandidate {
                        id: n,
                        path: PathBuf::from(tree.path(n)),
                        size,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    size: u64,
    modified: u64,
    hash: u64,
    width: u32,
    height: u32,
}

/// 指纹缓存，可在多次查找之间保存到磁盘。
#[derive(Debug, Default)]
pub struct PhashCache {
    entries: Mutex<HashMap<String, Entry>>,
}

const CACHE_MAGIC: &[u8; 8] = b"DLPHASH1";

impl PhashCache {
    pub fn len(&self) -> usize {
        self.entries.lock().expect("phash cache poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 从文件读入。文件不存在或已损坏时得到空缓存。
    pub fn load(path: &Path) -> Self {
        Self::parse(&fs::read(path).unwrap_or_default()).unwrap_or_default()
    }

    fn parse(b: &[u8]) -> Option<Self> {
        if b.get(..8)? != CACHE_MAGIC {
            return None;
        }
        let u32_at = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?));
        let u64_at = |o: usize| Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?));
        let count = u64_at(8)?;
        let mut off = 16;
        let mut map = HashMap::new();
        for _ in 0..count {
            let len = u32_at(off)? as usize;
            let key = std::str::from_utf8(b.get(off + 4..off + 4 + len)?).ok()?;
            off += 4 + len;
            map.insert(
                key.to_string(),
                Entry {
                    size: u64_at(off)?,
                    modified: u64_at(off + 8)?,
                    hash: u64_at(off + 16)?,
                    width: u32_at(off + 24)?,
                    height: u32_at(off + 28)?,
                },
            );
            off += 32;
        }
        (off == b.len()).then(|| Self {
            entries: Mutex::new(map),
        })
    }

    /// 写入文件：先写临时文件再改名。
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let entries = self.entries.lock().expect("phash cache poisoned");
        let mut buf = Vec::new();
        buf.extend_from_slice(CACHE_MAGIC);
        buf.extend_from_slice(&(entries.len() as u64).to_le_bytes());
        for (k, e) in entries.iter() {
            buf.extend_from_slice(&(k.len() as u32).to_le_bytes());
            buf.extend_from_slice(k.as_bytes());
            for v in [e.size, e.modified, e.hash] {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf.extend_from_slice(&e.width.to_le_bytes());
            buf.extend_from_slice(&e.height.to_le_bytes());
        }
        drop(entries);
        let tmp = path.with_extension("tmp");
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&buf)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    }

    fn get(&self, key: &str, size: u64, modified: u64) -> Option<Entry> {
        let entries = self.entries.lock().expect("phash cache poisoned");
        entries
            .get(key)
            .copied()
            .filter(|e| e.size == size && e.modified == modified)
    }

    fn put(&self, key: String, entry: Entry) {
        self.entries
            .lock()
            .expect("phash cache poisoned")
            .insert(key, entry);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarImage {
    pub id: NodeId,
    pub path: PathBuf,
    pub size: u64,
    pub modified_ms: u64,
    pub width: u32,
    pub height: u32,
    pub hash: u64,
}

/// 一组相似的图片，画质最好的（像素最多，其次文件最大）排在最前。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarGroup {
    pub files: Vec<SimilarImage>,
}

impl SimilarGroup {
    /// 只保留第一张时能释放的空间。
    pub fn wasted(&self) -> u64 {
        self.files.iter().skip(1).map(|f| f.size).sum()
    }
}

#[derive(Debug, Default)]
pub struct SimilarReport {
    /// 按可释放空间从大到小。
    pub groups: Vec<SimilarGroup>,
    /// 打不开或解码失败的图片数。
    pub skipped: u64,
    /// 实际解码的图片数（命中缓存的不算）。
    pub decoded: u64,
}

/// 两个指纹不同的位数。
pub fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// 图片的感知哈希。
pub fn phash(img: &DynamicImage) -> u64 {
    let small = img.thumbnail_exact(N as u32, N as u32).to_luma8();
    let px: Vec<f64> = small.pixels().map(|p| f64::from(p.0[0])).collect();
    // 只需要 8×8 个低频系数：cos[u][x] = cos((2x + 1)uπ / 2N)。
    let cos: Vec<[f64; N]> = (0..8)
        .map(|u| {
            let mut row = [0.0; N];
            for (x, c) in row.iter_mut().enumerate() {
                *c = (((2 * x + 1) * u) as f64 * std::f64::consts::PI / (2 * N) as f64).cos();
            }
            row
        })
        .collect();
    // 先对每一行做变换，再对列。
    let mut rows = [[0.0f64; 8]; N];
    for (y, row) in rows.iter_mut().enumerate() {
        for (v, out) in row.iter_mut().enumerate() {
            *out = (0..N).map(|x| px[y * N + x] * cos[v][x]).sum();
        }
    }
    let mut coeffs = [0.0f64; 64];
    for u in 0..8 {
        for v in 0..8 {
            coeffs[u * 8 + v] = (0..N).map(|y| rows[y][v] * cos[u][y]).sum();
        }
    }
    // 直流分量只反映整体亮度，不参与比较。
    let mut ac: Vec<f64> = coeffs[1..].to_vec();
    ac.sort_by(f64::total_cmp);
    let median = ac[ac.len() / 2];
    coeffs
        .iter()
        .enumerate()
        .skip(1)
        .fold(0u64, |h, (i, &c)| if c > median { h | (1 << i) } else { h })
}

fn open(path: &Path) -> io::Result<DynamicImage> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    reader.decode().map_err(io::Error::other)
}

/// 生成预览用的缩略图（PNG），长边不超过 `max` 像素。
pub fn thumbnail_png(path: &Path, max: u32) -> io::Result<Vec<u8>> {
    let img = open(path)?.thumbnail(max, max);
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png)
        .map_err(io::Error::other)?;
    Ok(out.into_inner())
}

/// 在 `candidates` 中找出相似的图片组。`max_distance` 为允许不同的最多位数（0–64）。
/// `progress(已处理, 总数)` 返回 `false` 即取消，此时返回 [`io::ErrorKind::Interrupted`]。
pub fn find_similar(
    candidates: &[ImageCandidate],
    cache: &PhashCache,
    max_distance: u32,
    threads: usize,
    progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> io::Result<SimilarReport> {
    let total = candidates.len() as u64;
    let next = AtomicUsize::new(0);
    let done = AtomicU64::new(0);
    let skipped = AtomicU64::new(0);
    let decoded = AtomicU64::new(0);
    let cancelled = AtomicBool::new(false);

    let mut images: Vec<SimilarImage> = thread::scope(|s| {
        let workers: Vec<_> = (0..threads.clamp(1, candidates.len().max(1)))
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= candidates.len() || cancelled.load(Ordering::Relaxed) {
                            break;
                        }
                        match fingerprint(&candidates[i], cache, &decoded) {
                            Some(img) => mine.push(img),
                            None => {
                                skipped.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if !progress(d, total) {
                            cancelled.store(true, Ordering::Relaxed);
                        }
                    }
                    mine
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("similar worker panicked"))
            .collect()
    });
    if cancelled.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "查找已取消"));
    }
    images.sort_by(|a, b| a.path.cmp(&b.path));

    // 两两比较，距离不超过阈值的并到一组（并查集）。
    let mut parent: Vec<usize> = (0..images.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..images.len() {
        for j in i + 1..images.len() {
            if distance(images[i].hash, images[j].hash) <= max_distance {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                if a != b {
                    parent[b] = a;
                }
            }
        }
    }
    let mut by_root: HashMap<usize, Vec<SimilarImage>> = HashMap::new();
    for (i, img) in images.into_iter().enumerate() {
        by_root.entry(root(&mut parent, i)).or_default().push(img);
    }
    let mut groups: Vec<SimilarGroup> = by_root
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|mut files| {
            files.sort_by(|a, b| {
                (u64::from(b.width) * u64::from(b.height))
                    .cmp(&(u64::from(a.width) * u64::from(a.height)))
                    .then(b.size.cmp(&a.size))
                    .then_with(|| a.path.cmp(&b.path))
            });
            SimilarGroup { files }
        })
        .collect();
    groups.sort_by(|a, b| {
        b.wasted()
            .cmp(&a.wasted())
            .then_with(|| a.files[0].path.cmp(&b.files[0].path))
    });

    Ok(SimilarReport {
        groups,
        skipped: skipped.into_inner(),
        decoded: decoded.into_inner(),
    })
}

fn fingerprint(
    c: &ImageCandidate,
    cache: &PhashCache,
    decoded: &AtomicU64,
) -> Option<SimilarImage> {
    let meta = fs::metadata(&c.path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64);
    let key = c.path.to_string_lossy().into_owned();
    let entry = match cache.get(&key, meta.len(), modified) {
        Some(e) => e,
        None => {
            let img = open(&c.path).ok()?;
            decoded.fetch_add(1, Ordering::Relaxed);
            let e = Entry {
                size: meta.len(),
                modified,
                hash: phash(&img),
                width: img.width(),
                height: img.height(),
            };
            cache.put(key, e);
            e
        }
    };
    Some(SimilarImage {
        id: c.id,
        path: c.path.clone(),
        size: meta.len(),
        modified_ms: modified / 1_000_000,
        width: entry.width,
        height: entry.height,
        hash: entry.hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb, RgbImage};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .unwrap();
            let dir = repo
                .join("target")
                .join("test-tmp")
                .join(format!("similar-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 一幅平滑变化的“风景”：`kind` 不同，图案完全不同。
    fn scene(w: u32, h: u32, kind: u32) -> RgbImage {
        ImageBuffer::from_fn(w, h, |x, y| {
            let (fx, fy) = (x as f64 / w as f64, y as f64 / h as f64);
            let v = match kind {
                0 => ((fx * 6.0).sin() + (fy * 4.0).cos()) * 0.25 + 0.5,
                _ => ((fx * fy * 20.0).sin() * (fy * 9.0 - fx * 3.0).cos()) * 0.5 + 0.5,
            };
            let g = (v * 255.0) as u8;
            Rgb([g, g.saturating_add(30), 255 - g])
        })
    }

    /// 调亮（伽马校正），不会把像素推到饱和。
    fn brighten(img: &RgbImage) -> RgbImage {
        let mut out = img.clone();
        for p in out.pixels_mut() {
            for c in &mut p.0 {
                *c = ((f64::from(*c) / 255.0).powf(0.85) * 255.0) as u8;
            }
        }
        out
    }

    #[test]
    fn same_picture_in_different_forms_is_similar() {
        let base = scene(640, 480, 0);
        let h = |img: &RgbImage| phash(&DynamicImage::ImageRgb8(img.clone()));
        let small = image::imageops::resize(&base, 320, 240, image::imageops::FilterType::Triangle);
        assert!(distance(h(&base), h(&small)) <= 4);
        assert!(distance(h(&base), h(&brighten(&base))) <= 8);
        assert!(distance(h(&base), h(&scene(640, 480, 1))) > 16);
    }

    #[test]
    fn groups_similar_images_and_skips_broken_ones() {
        let t = TempDir::new("group");
        let base = scene(800, 600, 0);
        base.save(t.0.join("原图.png")).unwrap();
        image::imageops::resize(&base, 400, 300, image::imageops::FilterType::Triangle)
            .save(t.0.join("缩小.png"))
            .unwrap();
        // 重新压缩成 JPEG（有损）。
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 60)
            .encode_image(&base)
            .unwrap();
        fs::write(t.0.join("压缩.jpg"), jpg).unwrap();
        brighten(&base).save(t.0.join("调亮.png")).unwrap();
        scene(800, 600, 1).save(t.0.join("另一张.png")).unwrap();
        fs::write(t.0.join("坏的.jpg"), vec![0xFFu8; 5_000]).unwrap();

        let tree = dl_walk::walk(&t.0).unwrap().tree;
        let cands = candidates(&tree, tree.root(), 1);
        assert_eq!(cands.len(), 6);
        let cache = PhashCache::default();
        let report = find_similar(&cands, &cache, 8, 4, &|_, _| true).unwrap();

        assert_eq!(report.groups.len(), 1, "{:#?}", report.groups);
        let names: Vec<String> = report.groups[0]
            .files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 4, "{names:?}");
        assert!(!names.contains(&"另一张.png".to_string()));
        // 像素最多的排在最前，缩小的那张排在最后。
        assert_eq!(
            (
                report.groups[0].files[0].width,
                report.groups[0].files[0].height
            ),
            (800, 600)
        );
        assert_eq!(names[3], "缩小.png");
        assert_eq!(report.skipped, 1);
        assert_eq!(report.decoded, 5);

        // 缓存存盘、读回，第二次不再解码。
        let path = t.0.join("phash.bin");
        cache.save(&path).unwrap();
        let reloaded = PhashCache::load(&path);
        assert_eq!(reloaded.len(), 5);
        let again = find_similar(&cands, &reloaded, 8, 4, &|_, _| true).unwrap();
        assert_eq!(again.decoded, 0);
        assert_eq!(again.groups, report.groups);

        let thumb = thumbnail_png(&t.0.join("原图.png"), 120).unwrap();
        let back = image::load_from_memory(&thumb).unwrap();
        assert_eq!((back.width(), back.height()), (120, 90));
    }

    #[test]
    fn damaged_cache_is_ignored() {
        assert!(PhashCache::parse(b"DLPHASH1\x09\0\0\0\0\0\0\0").is_none());
        assert!(PhashCache::parse(b"nonsense").is_none());
    }
}
