use std::fmt::Write as _;

use crate::protocol::{SnapshotSourceDto, TaskView};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{List, ListItem, ListState, Paragraph, Tabs, Wrap},
};

use super::ui_support::{
    bordered, detail_label, detail_label_width, display_color, resource_labels, source_label,
    status_label, status_visual,
};
use super::{ActiveTab, App, text_view};

/// TUI 的强调色。
const ACCENT: Color = Color::Cyan;

/// 绘制完整的 TUI 页面。
pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 16 || area.height < 4 {
        render_too_small(frame, area, app);
        return;
    }
    if area.width < 30 || area.height < 10 {
        render_compact_summary(frame, area, app);
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(area);
    render_header(frame, sections[0], app);
    match app.active_tab() {
        ActiveTab::Tasks => render_tasks(frame, sections[1], app),
        ActiveTab::Dependencies => render_dependencies(frame, sections[1], app),
        ActiveTab::Logs => render_logs(frame, sections[1], app),
    }
    render_footer(frame, sections[2], app);
}

/// 绘制项目标题、连接来源和页签。
fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = bordered(app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);
    let (source, source_color) = source_label(app.snapshot().source, app.plain_mode());
    let separator = if app.plain_mode() { " | " } else { " · " };
    let title = Line::from(vec![
        Span::styled(
            text_view::clipped(
                &format!(" Procora{separator}{} ", app.snapshot().project),
                app.automatic_text_offset(),
                usize::from(area.width.saturating_sub(12)),
            ),
            Style::default()
                .fg(display_color(app, ACCENT))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(source, Style::default().fg(source_color)),
    ]);
    let tabs = Tabs::new(["1 任务", "2 依赖", "3 日志"])
        .select(app.active_tab().index())
        .highlight_style(
            Style::default()
                .fg(display_color(app, ACCENT))
                .add_modifier(Modifier::BOLD),
        )
        .divider(if app.plain_mode() { " | " } else { " │ " });
    frame.render_widget(Paragraph::new(title), rows[0]);
    frame.render_widget(tabs, rows[1]);
}

/// 绘制任务主从详情页面。
fn render_tasks(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let direction = if area.width >= 80 {
        Direction::Horizontal
    } else {
        Direction::Vertical
    };
    let panes = Layout::default()
        .direction(direction)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(area);
    render_task_list(frame, panes[0], app);
    render_task_details(frame, panes[1], app.selected_task(), app);
}

/// 绘制可快速扫描的任务列表。
fn render_task_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let items = app
        .snapshot()
        .tasks
        .iter()
        .enumerate()
        .map(|(index, task)| {
            let (symbol, color) = status_visual(task.status, app.plain_mode());
            let available = usize::from(area.width.saturating_sub(8));
            let offset = app.text_offset(index == app.selected_index());
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {symbol} "),
                    Style::default().fg(display_color(app, color)),
                ),
                Span::raw(text_view::clipped(task.task_id.as_str(), offset, available)),
            ]))
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(bordered(app).title(format!("任务 · {}", app.snapshot().tasks.len())))
        .highlight_symbol(if app.plain_mode() { "> " } else { "› " })
        .highlight_style(
            Style::default()
                .fg(display_color(app, ACCENT))
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default()
        .with_selected((!app.snapshot().tasks.is_empty()).then_some(app.selected_index()));
    frame.render_stateful_widget(list, area, &mut state);
}

/// 绘制当前任务的命令、依赖、资源与状态解释。
fn render_task_details(frame: &mut Frame<'_>, area: Rect, task: Option<&TaskView>, app: &App) {
    let content = task.map_or_else(
        || Text::from("配置中没有任务。\n添加任务后重新打开 TUI。"),
        |task| {
            let (_, status_color) = status_visual(task.status, app.plain_mode());
            let dependencies = if task.dependencies.is_empty() {
                "无".to_owned()
            } else {
                task.dependencies
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let (cpu, memory) = resource_labels(task, app.plain_mode());
            let mut lines = vec![
                detail_line("任务", task.task_id.as_str(), area.width, app),
                detail_line("命令", &task.command, area.width, app),
                Line::from(vec![
                    Span::styled(
                        detail_label("状态"),
                        Style::default().fg(display_color(app, Color::DarkGray)),
                    ),
                    Span::styled(
                        status_label(task.status),
                        Style::default().fg(display_color(app, status_color)),
                    ),
                ]),
                detail_line("依赖", &dependencies, area.width, app),
                detail_line("健康", health_label(task.health), area.width, app),
                detail_line("CPU", &cpu, area.width, app),
                detail_line("内存", &memory, area.width, app),
            ];
            if let Some(message) = &task.message {
                lines.push(Line::default());
                lines.push(detail_line("说明", message, area.width, app));
            }
            Text::from(lines)
        },
    );
    let details = Paragraph::new(content).block(bordered(app).title("详情"));
    frame.render_widget(details, area);
}

/// 返回 Task 健康状态的中文标签。
const fn health_label(health: crate::protocol::TaskHealthDto) -> &'static str {
    match health {
        crate::protocol::TaskHealthDto::Unknown => "未知",
        crate::protocol::TaskHealthDto::Starting => "检查中",
        crate::protocol::TaskHealthDto::Healthy => "健康",
        crate::protocol::TaskHealthDto::Unhealthy => "不健康",
        crate::protocol::TaskHealthDto::NotConfigured => "未配置",
    }
}

