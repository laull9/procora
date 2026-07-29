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
    config_help_ui, config_text_ui,
    config_ui_support::{centered_rect, focus_style},
    text_view,
};

/// 绘制配置编辑器，并按当前模式选择结构化表单或高级文本界面。
pub(crate) fn render(frame: &mut Frame<'_>, editor: &ConfigEditor) {
    let area = frame.area();
    if area.width < 16 || area.height < 5 {
        render_too_small(frame, area, editor);
        render_editor_exit_prompt(frame, editor);
        return;
    }
    let compact = area.width < 72 || area.height < 18;
    let (header_height, footer_height) = if compact { (1, 2) } else { (3, 3) };
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(3),
            Constraint::Length(footer_height),
        ])
        .split(area);
    let mode = if editor.is_form_mode() {
        "结构化表单"
    } else {
        "高级文本"
    };
    let title_text = format!("Procora 配置编辑器 · {mode} · {}", editor.path().display());
    let title_width = if compact {
        outer[0].width
    } else {
        outer[0].width.saturating_sub(2)
    };
    let mut title = Paragraph::new(text_view::clipped(&title_text, 0, usize::from(title_width)))
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    if !compact {
        title = title.block(Block::default().borders(Borders::ALL));
    }
    frame.render_widget(title, outer[0]);

    if let Some(form) = editor.form().filter(|_| editor.is_form_mode()) {
        render_form(frame, outer[1], form);
    } else {
        config_text_ui::render(frame, outer[1], editor);
    }
    let footer_width = usize::from(if compact {
        outer[2].width
    } else {
        outer[2].width.saturating_sub(2)
    });
    let footer_text = if compact {
        vec![
            Line::from(text_view::clipped(editor.message(), 0, footer_width)),
            Line::from(text_view::clipped(
                "Ctrl-S保存 · Esc退出 · F1/F2模式",
                0,
                footer_width,
            )),
        ]
    } else {
        vec![Line::from(text_view::clipped(
            editor.message(),
            0,
            footer_width,
        ))]
    };
    let mut footer =
        Paragraph::new(footer_text).style(config_text_ui::message_style(editor.message()));
    if !compact {
        footer = footer.block(Block::default().title("状态").borders(Borders::ALL));
    }
    frame.render_widget(footer, outer[2]);
    render_editor_exit_prompt(frame, editor);
}

/// 绘制全局未保存退出选择，终端缩小时仍保留恢复路径。
fn render_editor_exit_prompt(frame: &mut Frame<'_>, editor: &ConfigEditor) {
    let Some(prompt) = editor.exit_prompt() else {
        return;
    };
    let area = centered_rect(72, 9, frame.area());
    frame.render_widget(Clear, area);
    prompt.render(frame, area, "退出配置编辑器", "检测到尚未保存的配置修改。");
}

/// 在极小终端中保留保存和退出恢复路径。
fn render_too_small(frame: &mut Frame<'_>, area: Rect, editor: &ConfigEditor) {
    let mode = if editor.is_form_mode() {
        "表单"
    } else {
        "文本"
    };
    frame.render_widget(
        Paragraph::new(format!("Procora配置·{mode}\n终端过小\nCtrl-S保存·Esc退出")),
        area,
    );
}

/// 绘制以项目、profile、Task 和管理依赖为核心的结构化编辑页。
fn render_form(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    if area.width < 72 || area.height < 18 {
        render_compact_form(frame, area, form);
        render_form_overlays(frame, form);
        return;
    }
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
    render_form_overlays(frame, form);
}

/// 窄屏只显示当前区域及其详情，避免四个列表被压成空框。
fn render_compact_form(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    if area.height < 8 {
        render_active_pane(frame, area, form);
        return;
    }
    let rows =
        Layout::vertical([Constraint::Percentage(46), Constraint::Percentage(54)]).split(area);
    render_active_pane(frame, rows[0], form);
    render_compact_form_detail(frame, rows[1], form);
}

/// 绘制当前获得焦点的结构化表单区域。
fn render_active_pane(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    match form.pane() {
        FormPane::Project => render_project(frame, area, form),
        FormPane::Tasks => render_tasks(frame, area, form),
        FormPane::Dependencies => render_dependencies(frame, area, form),
        FormPane::Profiles => render_profiles(frame, area, form),
    }
}

/// 绘制表单弹层与删除确认，供宽窄布局复用。
fn render_form_overlays(frame: &mut Frame<'_>, form: &FormState) {
    if let Some(dialog) = form.dialog() {
        config_dialog_ui::render(frame, dialog);
        if let Some(prompt) = form.dialog_exit_prompt() {
            let area = centered_rect(72, 9, frame.area());
            frame.render_widget(Clear, area);
            prompt.render(
                frame,
                area,
                "退出本轮编辑",
                "检测到本轮字段内容发生了变化。",
            );
        }
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
                    form.text_offset(focused),
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
    let focused = form.pane() == FormPane::Profiles;
    let items = form
        .config()
        .profiles()
        .enumerate()
        .map(|(index, (name, profile))| {
            ListItem::new(text_view::clipped(
                &format!("{name}  ·  {}", profile.summary()),
                form.text_offset(focused && index == form.selected()),
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
    let focused = form.pane() == FormPane::Tasks;
    let items = form
        .config()
        .tasks()
        .enumerate()
        .map(|(index, (name, task))| {
            ListItem::new(text_view::clipped(
                &format!("{name}  ·  {}", task.command),
                form.text_offset(focused && index == form.selected()),
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
    let focused = form.pane() == FormPane::Dependencies;
    let items = form
        .config()
        .dependencies()
        .enumerate()
        .map(|(index, (name, dependency))| {
            ListItem::new(text_view::clipped(
                &format!("{name}  ·  {}", dependency.source),
                form.text_offset(focused && index == form.selected()),
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
    let (section, detail) = form_detail(form);
    let mut lines = vec![
        Line::styled(
            section,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(detail),
        Line::raw(""),
    ];
    lines.extend(config_help_ui::form_key_hints(form));
    lines.extend([
        Line::raw(""),
        Line::styled("字段提示", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw("命令可直接带参数；精确参数仍优先使用 JSON 数组。"),
        Line::raw("环境变量/请求头字段按 F4 打开键值表。"),
        Line::raw("依赖用 task:started,task2:healthy。"),
    ]);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().title("详情与帮助").borders(Borders::ALL)),
        area,
    );
}

/// 窄屏把常用操作放在详情之前，避免说明因纵向裁剪而消失。
fn render_compact_form_detail(frame: &mut Frame<'_>, area: Rect, form: &FormState) {
    let (section, detail) = form_detail(form);
    let width = usize::from(area.width.saturating_sub(2));
    let lines = vec![
        Line::from(text_view::clipped("Tab换区 · ↑↓选择 · Enter编辑", 0, width)),
        Line::from(text_view::clipped(
            "n新建 · d删除 · Ctrl-S保存 · F2文本",
            0,
            width,
        )),
        Line::styled(
            section,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(detail),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().title("当前区域详情").borders(Borders::ALL)),
        area,
    );
}

/// 返回当前表单区域的可读详情。
fn form_detail(form: &FormState) -> (&'static str, String) {
    match form.pane() {
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
    }
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
