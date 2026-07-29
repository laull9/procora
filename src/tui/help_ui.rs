//! 总览与服务详情共享的快捷键帮助浮层。

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use super::{text_view, ui_support::display_color_for};

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
    if area.width < 16 || area.height < 4 {
        frame.render_widget(
            Paragraph::new("? / Esc 关闭帮助")
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL).title("帮助")),
            area,
        );
        return;
    }
    let desired_width = if area.width >= 76 {
        72
    } else {
        area.width.saturating_sub(2)
    };
    let compact = desired_width < 48;
    if compact {
        let line_width = usize::from(desired_width.saturating_sub(2));
        lines = lines
            .into_iter()
            .map(|line| compact_line(&line, line_width))
            .collect();
    } else {
        lines.push(Line::default());
        lines.push(key_line("? / Esc / q", "关闭帮助", plain));
    }
    let desired_height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .min(area.height.saturating_sub(2));
    let popup = centered(area, desired_width, desired_height.max(5));
    frame.render_widget(Clear, popup);
    let title = text_view::clipped(title, 0, usize::from(popup.width.saturating_sub(4)));
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "));
    if compact {
        block = block.title_bottom("?/Esc 关闭");
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        popup,
    );
}

/// 在窄帮助层中把固定双栏压成一行，并保留键位强调。
fn compact_line(line: &Line<'static>, width: usize) -> Line<'static> {
    let Some(keys) = line.spans.first() else {
        return Line::default();
    };
    let key_text = keys.content.trim();
    let key_text = text_view::clipped(key_text, 0, width);
    let key_width = text_view::width(&key_text);
    let description = line
        .spans
        .iter()
        .skip(1)
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let separator = " · ";
    let remaining = width
        .saturating_sub(key_width)
        .saturating_sub(text_view::width(separator));
    if description.is_empty() || remaining == 0 {
        return Line::styled(key_text, keys.style);
    }
    Line::from(vec![
        Span::styled(key_text, keys.style),
        Span::raw(separator),
        Span::raw(text_view::clipped(&description, 0, remaining)),
    ])
}

/// 返回指定尺寸的居中矩形。
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}
