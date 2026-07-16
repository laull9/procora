use crate::protocol::{SnapshotSourceDto, TaskStatusDto, TaskView};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
};

use super::{ActiveTab, App};

/// TUI 的强调色。
const ACCENT: Color = Color::Cyan;

/// 低能力终端使用的 ASCII 边框。
const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// 绘制完整的 TUI 页面。
pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 30 || area.height < 8 {
        render_too_small(frame, area, app);
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
            format!(" Procora{separator}{} ", app.snapshot().project),
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
        .map(|task| {
            let (symbol, color) = status_visual(task.status, app.plain_mode());
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {symbol} "),
                    Style::default().fg(display_color(app, color)),
                ),
                Span::raw(task.task_id.to_string()),
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
                detail_line("任务", task.task_id.as_str(), app),
                detail_line("命令", &task.command, app),
                Line::from(vec![
                    Span::styled(
                        "状态  ",
                        Style::default().fg(display_color(app, Color::DarkGray)),
                    ),
                    Span::styled(
                        status_label(task.status),
                        Style::default().fg(display_color(app, status_color)),
                    ),
                ]),
                detail_line("依赖", &dependencies, app),
                detail_line("健康", health_label(task.health), app),
                detail_line("CPU", &cpu, app),
                detail_line("内存", &memory, app),
            ];
            if let Some(message) = &task.message {
                lines.push(Line::default());
                lines.push(detail_line("说明", message, app));
            }
            Text::from(lines)
        },
    );
    let details = Paragraph::new(content)
        .block(bordered(app).title("详情"))
        .wrap(Wrap { trim: false });
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
                format!(
                    "{} {}",
                    if app.plain_mode() { "*" } else { "●" },
                    task.task_id
                ),
                selected == Some(&task.task_id),
                app,
            ));
        } else {
            for dependency in &task.dependencies {
                lines.push(graph_line(
                    format!(
                        "{} {} {}",
                        dependency,
                        if app.plain_mode() { "->" } else { "──▶" },
                        task.task_id
                    ),
                    selected == Some(&task.task_id),
                    app,
                ));
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from("配置中没有可显示的任务依赖。"));
    }
    let graph = Paragraph::new(lines)
        .block(bordered(app).title("直接依赖"))
        .wrap(Wrap { trim: false });
    frame.render_widget(graph, area);
}

/// 绘制日志观察页及未连接状态。
fn render_logs(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let selected = app.selected_task();
    let task = selected.map_or_else(|| "未选择任务".to_owned(), |task| task.task_id.to_string());
    if let Some(selected) = selected
        && let Some(content) = app.log_text(&selected.task_id)
    {
        let prefix = if app.has_log_gap(&selected.task_id) {
            if app.plain_mode() {
                "! 日志游标曾过期，以下内容从当前可用位置恢复\n\n"
            } else {
                "⚠ 日志游标曾过期，以下内容从当前可用位置恢复\n\n"
            }
        } else {
            ""
        };
        let logs = Paragraph::new(format!("{prefix}{content}"))
            .block(bordered(app).title(format!("日志 · {task}")))
            .wrap(Wrap { trim: false });
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
        .block(bordered(app).title(format!("日志 · {task}")))
        .wrap(Wrap { trim: false });
    frame.render_widget(logs, area);
}

/// 绘制键盘操作提示。
fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let live = matches!(
        app.snapshot().source,
        SnapshotSourceDto::EmbeddedLive | SnapshotSourceDto::CenterLive
    );
    let controls = if area.width < 64 && live && app.control_allowed() {
        "j/k 选择  Tab 切页  s/x/r 控制  q 退出"
    } else if area.width < 64 {
        "j/k 选择  Tab 切页  1/2/3 直达  q 退出"
    } else if live && app.control_allowed() {
        "↑↓/jk 选择  Tab/←→ 切页  s 启动  x 停止  r 重启  q/Esc 退出"
    } else {
        "↑↓/jk 选择任务  Tab/←→ 切换页面  1/2/3 直达  q/Esc 退出"
    };
    let mut lines = vec![Line::from(controls)];
    if let Some(feedback) = app.feedback() {
        lines.push(Line::from(feedback));
    }
    let footer =
        Paragraph::new(lines).style(Style::default().fg(display_color(app, Color::DarkGray)));
    frame.render_widget(footer, area);
}

