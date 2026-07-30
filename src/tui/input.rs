//! TUI 跨终端输入事件的归一化规则。

use crossterm::event::KeyEventKind;

/// 返回按键事件是否应触发一次界面动作。
pub(super) const fn key_activates(kind: KeyEventKind) -> bool {
    matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyEventKind;

    use super::key_activates;

    #[test]
    // 增强键盘协议和Windows终端产生的重复事件应继续驱动长按操作。
    fn repeated_keys_remain_actionable() {
        assert!(key_activates(KeyEventKind::Press));
        assert!(key_activates(KeyEventKind::Repeat));
        assert!(!key_activates(KeyEventKind::Release));
    }
}
