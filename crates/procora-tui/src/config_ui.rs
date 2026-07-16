use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::ConfigEditor;

/// 绘制带字段引导的配置编辑页面。
pub(crate) fn render(frame: &mut Frame<'_>, editor: &ConfigEditor) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());
    let title = Paragraph::new(format!("Procora 配置编辑器 · {}", editor.path().display()))
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, outer[0]);

    let columns = if outer[1].width >= 92 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(outer[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100), Constraint::Length(0)])
            .split(outer[1])
    };
    render_editor(frame, columns[0], editor);
    if columns[1].width > 0 {
        render_guide(frame, columns[1]);
    }
    let footer = Paragraph::new(editor.message())
        .block(Block::default().title("状态").borders(Borders::ALL))
        .style(message_style(editor.message()));
    frame.render_widget(footer, outer[2]);
}

/// 绘制带行号的文本缓冲区并设置终端光标。
fn render_editor(frame: &mut Frame<'_>, area: Rect, editor: &ConfigEditor) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let mut editor = editor.clone();
    editor.ensure_visible(inner_height);
    let scroll = editor.scroll();
    let lines = editor
        .lines()
        .enumerate()
        .skip(scroll)
        .take(inner_height)
        .map(|(index, text)| {
            Line::from(vec![
                Span::styled(
                    format!("{:>4} ", index + 1),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(text),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().title("配置").borders(Borders::ALL)),
        area,
    );
    let (row, column) = editor.cursor();
    if row >= scroll && row < scroll + inner_height {
        let x = area.x + 1 + 5 + u16::try_from(column).unwrap_or(u16::MAX);
        let y = area.y + 1 + u16::try_from(row - scroll).unwrap_or(u16::MAX);
        frame.set_cursor_position((x.min(area.right().saturating_sub(1)), y));
    }
}

/// 绘制核心字段、依赖来源和占位符说明。
fn render_guide(frame: &mut Frame<'_>, area: Rect) {
    let guide = [
        Line::styled("基础", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("version: 1"),
        Line::raw("project: 稳定服务名"),
        Line::raw("tasks.<id>.command / args"),
        Line::raw("depends_on / restart / env / cwd"),
        Line::raw(""),
        Line::styled("管理依赖", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("dependencies.<id>.source"),
        Line::raw("  http(s):// / ssh:// / user@host:/"),
        Line::raw("dependencies.<id>.version"),
        Line::raw("checksum: sha256:<64 hex>"),
        Line::raw("unpack: auto | never"),
        Line::raw("kind: auto | binary | file | directory"),
        Line::raw("path: 归档内相对路径"),
        Line::raw("verify.args / verify.contains"),
        Line::raw(""),
        Line::styled("任务引用", Style::default().add_modifier(Modifier::BOLD)),
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
fn message_style(message: &str) -> Style {
    if message.starts_with("配置无效") || message.starts_with("保存失败") {
        Style::default().fg(Color::Red)
    } else if message.starts_with("已保存") {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Yellow)
    }
}
