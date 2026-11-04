# DiskLens

Windows 磁盘空间分析与管理工具：直读 NTFS MFT 快速扫描，用矩形树图和旭日图展示占用，并提供安全的清理、查重功能。

官网：<https://calabli.github.io/DiskLens-Win/>

## 功能

- **扫描**：以管理员身份扫描整个 NTFS 磁盘时直读 MFT；其他情况普通遍历，边扫边显示最大的文件夹。结果缓存在本机，下次打开按 USN 日志只更新变化。
- **查看**：矩形树图（可嵌套 1～3 层）和旭日图，按七种类型上色；列表可选列、排序；按类型、修改时间、大小分组统计。
- **搜索**：`size:>1gb`、`ext:iso;zip`、`type:音视频`、`dm:>7d`、`da:<1y`、`file:` / `folder:` 等条件，搜索时图上没命中的方块变灰。
- **清理**：二十多条内置规则，每条写明删除后会怎样、分级标出风险；对应程序（如浏览器）开着时不清理。默认进回收站，系统关键位置受保护，每次操作写日志。
- **整理工具**：重复文件（可替换为硬链接）、相似图片、空文件夹与失效链接、大文件与长期未用的文件、收集篮。
- **其他**：移到其他磁盘并留连接点、存储详情（页面文件、休眠文件、还原点、WinSxS 等）、读不到的文件夹及一键以管理员身份重新扫描、浅色 / 深色外观。

## 目录结构

| 路径 | 内容 |
| --- | --- |
| `crates/dl-tree` | 紧凑的内存文件树、大小汇总、树图与旭日图布局 |
| `crates/dl-ntfs` | NTFS 直读：解析 MFT 组装文件树，按 USN 日志增量刷新 |
| `crates/dl-walk` | 普通目录遍历（非 NTFS 卷、网络盘、未提权时使用） |
| `crates/dl-analyze` | 分类、搜索语法、统计、重复文件、相似图片、空文件夹与失效链接 |
| `crates/dl-clean` | 受保护路径、删除、硬链接替换、移动并留连接点、清理规则、操作日志 |
| `crates/dl-snapshot` | 扫描结果的保存与读取（zstd 压缩） |
| `app/src-tauri` | Tauri 2 外壳，把内核包装成命令 |
| `app/src` | 前端（SvelteKit 静态模式 + Svelte 5 + TypeScript） |
| `rules/default.yaml` | 内置清理规则，写法见文件开头的说明 |
| `tests/fixtures` | NTFS 测试卷的创建、挂载、删除脚本 |
| `docs` | 产品官网（单个静态页面，由 GitHub Pages 从 `main` 的 `/docs` 发布） |

## 开发环境

- Rust stable（MSVC 工具链，见 `rust-toolchain.toml`）、VS 生成工具（C++ 工作负载）、Windows SDK
- Node.js 24、pnpm

## 常用命令

```powershell
# 启动开发版（在 app 目录下）
pnpm install
pnpm tauri dev

# 启动后直接扫描某个位置，而不是打开上次的结果
pnpm tauri dev -- -- --scan 'D:\'

# 打安装包（在 app 目录下），输出在 target\release\bundle\nsis 和 msi
pnpm tauri build

# Rust 检查与测试（在仓库根目录下）
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 前端检查（在 app 目录下）
pnpm check
pnpm lint
```

推送到 `main` 或提交 PR 时，GitHub Actions 会运行上面全部检查（见 `.github/workflows/ci.yml`）。

## 运行时的数据

都在本机，程序不联网：

| 位置 | 内容 |
| --- | --- |
| `%LOCALAPPDATA%\DiskLens\cache\*.dlscan` | 每个扫描位置上次的结果 |
| `%LOCALAPPDATA%\DiskLens\cache\*.dlhash` | 重复文件和相似图片的哈希缓存 |
| `%LOCALAPPDATA%\DiskLens\logs` | 删除、硬链接替换、移动的操作日志 |

## 测试卷

MFT 直读和 USN 刷新的测试要用到一块 NTFS 测试卷。以下脚本都要在管理员 PowerShell 中运行，详见各脚本开头的说明：

- `tests/fixtures/New-TestVolume.ps1`：在 `D:\DiskLensTest` 下创建测试卷（VHDX，挂载为 `T:`），放入硬链接、连接点、压缩与稀疏文件等边界情况。
- `tests/fixtures/Mount-TestVolume.ps1`：重启后重新挂载。
- `tests/fixtures/Remove-TestVolume.ps1`：卸下测试卷；加 `-DeleteFile` 同时删除 VHDX 文件。

需要测试卷或管理员权限的测试默认跳过（`#[ignore]`）。挂好测试卷后，在管理员终端里这样运行：

```powershell
$env:DL_TEST_VOLUME = 'T'
cargo test --workspace -- --ignored
```