/// 绘制当前任务图的直接依赖边。
fn render_dependencies(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let selected = app.selected_task().map(|task| &task.task_id);
    let mut lines = Vec::new();
    for task in &app.snapshot().tasks {
        if task.dependencies.is_empty() {
            lines.push(graph_line(
                &format!(
                    "{} {}",
                    if app.plain_mode() { "*" } else { "●" },
                    task.task_id
                ),
                selected == Some(&task.task_id),
                area.width,
                app,
            ));
        } else {
            for dependency in &task.dependencies {
                lines.push(graph_line(
                    &format!(
                        "{} {} {}",
                        dependency,
                        if app.plain_mode() { "->" } else { "──▶" },
                        task.task_id
                    ),
                    selected == Some(&task.task_id),
                    area.width,
                    app,
                ));
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from("配置中没有可显示的任务依赖。"));
    }
    let graph = Paragraph::new(lines).block(bordered(app).title("直接依赖"));
    frame.render_widget(graph, area);
}

/// 绘制日志观察页及未连接状态。
fn render_logs(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let selected = app.selected_task();
    let task = selected.map_or_else(|| "未选择任务".to_owned(), |task| task.task_id.to_string());
    if let Some(selected) = selected
        && let Some(mut content) = app.styled_log_text(&selected.task_id)
    {
        if app.has_log_gap(&selected.task_id) {
            let warning = if app.plain_mode() {
                "! 日志游标曾过期，以下内容从当前可用位置恢复"
            } else {
                "⚠ 日志游标曾过期，以下内容从当前可用位置恢复"
            };
            content
                .lines
                .splice(0..0, [Line::from(warning), Line::default()]);
        }
        let viewport_lines = usize::from(area.height.saturating_sub(2));
        let scroll_top = app.log_scroll_top(&selected.task_id, viewport_lines);
        let distance = app.log_scroll_distance(&selected.task_id);
        let mut position = if distance == 0 {
            "跟随尾部".to_owned()
        } else {
            format!("已上翻 {distance} 行")
        };
        if !app.log_query().is_empty() {
            let filter = if app.log_filter_enabled() {
                "过滤"
            } else {
                "搜索"
            };
            let matches = app.log_match_position(&selected.task_id).map_or_else(
                || "0/0".to_owned(),
                |(current, total)| format!("{current}/{total}"),
            );
            let _ = write!(position, " · {filter} `{}` {matches}", app.log_query());
        }
        let logs = Paragraph::new(content)
            .block(bordered(app).title(log_title(area.width, &task, Some(&position), app)))
            .scroll((
                u16::try_from(scroll_top).unwrap_or(u16::MAX),
                u16::try_from(app.text_offset(true)).unwrap_or(u16::MAX),
            ));
        frame.render_widget(logs, area);
        return;
    }
    let message = match app.snapshot().source {
        SnapshotSourceDto::ConfigPreview => "预览模式不提供日志",
        SnapshotSourceDto::EmbeddedLive | SnapshotSourceDto::CenterLive => "暂无日志",
        SnapshotSourceDto::CenterStale => "连接已中断，日志可能不是最新状态",
    };
    let logs = Paragraph::new(message)
        .alignment(Alignment::Center)
        .block(bordered(app).title(log_title(area.width, &task, None, app)))
        .wrap(Wrap { trim: false });
    frame.render_widget(logs, area);
}

/// 绘制键盘操作提示。
fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let live = matches!(
        app.snapshot().source,
        SnapshotSourceDto::EmbeddedLive | SnapshotSourceDto::CenterLive
    );
    let controls = if app.active_tab() == ActiveTab::Logs {
        log_controls(app, area.width)
    } else if area.width < 64 && live && app.control_allowed() {
        "j/k 选择  Tab 切页  s/x/r 控制  q 退出"
    } else if area.width < 64 {
        "j/k 选择  Tab 切页  1/2/3 直达  q 退出"
    } else if live && app.control_allowed() {
        "↑↓/jk 选择  Tab 切页  ←→ 横移文本  s 启动  x 停止  r 重启  q/Esc 退出"
    } else {
        "↑↓/jk 选择任务  Tab 切页  ←→ 横移文本  1/2/3 直达  q/Esc 退出"
    };
    let controls = if app.back_navigation() {
        controls.replace("退出", "返回")
    } else {
        controls.to_owned()
    };
    let auto_scroll = if app.auto_scroll_enabled() && app.manual_scroll_frozen() {
        "开·高亮冻结"
    } else if app.auto_scroll_enabled() {
        "开"
    } else {
        "关"
    };
    let controls = if app.active_tab() == ActiveTab::Logs {
        format!("{controls}  / 搜索  n/N 匹配  f 过滤  C 清空")
    } else {
        format!("{controls}  F3 自动横移:{auto_scroll}")
    };
    let width = usize::from(area.width);
    let mut lines = vec![Line::from(text_view::clipped(&controls, 0, width))];
    if let Some(input) = app.log_search_input() {
        lines.push(Line::from(text_view::clipped(
            &format!("搜索日志：{input}_"),
            0,
            width,
        )));
    } else if let Some(feedback) = app.feedback() {
        lines.push(Line::from(text_view::clipped(
            feedback,
            app.automatic_text_offset(),
            width,
        )));
    }
    let footer =
        Paragraph::new(lines).style(Style::default().fg(display_color(app, Color::DarkGray)));
    frame.render_widget(footer, area);
}