/// 在终端无法容纳稳定布局时显示恢复提示。
fn render_too_small(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let size = if app.plain_mode() { "30x8" } else { "30×8" };
    let message = Paragraph::new(format!("终端尺寸过小\n请扩大到至少 {size}"))
        .alignment(Alignment::Center)
        .block(bordered(app).title("Procora"));
    frame.render_widget(message, area);
}

/// 创建统一的详情字段行。
fn detail_line(label: &str, value: impl Into<String>, app: &App) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label}  "),
            Style::default().fg(display_color(app, Color::DarkGray)),
        ),
        Span::raw(value.into()),
    ])
}

/// 创建依赖图中的一行并按选择状态着色。
fn graph_line(content: String, selected: bool, app: &App) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(display_color(app, ACCENT))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Line::styled(content, style)
}

/// 返回快照来源标签及颜色。
const fn source_label(source: SnapshotSourceDto, plain: bool) -> (&'static str, Color) {
    let (label, color) = match source {
        SnapshotSourceDto::ConfigPreview => ("预览", Color::Yellow),
        SnapshotSourceDto::EmbeddedLive => ("临时服务", Color::Green),
        SnapshotSourceDto::CenterLive => ("全局服务", Color::Green),
        SnapshotSourceDto::CenterStale => ("连接中断", Color::Red),
    };
    (label, if plain { Color::Reset } else { color })
}

/// 返回任务状态的符号与颜色。
const fn status_visual(status: TaskStatusDto, plain: bool) -> (&'static str, Color) {
    if plain {
        return match status {
            TaskStatusDto::Pending => ("o", Color::Reset),
            TaskStatusDto::Blocked => ("?", Color::Reset),
            TaskStatusDto::Running => ("*", Color::Reset),
            TaskStatusDto::Stopped => ("-", Color::Reset),
            TaskStatusDto::Failed => ("x", Color::Reset),
        };
    }
    match status {
        TaskStatusDto::Pending => ("○", Color::Yellow),
        TaskStatusDto::Blocked => ("◆", Color::Magenta),
        TaskStatusDto::Running => ("●", Color::Green),
        TaskStatusDto::Stopped => ("■", Color::DarkGray),
        TaskStatusDto::Failed => ("×", Color::Red),
    }
}

/// 返回任务状态的中文标签。
const fn status_label(status: TaskStatusDto) -> &'static str {
    match status {
        TaskStatusDto::Pending => "等待调度",
        TaskStatusDto::Blocked => "依赖阻断",
        TaskStatusDto::Running => "运行中",
        TaskStatusDto::Stopped => "已停止",
        TaskStatusDto::Failed => "失败",
    }
}

/// 返回任务资源的可读标签。
fn resource_labels(task: &TaskView, plain: bool) -> (String, String) {
    let unavailable = if plain { "-" } else { "—" };
    task.resources.map_or_else(
        || (unavailable.to_owned(), unavailable.to_owned()),
        |resources| {
            let cpu = resources.cpu_tenths_percent.map_or_else(
                || unavailable.to_owned(),
                |value| format!("{}.{:01}%", value / 10, value % 10),
            );
            let memory = resources
                .memory_bytes
                .map_or_else(|| unavailable.to_owned(), format_bytes);
            (cpu, memory)
        },
    )
}

/// 创建适配当前终端能力的边框块。
fn bordered<'a>(app: &App) -> Block<'a> {
    let block = Block::default().borders(Borders::ALL);
    if app.plain_mode() {
        block.border_set(ASCII_BORDER)
    } else {
        block
    }
}

/// 在纯文本模式下关闭显式颜色。
const fn display_color(app: &App, color: Color) -> Color {
    if app.plain_mode() {
        Color::Reset
    } else {
        color
    }
}

/// 将字节数格式化为适合终端详情面板的短文本。
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if bytes >= GIB {
        format_unit(bytes, GIB, "GiB")
    } else if bytes >= MIB {
        format_unit(bytes, MIB, "MiB")
    } else if bytes >= KIB {
        format_unit(bytes, KIB, "KiB")
    } else {
        format!("{bytes} B")
    }
}

/// 使用整数运算生成保留一位小数的容量文本。
fn format_unit(bytes: u64, unit: u64, suffix: &str) -> String {
    let whole = bytes / unit;
    let decimal = (bytes % unit) * 10 / unit;
    format!("{whole}.{decimal} {suffix}")
}
