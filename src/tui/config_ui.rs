use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use super::{
    ConfigEditor, config_form::FormPane, config_form_dialog::Dialog, config_form_state::FormState,
};

/// 绘制配置编辑器，并按当前模式选择结构化表单或高级文本界面。
pub(crate) fn render(frame: &mut Frame<'_>, editor: &ConfigEditor) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());
    let mode = if editor.is_form_mode() {
        "结构化表单"
    } else {
        "高级文本"
    };
    let title = Paragraph::new(format!(
        "Procora 配置编辑器 · {mode} · {}",
        editor.path().display()
    ))
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, outer[0]);

    if let Some(form) = editor.form().filter(|_| editor.is_form_mode()) {
        render_form(frame, outer[1], form);
    } else {
        render_text_mode(frame, outer[1], editor);
    }
    let footer = Paragraph::new(editor.message())
        .block(Block::default().title("状态").borders(Borders::ALL))
        .style(message_style(editor.message()));
    frame.render_widget(footer, outer[2]);
}

/// 绘制以项目、Task 和管理依赖为核心的结构化编辑页。
fn render_form(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(area);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(5),
            Constraint::Min(5),
        ])
        .split(columns[0]);
    render_project(frame, left[0], form);
    render_tasks(frame, left[1], form);
    render_dependencies(frame, left[2], form);
    render_form_detail(frame, columns[1], form);
    if let Some(dialog) = form.dialog() {
        render_dialog(frame, dialog);
    } else if let Some(name) = form.pending_delete_name() {
        render_delete_confirmation(frame, name);
    }
}

/// 绘制项目基础信息卡片。
fn render_project(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    let focused = form.pane() == FormPane::Project;
    let title = if focused {
        "项目  ← Enter 编辑"
    } else {
        "项目"
    };
    let style = if focused {
        focus_style()
    } else {
        Style::default()
    };
    frame.render_widget(
        Paragraph::new(vec![Line::from(vec![
            Span::styled("名称：", Style::default().fg(Color::DarkGray)),
            Span::raw(form.config().project()),
        ])])
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(style),
        ),
        area,
    );
}

/// 绘制可选择的 Task 列表。
fn render_tasks(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    let items = form
        .config()
        .tasks()
        .map(|(name, task)| ListItem::new(format!("{name}  ·  {}", task.command)))
        .collect::<Vec<_>>();
    let focused = form.pane() == FormPane::Tasks;
    let title = if focused {
        "Tasks  ← Enter 编辑 · n 新建 · d 删除"
    } else {
        "Tasks"
    };
    let mut state = ListState::default();
    if focused && !items.is_empty() {
        state.select(Some(form.selected()));
    }
    let list = List::new(if items.is_empty() {
        vec![ListItem::new("（暂无 Task，按 n 新建）")]
    } else {
        items
    })
    .block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(if focused {
                focus_style()
            } else {
                Style::default()
            }),
    )
    .highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_stateful_widget(list, area, &mut state);
}

/// 绘制可选择的管理依赖列表。
fn render_dependencies(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    let items = form
        .config()
        .dependencies()
        .map(|(name, dependency)| ListItem::new(format!("{name}  ·  {}", dependency.source)))
        .collect::<Vec<_>>();
    let focused = form.pane() == FormPane::Dependencies;
    let title = if focused {
        "管理依赖  ← Enter 编辑 · n 新建 · d 删除"
    } else {
        "管理依赖"
    };
    let mut state = ListState::default();
    if focused && !items.is_empty() {
        state.select(Some(form.selected()));
    }
    let list = List::new(if items.is_empty() {
        vec![ListItem::new("（暂无依赖，按 n 新建）")]
    } else {
        items
    })
    .block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(if focused {
                focus_style()
            } else {
                Style::default()
            }),
    )
    .highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_stateful_widget(list, area, &mut state);
}

