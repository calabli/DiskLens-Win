//! 清理：先检查受保护路径，再删除（默认进回收站）或用硬链接替换重复文件，每一步写入操作日志。
//!
//! - [`rules`]：清理规则（YAML），在扫描结果里找出可以清理的位置
//! - [`protect`]：哪些路径不能直接删除
//! - [`delete`]：删除到回收站或彻底删除
//! - [`link`]：用硬链接替换内容相同的文件
//! - [`relocate`]：把文件夹移到另一个磁盘，原位置留下连接点
//! - [`log`]：操作日志（JSON Lines）

pub mod delete;
pub mod link;
pub mod log;
pub mod protect;
pub mod relocate;
pub mod rules;

use std::path::PathBuf;

pub use delete::Mode;
pub use log::{LogEntry, OpLog};
pub use protect::Protector;

/// 要删除的一项。`size` 只用于日志和汇总。
#[derive(Debug, Clone)]
pub struct Target {
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Deleted,
    /// 已替换为硬链接。
    Linked,
    /// 已移到另一个磁盘，原位置是连接点。
    Moved,
    /// 受保护，没有尝试删除。
    Blocked(String),
    Failed(String),
}

/// 逐项检查并删除，每项都写日志（被拦截的也写）。日志写不进去不影响删除结果，
/// 但会作为第二个返回值报告。
pub fn run(
    targets: &[Target],
    mode: Mode,
    protector: &Protector,
    log: &OpLog,
) -> (Vec<Outcome>, Option<std::io::Error>) {
    run_with(targets, mode, &|p| protector.check(p), log)
}

/// 同 [`run`]，用 `check` 判断能否删除（返回拦截原因）。清理规则用它放行规则指定的位置。
pub fn run_with(
    targets: &[Target],
    mode: Mode,
    check: &dyn Fn(&std::path::Path) -> Option<String>,
    log: &OpLog,
) -> (Vec<Outcome>, Option<std::io::Error>) {
    let mut log_error = None;
    let outcomes = targets
        .iter()
        .map(|t| {
            let outcome = match check(&t.path) {
                Some(reason) => Outcome::Blocked(reason),
                None => match delete::delete(&t.path, mode) {
                    Ok(()) => Outcome::Deleted,
                    Err(e) => Outcome::Failed(e),
                },
            };
            if let Err(e) = log.append(&LogEntry::new(&t.path, t.size, mode, &outcome)) {
                log_error.get_or_insert(e);
            }
            outcome
        })
        .collect();
    (outcomes, log_error)
}

/// 用硬链接替换：每一对里把 `.1` 替换为指向 `.0` 的硬链接。`.1` 受保护时不替换。
/// 每项都写日志，规则同 [`run`]。
pub fn link_run(
    pairs: &[(Target, Target)],
    protector: &Protector,
    log: &OpLog,
) -> (Vec<Outcome>, Option<std::io::Error>) {
    let mut log_error = None;
    let outcomes = pairs
        .iter()
        .map(|(source, target)| {
            let outcome = match protector.check(&target.path) {
                Some(reason) => Outcome::Blocked(reason),
                None => match link::replace_with_hardlink(&source.path, &target.path) {
                    Ok(_) => Outcome::Linked,
                    Err(e) => Outcome::Failed(e),
                },
            };
            let entry = LogEntry::link(&target.path, &source.path, target.size, &outcome);
            if let Err(e) = log.append(&entry) {
                log_error.get_or_insert(e);
            }
            outcome
        })
        .collect();
    (outcomes, log_error)
}
