# DiskLens

Windows 磁盘空间分析与管理工具：直读 NTFS MFT 快速扫描，用矩形树图和旭日图展示占用，并提供安全的清理、查重功能。

## 目录结构

| 路径 | 内容 |
| --- | --- |
| `crates/dl-tree` | 紧凑的内存文件树、大小汇总 |
| `crates/dl-walk` | 普通目录遍历（非 NTFS 卷、网络盘、未提权时使用） |
| `app/src-tauri` | Tauri 2 外壳，把内核包装成命令 |
| `app/src` | 前端（SvelteKit 静态模式 + Svelte 5 + TypeScript） |
| `tests/fixtures` | 测试卷生成脚本 |
| `docs` | 产品官网（单个静态页面，可用 GitHub Pages 从 `main` 的 `/docs` 发布） |

后续里程碑会加入 `dl-ntfs`、`dl-analyze`、`dl-clean`、`dl-snapshot`、`dl-cli`。

## 开发环境

- Rust stable（MSVC 工具链）、VS 生成工具（C++ 工作负载）、Windows SDK
- Node.js 24、pnpm

## 常用命令

```powershell
# 启动开发版（在 app 目录下）
pnpm install
pnpm tauri dev

# Rust 检查与测试（在仓库根目录下）
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 前端检查（在 app 目录下）
pnpm check
pnpm lint
```

## 测试卷

`tests/fixtures/New-TestVolume.ps1` 会在 `D:\DiskLensTest` 下创建一个 NTFS 测试卷，并放入硬链接、连接点、压缩与稀疏文件等边界情况。需要在管理员 PowerShell 中运行，详见脚本开头的说明。
