//! 配置表单与子弹窗共享的布局和焦点样式。

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
};

/// 返回当前焦点边框和选中行的样式。
pub(super) fn focus_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// 将百分比宽度和固定高度居中，并在窄屏使用完整可用宽度。
pub(super) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let popup_width = if area.width < 48 {
        area.width
    } else {
        area.width
            .saturating_mul(width.min(100))
            .saturating_div(100)
    };
    let popup_height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(popup_width) / 2,
        y: area.y + area.height.saturating_sub(popup_height) / 2,
        width: popup_width,
        height: popup_height,
    }
}