/// 在小终端中优先保留项目、来源、Task 状态和退出入口。
fn render_compact_summary(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let (source, source_color) = source_label(app.snapshot().source, app.plain_mode());
    let mut lines = vec![
        Line::from(Span::styled(
            text_view::clipped(
                &format!("Procora · {}", app.snapshot().project),
                app.automatic_text_offset(),
                usize::from(area.width),
            ),
            Style::default()
                .fg(display_color(app, ACCENT))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(source, Style::default().fg(source_color))),
    ];
    if let Some(task) = app.selected_task() {
        lines.push(Line::from(format!(
            "{} · {}",
            task.task_id,
            status_label(task.status)
        )));
        if area.height >= 6
            && let Some(message) = app.feedback().or(task.message.as_deref())
        {
            lines.push(Line::from(text_view::clipped(
                message,
                app.text_offset(true),
                usize::from(area.width),
            )));
        }
    } else {
        lines.push(Line::from("无 Task"));
    }
    lines.push(Line::from(if app.back_navigation() {
        "q/Esc 返回 · 放大终端查看详情"
    } else {
        "q/Esc 退出 · 放大终端查看详情"
    }));
    frame.render_widget(Paragraph::new(lines), area);
}

/// 构造日志标题，并仅在宽度足够时附加 Task 切换提示。
fn log_title(area_width: u16, task: &str, position: Option<&str>, app: &App) -> String {
    let base = position.map_or_else(
        || format!("日志 · {task}"),
        |position| format!("日志 · {task} · {position}"),
    );
    let switch_hint = if app.plain_mode() {
        "j/k 切换任务日志"
    } else {
        "↑/↓ 切换任务日志"
    };
    let expanded = position.map_or_else(
        || format!("日志 · {task} · {switch_hint}"),
        |position| format!("日志 · {task} · {switch_hint} · {position}"),
    );
    if Line::from(expanded.as_str()).width().saturating_add(4) <= usize::from(area_width) {
        expanded
    } else {
        base
    }
}

/// 返回日志页的平台键位与鼠标操作提示。
fn log_controls(app: &App, width: u16) -> &'static str {
    match (app.mac_key_hints(), width < 76) {
        (true, true) => "Fn+↑/↓ 翻页  Fn+←/→ 首尾  滚轮滚动  q 退出",
        (true, false) => {
            "↑↓/jk 换任务  ←→ 横移  Fn+↑/↓ 翻页  Fn+←/→ 首尾  滚轮滚动  Tab 切页  q/Esc 退出"
        }
        (false, true) => "PgUp/PgDn 翻页  Home/End 首尾  滚轮滚动  q 退出",
        (false, false) => {
            "↑↓/jk 换任务  ←→ 横移  PgUp/PgDn 翻页  Home/End 首尾  滚轮滚动  Tab 切页  q/Esc 退出"
        }
    }
}

/// 在终端无法容纳稳定布局时显示恢复提示。
fn render_too_small(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let message = Paragraph::new("Procora\n终端过小")
        .alignment(Alignment::Center)
        .style(Style::default().fg(display_color(app, ACCENT)));
    frame.render_widget(message, area);
}

/// 创建统一的详情字段行。
fn detail_line(label: &str, value: impl Into<String>, area_width: u16, app: &App) -> Line<'static> {
    let value = value.into();
    let available = usize::from(area_width.saturating_sub(2)).saturating_sub(detail_label_width());
    Line::from(vec![
        Span::styled(
            detail_label(label),
            Style::default().fg(display_color(app, Color::DarkGray)),
        ),
        Span::raw(text_view::clipped(&value, app.text_offset(true), available)),
    ])
}

/// 创建依赖图中的一行并按选择状态着色。
fn graph_line(content: &str, selected: bool, area_width: u16, app: &App) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(display_color(app, ACCENT))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Line::styled(
        text_view::clipped(
            content,
            app.text_offset(selected),
            usize::from(area_width.saturating_sub(2)),
        ),
        style,
    )
}
