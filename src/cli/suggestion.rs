//! 命令行拼写建议。

use std::path::Path;

/// 可在顶层执行的公开命令名称。
pub(crate) const TOP_LEVEL: &[&str] = &[
    "init",
    "edit",
    "deps",
    "clean",
    "up",
    "down",
    "status",
    "enable",
    "disable",
    "add",
    "list",
    "history",
    "start",
    "restart",
    "preview",
    "push",
    "apply",
    "stop",
    "remove",
    "show",
    "logs",
    "validate",
    "graph",
    "config",
    "doctor",
    "completions",
];

/// `procora server` 支持的子命令名称。
pub(crate) const SERVER: &[&str] = &[
    "list", "history", "start", "restart", "preview", "apply", "stop", "remove",
];

/// 为不存在的单段路径查找足够相似的命令。
pub(crate) fn for_missing_path<'a>(target: &Path, candidates: &'a [&str]) -> Option<&'a str> {
    if target.exists() || target.components().count() != 1 {
        return None;
    }
    let value = target.to_str()?;
    candidates
        .iter()
        .map(|candidate| (strsim::jaro(value, candidate), *candidate))
        .filter(|(score, _)| *score > 0.7)
        .max_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, candidate)| candidate)
}
