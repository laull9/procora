//! 按终端宽度选择完整、不被截断的底栏按键提示。

/// 从详细到精简的候选中选择终端宽度可完整容纳的一项。
pub(crate) fn adaptive(candidates: &[String], width: u16) -> String {
    let width = usize::from(width);
    candidates
        .iter()
        .find(|candidate| super::text_view::width(candidate) <= width)
        .cloned()
        .or_else(|| candidates.last().cloned())
        .unwrap_or_default()
}

/// 使用统一分隔符连接一组按键提示。
pub(crate) fn join(hints: &[&str]) -> String {
    hints.join("  ")
}
