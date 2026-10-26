//! 文件分类。
//!
//! 先看所在目录：位于 `node_modules`、`.git`、`target` 这类开发缓存目录下的一律算开发缓存；
//! 再看扩展名；`$` 开头的 NTFS 元文件和页面文件、休眠文件算系统文件。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    System,
    App,
    Dev,
    Media,
    Package,
    Document,
    Other,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::System,
        Category::App,
        Category::Dev,
        Category::Media,
        Category::Package,
        Category::Document,
        Category::Other,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    /// 前端使用的标识。
    pub const fn key(self) -> &'static str {
        match self {
            Category::System => "system",
            Category::App => "app",
            Category::Dev => "dev",
            Category::Media => "media",
            Category::Package => "package",
            Category::Document => "document",
            Category::Other => "other",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Category::System => "系统文件",
            Category::App => "应用程序",
            Category::Dev => "开发缓存",
            Category::Media => "音视频",
            Category::Package => "安装包与镜像",
            Category::Document => "文档与图片",
            Category::Other => "其他",
        }
    }
}

/// 位于这些目录之下的内容都算开发缓存（不区分大小写）。
const DEV_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    ".gradle",
    ".m2",
    "__pycache__",
    ".pnpm-store",
    ".cargo",
    ".rustup",
    ".nuget",
    ".npm",
    "npm-cache",
    ".venv",
    "venv",
    ".tox",
    ".next",
    ".svelte-kit",
    ".ollama",
    ".cache",
];

/// 目录名是否是开发缓存目录。
pub fn is_dev_dir(name: &str) -> bool {
    DEV_DIRS.iter().any(|d| d.eq_ignore_ascii_case(name))
}

/// 小写扩展名（不含点）；没有扩展名时返回空串。
pub fn extension(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// 按文件名归类。`in_dev_dir` 表示文件位于某个开发缓存目录之下。
///
/// 每个文件都会调用一次，所以不做堆分配：扩展名在栈上的缓冲区里转小写。
pub fn classify(name: &str, in_dev_dir: bool) -> Category {
    if in_dev_dir {
        return Category::Dev;
    }
    if name.starts_with('$')
        || ["pagefile.sys", "hiberfil.sys", "swapfile.sys"]
            .iter()
            .any(|s| s.eq_ignore_ascii_case(name))
    {
        return Category::System;
    }
    let mut buf = [0u8; 16];
    let ext = match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() && name.len() - i - 1 <= buf.len() => {
            let raw = &name.as_bytes()[i + 1..];
            buf[..raw.len()].copy_from_slice(raw);
            buf[..raw.len()].make_ascii_lowercase();
            std::str::from_utf8(&buf[..raw.len()]).unwrap_or("")
        }
        _ => "",
    };
    match ext {
        "sys" | "drv" | "mui" | "cat" | "etl" | "evtx" | "regtrans-ms" | "blf" | "dmp" => {
            Category::System
        }
        "exe" | "dll" | "ocx" | "cpl" | "scr" | "msix" | "appx" | "pak" | "asar" => Category::App,
        // 编译产物、包和模型文件；源代码不算缓存，归入“其他”。
        "rlib" | "rmeta" | "pdb" | "lib" | "obj" | "o" | "a" | "class" | "jar" | "pyc" | "whl"
        | "nupkg" | "ilk" | "idb" | "pch" | "ipch" | "gguf" | "safetensors" | "onnx" | "pt"
        | "ckpt" => Category::Dev,
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "ts" | "mpg" | "mpeg"
        | "mp3" | "flac" | "wav" | "m4a" | "aac" | "ogg" | "wma" | "ape" => Category::Media,
        "iso" | "img" | "msi" | "cab" | "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "xz"
        | "bz2" | "zst" | "vhd" | "vhdx" | "vmdk" | "wim" | "esd" | "dmg" => Category::Package,
        "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "pdf" | "txt" | "md" | "rtf" | "odt"
        | "csv" | "epub" | "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "heic" | "tif"
        | "tiff" | "raw" | "cr2" | "nef" | "arw" | "dng" | "psd" | "ai" | "svg" => {
            Category::Document
        }
        _ => Category::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_extension() {
        assert_eq!(classify("movie.MKV", false), Category::Media);
        assert_eq!(classify("Win11_24H2.iso", false), Category::Package);
        assert_eq!(classify("报告.docx", false), Category::Document);
        assert_eq!(classify("setup.exe", false), Category::App);
        assert_eq!(classify("model.gguf", false), Category::Dev);
        assert_eq!(classify("README", false), Category::Other);
        assert_eq!(classify(".gitignore", false), Category::Other);
    }

    #[test]
    fn system_files_and_dev_dirs() {
        assert_eq!(classify("$MFT", false), Category::System);
        assert_eq!(classify("pagefile.sys", false), Category::System);
        assert_eq!(classify("photo.png", true), Category::Dev);
        assert!(is_dev_dir("node_modules"));
        assert!(is_dev_dir("Target"));
        assert!(!is_dev_dir("targets"));
    }

    #[test]
    fn extension_edge_cases() {
        assert_eq!(extension("a.tar.GZ"), "gz");
        assert_eq!(extension("noext"), "");
        assert_eq!(extension("trailing."), "");
        assert_eq!(extension(".hidden"), "");
    }
}
