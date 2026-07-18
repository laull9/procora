//! 配置表单与子弹窗共享的布局和焦点样式。

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
};

/// 返回当前焦点边框和选中行的样式。
pub(super) fn focus_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// 将百分比宽度和固定高度居中为弹窗区域。
pub(super) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width.min(100)) / 2),
            Constraint::Percentage(width.min(100)),
            Constraint::Percentage((100 - width.min(100)) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(horizontal[1])[1]
}
