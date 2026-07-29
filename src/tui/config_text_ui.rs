//! 配置编辑器高级文本模式的自适应正文与引导。

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::{ConfigEditor, config_highlight};

/// 绘制高级文本编辑模式，并仅在宽屏保留说明侧栏。
pub(super) fn render(frame: &mut Frame<'_>, area: Rect, editor: &ConfigEditor) {
    let columns = if area.width >= 92 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100), Constraint::Length(0)])
            .split(area)
    };
    render_editor(frame, columns[0], editor);
    if columns[1].width > 0 {
        render_guide(frame, columns[1]);
    }
}

/// 绘制带行号的文本缓冲区并设置终端光标。
fn render_editor(frame: &mut Frame<'_>, area: Rect, editor: &ConfigEditor) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let content_width = usize::from(area.width.saturating_sub(7));
    let mut editor = editor.clone();
    editor.ensure_visible(inner_height);
    editor.ensure_horizontal_visible(content_width);
    let scroll = editor.scroll();
    let highlighted = config_highlight::highlighted_lines(editor.format(), editor.lines())
        .into_iter()
        .enumerate()
        .skip(scroll)
        .take(inner_height)
        .collect::<Vec<_>>();
    let numbers = highlighted
        .iter()
        .map(|(index, _)| {
            Line::styled(
                format!("{:>4} ", index + 1),
                Style::default().fg(Color::DarkGray),
            )
        })
        .collect::<Vec<_>>();
    let lines = highlighted
        .into_iter()
        .map(|(_, spans)| Line::from(spans))
        .collect::<Vec<_>>();
    let title = if area.width >= 32 {
        "高级文本配置 · F1 表单"
    } else {
        "文本 · F1表单"
    };
    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(5), Constraint::Min(1)])
        .split(inner);
    frame.render_widget(Paragraph::new(numbers), columns[0]);
    frame.render_widget(
        Paragraph::new(lines).scroll((
            0,
            u16::try_from(editor.horizontal_scroll()).unwrap_or(u16::MAX),
        )),
        columns[1],
    );
    let (row, column) = editor.cursor();
    if row >= scroll && row < scroll + inner_height {
        let display_column = editor.lines().nth(row).map_or(column, |line| {
            Line::from(line.chars().take(column).collect::<String>()).width()
        });
        let x = columns[1].x
            + u16::try_from(display_column.saturating_sub(editor.horizontal_scroll()))
                .unwrap_or(u16::MAX);
        let y = area.y + 1 + u16::try_from(row - scroll).unwrap_or(u16::MAX);
        frame.set_cursor_position((x.min(columns[1].right().saturating_sub(1)), y));
    }
}

/// 绘制完整配置文本模式的字段说明。
fn render_guide(frame: &mut Frame<'_>, area: Rect) {
    let guide = [
        Line::styled("表单优先", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("F1 返回结构化表单"),
        Line::raw("Task、依赖和常用策略均可弹窗编辑"),
        Line::raw(""),
        Line::styled("高级字段", Style::default().add_modifier(Modifier::BOLD)),
        Line::styled("管理依赖", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("dependencies.<id>: https://...（一行即可）"),
        Line::raw("对象写法可选 source / version / mirrors"),
        Line::raw("checksum / unpack / kind / path"),
        Line::raw("verify.command / args / contains"),
        Line::raw("${dependency.<id>}"),
        Line::raw(""),
        Line::styled("按键", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("Ctrl-S 校验并保存"),
        Line::raw("Esc / Ctrl-C 退出"),
        Line::raw("Tab 插入两个空格"),
    ];
    frame.render_widget(
        Paragraph::new(guide.to_vec())
            .wrap(Wrap { trim: false })
            .block(Block::default().title("配置引导").borders(Borders::ALL)),
        area,
    );
}

/// 根据反馈文本选择状态颜色。
pub(super) fn message_style(message: &str) -> Style {
    if message.starts_with("配置无效")
        || message.starts_with("保存失败")
        || message.starts_with("表单输出失败")
    {
        Style::default().fg(Color::Red)
    } else if message.starts_with("已保存") {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Yellow)
    }
}
