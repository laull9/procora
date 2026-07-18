use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use super::{
    ConfigEditor, config_dialog_ui,
    config_form::FormPane,
    config_form_state::FormState,
    config_highlight,
    config_ui_support::{centered_rect, focus_style},
    text_view,
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
    let title_text = format!("Procora 配置编辑器 · {mode} · {}", editor.path().display());
    let title = Paragraph::new(text_view::clipped(
        &title_text,
        0,
        usize::from(outer[0].width.saturating_sub(2)),
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
    let footer = Paragraph::new(text_view::clipped(
        editor.message(),
        0,
        usize::from(outer[2].width.saturating_sub(2)),
    ))
    .block(Block::default().title("状态").borders(Borders::ALL))
    .style(message_style(editor.message()));
    frame.render_widget(footer, outer[2]);
}

/// 绘制以项目、profile、Task 和管理依赖为核心的结构化编辑页。
fn render_form(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(area);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(4),
            Constraint::Min(5),
            Constraint::Min(5),
        ])
        .split(columns[0]);
    render_project(frame, left[0], form);
    render_tasks(frame, left[1], form);
    render_dependencies(frame, left[2], form);
    render_profiles(frame, left[3], form);
    render_form_detail(frame, columns[1], form);
    if let Some(dialog) = form.dialog() {
        config_dialog_ui::render(frame, dialog);
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
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("名称：", Style::default().fg(Color::DarkGray)),
                Span::raw(text_view::clipped(
                    form.config().project(),
                    if focused { form.horizontal_offset() } else { 0 },
                    usize::from(area.width.saturating_sub(8)),
                )),
            ]),
            Line::from(vec![
                Span::styled("默认环境：", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{} 项", form.config().env.len())),
            ]),
            Line::from(vec![
                Span::styled("命名模板：", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{} 个", form.config().template_count())),
            ]),
        ])
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(style),
        ),
        area,
    );
}

/// 绘制可选择的命名 profile 列表。
fn render_profiles(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    let items = form
        .config()
        .profiles()
        .enumerate()
        .map(|(index, (name, profile))| {
            ListItem::new(text_view::clipped(
                &format!("{name}  ·  {}", profile.summary()),
                if index == form.selected() {
                    form.horizontal_offset()
                } else {
                    0
                },
                usize::from(area.width.saturating_sub(2)),
            ))
        })
        .collect();
    render_named_list(
        frame,
        area,
        form,
        FormPane::Profiles,
        "Profiles  ← Enter 编辑 · n 新建 · d 删除",
        "Profiles",
        "（暂无 profile，按 n 新建）",
        items,
    );
}

/// 绘制可选择的 Task 列表。
fn render_tasks(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    let items = form
        .config()
        .tasks()
        .enumerate()
        .map(|(index, (name, task))| {
            ListItem::new(text_view::clipped(
                &format!("{name}  ·  {}", task.command),
                if index == form.selected() {
                    form.horizontal_offset()
                } else {
                    0
                },
                usize::from(area.width.saturating_sub(2)),
            ))
        })
        .collect();
    render_named_list(
        frame,
        area,
        form,
        FormPane::Tasks,
        "Tasks  ← Enter 编辑 · h 健康检查 · n 新建 · d 删除",
        "Tasks",
        "（暂无 Task，按 n 新建）",
        items,
    );
}

/// 绘制可选择的管理依赖列表。
fn render_dependencies(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    let items = form
        .config()
        .dependencies()
        .enumerate()
        .map(|(index, (name, dependency))| {
            ListItem::new(text_view::clipped(
                &format!("{name}  ·  {}", dependency.source),
                if index == form.selected() {
                    form.horizontal_offset()
                } else {
                    0
                },
                usize::from(area.width.saturating_sub(2)),
            ))
        })
        .collect();
    render_named_list(
        frame,
        area,
        form,
        FormPane::Dependencies,
        "管理依赖  ← Enter 常用字段 · a 高级策略 · n 新建 · d 删除",
        "管理依赖",
        "（暂无依赖，按 n 新建）",
        items,
    );
}

