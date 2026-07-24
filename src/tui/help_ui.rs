//! 总览与服务详情共享的快捷键帮助浮层。

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use super::ui_support::display_color_for;

/// 快捷键帮助浮层的显示状态。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum HelpVisibility {
    /// 不显示帮助。
    #[default]
    Hidden,
    /// 显示帮助。
    Visible,
}

impl HelpVisibility {
    /// 打开帮助浮层。
    pub(crate) const fn show(&mut self) {
        *self = Self::Visible;
    }

    /// 关闭帮助浮层。
    pub(crate) const fn hide(&mut self) {
        *self = Self::Hidden;
    }

    /// 返回帮助浮层是否正在显示。
    pub(crate) const fn visible(self) -> bool {
        matches!(self, Self::Visible)
    }
}

/// 构造一行带高亮键位的帮助文本。
pub(crate) fn key_line(
    keys: impl Into<String>,
    description: impl Into<String>,
    plain: bool,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<16}", keys.into()),
            Style::default()
                .fg(display_color_for(plain, Color::Yellow))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(description.into()),
    ])
}

/// 在当前页面上方绘制居中的快捷键帮助。
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    mut lines: Vec<Line<'static>>,
    plain: bool,
) {
    if area.width < 24 || area.height < 6 {
        frame.render_widget(
            Paragraph::new("? / Esc 关闭帮助")
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL).title("帮助")),
            area,
        );
        return;
    }
    lines.push(Line::default());
    lines.push(key_line("? / Esc / q", "关闭帮助", plain));
    let desired_width = if area.width >= 76 {
        72
    } else {
        area.width.saturating_sub(2)
    };
    let desired_height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .min(area.height.saturating_sub(2));
    let popup = centered(area, desired_width, desired_height.max(5));
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} ")),
        ),
        popup,
    );
}

/// 返回指定尺寸的居中矩形。
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(width) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(vertical[1])[1]
}
