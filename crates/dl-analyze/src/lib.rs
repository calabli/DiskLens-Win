//! 对文件树做的分析：
//! - [`category`]：按扩展名和所在目录把文件归类（用于着色和类型统计）
//! - [`query`]：搜索语法，例如 `size:>1gb ext:iso;zip 安装`
//! - [`stats`]：子树的类型统计、每个子项的主要类型、按条件搜索最大的若干项
//! - [`dupes`]：重复文件（按大小、部分哈希、全文哈希三步判定）
//! - [`findings`]：空文件夹、空文件、失效的链接和快捷方式

pub mod category;
pub mod dupes;
pub mod findings;
pub mod query;
pub mod stats;

pub use category::Category;
pub use query::{ParseError, Query};
pub use stats::{CategoryTotals, ChildBreakdown, breakdown, dominant_dirs, search};
