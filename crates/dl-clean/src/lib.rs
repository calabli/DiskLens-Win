//! 清理：先检查受保护路径，再删除（默认进回收站），每一步写入操作日志。
//!
//! - [`protect`]：哪些路径不能直接删除
//! - [`delete`]：删除到回收站或彻底删除
//! - [`log`]：操作日志（JSON Lines）

pub mod delete;
pub mod log;
pub mod protect;

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
    let mut log_error = None;
    let outcomes = targets
        .iter()
        .map(|t| {
            let outcome = match protector.check(&t.path) {
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