/// 绘制带统一焦点和空状态的命名配置列表。
#[allow(clippy::too_many_arguments)]
fn render_named_list(
    frame: &mut Frame<'_>,
    area: Rect,
    form: &FormState,
    pane: FormPane,
    focused_title: &str,
    title: &str,
    empty: &str,
    items: Vec<ListItem<'_>>,
) {
    let focused = form.pane() == pane;
    let mut state = ListState::default();
    if focused && !items.is_empty() {
        state.select(Some(form.selected()));
    }
    let list = List::new(if items.is_empty() {
        vec![ListItem::new(empty)]
    } else {
        items
    })
    .block(
        Block::default()
            .title(if focused { focused_title } else { title })
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
        FormPane::Project => (
            "项目",
            format!(
                "项目名称：{}\n活动 profile：{}（共 {} 个）\n当前准入 Task：{} 个\n项目变量：{} 个（已解析 {} 个）\n默认环境变量：{} 项\nTask 默认：{}\n命名模板：{} 个（F2 可编辑定义）",
                form.config().project(),
                form.config().active_profile().unwrap_or("基础配置"),
                form.config().profile_count(),
                form.config().tasks().count(),
                form.config().vars.len(),
                form.config().resolved_vars.len(),
                form.config().env.len(),
                form.config().task_defaults.summary(),
                form.config().template_count()
            ),
        ),
        FormPane::Profiles => form.config().profiles().nth(form.selected()).map_or_else(
            || ("Profile", "尚未配置 profile".to_owned()),
            |(name, profile)| ("Profile", profile.detail(name)),
        ),
        FormPane::Tasks => form.config().tasks().nth(form.selected()).map_or_else(
            || ("Task", "尚未配置 Task".to_owned()),
            |(name, task)| {
                (
                    "Task",
                    format!(
                        "名称：{name}\n继承模板：{}\n命令：{}\n工作目录：{}（{}）\n环境文件：{}\n健康检查：{}\n成功退出码：{}（{}）\n重启策略：{}（{}）",
                        task.extends.as_deref().unwrap_or("未配置"),
                        task.command,
                        task.cwd.as_deref().unwrap_or("未配置"),
                        task.origin_label("cwd"),
                        task.env_file.as_deref().unwrap_or("未配置"),
                        task.health_label(),
                        task.success_exit_codes
                            .iter()
                            .map(i32::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        task.origin_label("success_exit_codes"),
                        task.restart,
                        task.origin_label("restart")
                    ),
                )
            },
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
                        format!(
                            "名称：{name}\n来源：{}\n版本：{}\n镜像：{} 个\n重试：{} 次 · 超时：{}\n大小上限：{} 字节",
                            dependency.source,
                            dependency.version,
                            dependency.mirrors.len(),
                            dependency.download.retries,
                            crate::config::format_duration(dependency.download.timeout_ms),
                            dependency.download.max_bytes
                        ),
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
        Line::raw("Tab / Shift-Tab 切换区域；↑ ↓ 在边界自动跨区"),
        Line::raw("← → 水平移动当前高亮文本"),
        Line::raw("Enter 编辑；Task 按 h 健康检查；依赖按 a 高级策略"),
        Line::raw("n 新建；d 删除（需二次确认）"),
        Line::raw("Ctrl-S 校验并保存；F2 高级文本"),
        Line::raw("Esc 退出（未保存内容会请求确认）"),
        Line::raw(""),
        Line::styled("字段提示", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("命令可直接带参数；精确参数仍优先使用 JSON 数组。"),
        Line::raw("环境变量/请求头字段按 F4 打开键值表。"),
        Line::raw("依赖用 task:started,task2:healthy。"),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().title("详情与帮助").borders(Borders::ALL)),
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
    let block = Block::default()
        .title("高级文本配置 · F1 表单")
        .borders(Borders::ALL);
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