/// 绘制当前结构化编辑状态的操作说明。
fn render_form_detail(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    let (section, detail) = match form.pane() {
        FormPane::Project => ("项目", format!("项目名称：{}", form.config().project())),
        FormPane::Tasks => form.config().tasks().nth(form.selected()).map_or_else(
            || ("Task", "尚未配置 Task".to_owned()),
            |(name, task)| ("Task", format!("名称：{name}\n命令：{}", task.command)),
        ),
        FormPane::Dependencies => form
            .config()
            .dependencies()
            .nth(form.selected())
            .map_or_else(
                || ("管理依赖", "尚未配置管理依赖".to_owned()),
                |(name, dependency)| {
                    (
                        "管理依赖",
                        format!("名称：{name}\n来源：{}", dependency.source),
                    )
                },
            ),
    };
    let lines = vec![
        Line::styled(
            section,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(detail),
        Line::raw(""),
        Line::styled("按键", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("Tab / ← → 切换区域；↑ ↓ 选择条目"),
        Line::raw("Enter 编辑；n 新建；d 删除（需二次确认）"),
        Line::raw("Ctrl-S 校验并保存；F2 高级文本"),
        Line::raw("Esc 退出（未保存内容会请求确认）"),
        Line::raw(""),
        Line::styled("字段提示", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("参数和验证参数用空格分隔。"),
        Line::raw("环境变量用 KEY=VALUE,KEY2=VALUE2。"),
        Line::raw("依赖用 task:started,task2:healthy。"),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().title("详情与帮助").borders(Borders::ALL)),
        area,
    );
}

/// 绘制字段输入和选择器弹窗。
fn render_dialog(frame: &mut Frame<'_>, dialog: &Dialog) {
    let height = u16::try_from(dialog.fields().count().saturating_add(5)).unwrap_or(u16::MAX);
    let area = centered_rect(
        86,
        height.min(frame.area().height.saturating_sub(2)),
        frame.area(),
    );
    frame.render_widget(Clear, area);
    let lines = dialog
        .fields()
        .map(|(label, value, selected)| {
            let marker = if selected { "› " } else { "  " };
            let style = if selected {
                focus_style()
            } else {
                Style::default()
            };
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(format!("{label}："), Style::default().fg(Color::DarkGray)),
                Span::styled(value.to_owned(), style),
            ])
        })
        .collect::<Vec<_>>();
    let hint = if dialog.selected_is_choice() {
        "↑↓ 切换字段，←→ 选择选项，Enter 确认，Esc 取消"
    } else {
        "直接输入；↑↓ 切换字段，Enter 确认，Esc 取消"
    };
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(dialog.title())
                .borders(Borders::ALL)
                .title_bottom(hint),
        ),
        area,
    );
}

/// 绘制删除条目的二次确认弹窗。
fn render_delete_confirmation(frame: &mut Frame<'_>, name: &str) {
    let area = centered_rect(62, 5, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(format!("确定删除 `{name}`？再次按 d 确认，Esc 取消。"))
            .block(Block::default().title("确认删除").borders(Borders::ALL)),
        area,
    );
}

/// 绘制高级文本编辑模式。
fn render_text_mode(frame: &mut Frame<'_>, area: Rect, editor: &ConfigEditor) {
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
        Paragraph::new(lines).block(
            Block::default()
                .title("高级文本配置 · F1 表单")
                .borders(Borders::ALL),
        ),
        area,
    );
    let (row, column) = editor.cursor();
    if row >= scroll && row < scroll + inner_height {
        let x = area.x + 1 + 5 + u16::try_from(column).unwrap_or(u16::MAX);
        let y = area.y + 1 + u16::try_from(row - scroll).unwrap_or(u16::MAX);
        frame.set_cursor_position((x.min(area.right().saturating_sub(1)), y));
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
        Line::raw("dependencies.<id>.source / version"),
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

/// 返回当前焦点边框和选中行的样式。
fn focus_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// 将百分比宽度和固定高度居中为弹窗区域。
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
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

/// 根据反馈文本选择状态颜色。
fn message_style(message: &str) -> Style {
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
