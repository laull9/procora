use crate::protocol::{ServiceStatusDto, ServiceViewDto};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{List, ListItem, ListState, Paragraph},
};

use super::{
    OverviewApp, text_view,
    ui_support::{bordered_for, display_color_for},
};

/// 总览页面的强调色。
const ACCENT: Color = Color::Cyan;

/// 绘制完整服务总览页面。
pub(super) fn render(frame: &mut Frame<'_>, app: &OverviewApp) {
    let area = frame.area();
    if area.width < 16 || area.height < 4 {
        render_too_small(frame, area, app);
        return;
    }
    if area.width < 30 || area.height < 10 {
        render_compact(frame, area, app);
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
    render_services(frame, sections[1], app);
    render_footer(frame, sections[2], app);
}

/// 绘制中心标题和服务状态统计。
fn render_header(frame: &mut Frame<'_>, area: Rect, app: &OverviewApp) {
    let block = bordered_for(app.plain_mode());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let running = app
        .services()
        .iter()
        .filter(|service| service.status == ServiceStatusDto::Running)
        .count();
    let failed = app
        .services()
        .iter()
        .filter(|service| service.status == ServiceStatusDto::Failed)
        .count();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Procora · 服务总览 ",
            Style::default()
                .fg(display_color_for(app.plain_mode(), ACCENT))
                .add_modifier(Modifier::BOLD),
        ))),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(format!(
            "全局服务 · 共 {} 个 · 运行 {running} · 失败 {failed}",
            app.services().len()
        )),
        rows[1],
    );
}

/// 绘制服务列表与选中服务详情。
fn render_services(frame: &mut Frame<'_>, area: Rect, app: &OverviewApp) {
    let direction = if area.width >= 80 {
        Direction::Horizontal
    } else {
        Direction::Vertical
    };
    let panes = Layout::default()
        .direction(direction)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(area);
    render_service_list(frame, panes[0], app);
    render_service_details(frame, panes[1], app.selected_service(), app);
}

/// 绘制可循环选择的服务列表。
fn render_service_list(frame: &mut Frame<'_>, area: Rect, app: &OverviewApp) {
    let items = app
        .services()
        .iter()
        .enumerate()
        .map(|(index, service)| {
            let (symbol, color) = service_status_visual(service.status, app.plain_mode());
            let available = usize::from(area.width.saturating_sub(8));
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {symbol} "),
                    Style::default().fg(display_color_for(app.plain_mode(), color)),
                ),
                Span::raw(text_view::clipped(
                    &service.name,
                    app.text_offset(index == app.selected_index()),
                    available,
                )),
            ]))
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(bordered_for(app.plain_mode()).title(format!("服务 · {}", app.services().len())))
        .highlight_symbol(if app.plain_mode() { "> " } else { "› " })
        .highlight_style(
            Style::default()
                .fg(display_color_for(app.plain_mode(), ACCENT))
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default()
        .with_selected((!app.services().is_empty()).then_some(app.selected_index()));
    frame.render_stateful_widget(list, area, &mut state);
}

/// 绘制当前服务的状态、Task 数量与路径。
fn render_service_details(
    frame: &mut Frame<'_>,
    area: Rect,
    service: Option<&ServiceViewDto>,
    app: &OverviewApp,
) {
    let content = service.map_or_else(
        || Text::from("尚未注册服务。\n使用 `procora add <path>` 添加服务。"),
        |service| {
            let (_, color) = service_status_visual(service.status, app.plain_mode());
            let mut lines = vec![
                detail_line("服务", &service.name, area.width, app),
                Line::from(vec![
                    Span::styled(
                        "状态  ",
                        Style::default().fg(display_color_for(app.plain_mode(), Color::DarkGray)),
                    ),
                    Span::styled(
                        service_status_label(service.status),
                        Style::default().fg(display_color_for(app.plain_mode(), color)),
                    ),
                ]),
                detail_line("Task", &service.task_count.to_string(), area.width, app),
                detail_line("目录", &service.root.to_string_lossy(), area.width, app),
                detail_line(
                    "配置",
                    &service.config_path.to_string_lossy(),
                    area.width,
                    app,
                ),
            ];
            if let Some(message) = &service.message {
                lines.push(Line::default());
                lines.push(detail_line("说明", message, area.width, app));
            }
            Text::from(lines)
        },
    );
    frame.render_widget(
        Paragraph::new(content).block(bordered_for(app.plain_mode()).title("详情")),
        area,
    );
}

/// 绘制总览操作提示和反馈。
fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &OverviewApp) {
    let controls = if area.width < 72 && app.control_allowed() {
        "j/k 选择  Enter 详情  s/x/r 控制  d 移除  q 退出"
    } else if area.width < 72 {
        "j/k 选择  Enter 详情  q 退出"
    } else if app.control_allowed() {
        "↑↓/jk 选择服务  Enter 打开详情  ←→ 横移文本  s 启动  x 停止  r 重启  d 移除  q/Esc 退出"
    } else {
        "↑↓/jk 选择服务  Enter 打开详情  ←→ 横移文本  q/Esc 退出"
    };
    let auto_scroll = if app.auto_scroll_enabled() && app.manual_scroll_frozen() {
        "开·高亮冻结"
    } else if app.auto_scroll_enabled() {
        "开"
    } else {
        "关"
    };
    let controls = format!("{controls}  F3 自动横移:{auto_scroll}");
    let width = usize::from(area.width);
    let mut lines = vec![Line::from(text_view::clipped(&controls, 0, width))];
    if let Some(feedback) = app.feedback() {
        lines.push(Line::from(text_view::clipped(
            feedback,
            app.automatic_text_offset(),
            width,
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(display_color_for(app.plain_mode(), Color::DarkGray))),
        area,
    );
}

/// 绘制受限终端中的服务摘要。
fn render_compact(frame: &mut Frame<'_>, area: Rect, app: &OverviewApp) {
    let mut lines = vec![Line::from(Span::styled(
        "Procora · 服务总览",
        Style::default()
            .fg(display_color_for(app.plain_mode(), ACCENT))
            .add_modifier(Modifier::BOLD),
    ))];
    if let Some(service) = app.selected_service() {
        lines.push(Line::from(format!(
            "{} · {} · {} Task",
            service.name,
            service_status_label(service.status),
            service.task_count
        )));
    } else {
        lines.push(Line::from("尚未注册服务"));
    }
    lines.push(Line::from("Enter 详情 · q/Esc 退出"));
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(bordered_for(app.plain_mode())),
        area,
    );
}

/// 绘制无法容纳内容时的恢复说明。
fn render_too_small(frame: &mut Frame<'_>, area: Rect, app: &OverviewApp) {
    frame.render_widget(
        Paragraph::new("Procora\n终端过小，请放大窗口\nq 退出")
            .alignment(Alignment::Center)
            .block(bordered_for(app.plain_mode())),
        area,
    );
}

/// 创建带统一标签宽度和水平折叠的详情行。
fn detail_line<'a>(label: &str, value: &str, width: u16, app: &OverviewApp) -> Line<'a> {
    let available = usize::from(width.saturating_sub(8));
    Line::from(vec![
        Span::styled(
            format!("{label:<4}"),
            Style::default().fg(display_color_for(app.plain_mode(), Color::DarkGray)),
        ),
        Span::raw(text_view::clipped(value, app.text_offset(true), available)),
    ])
}

/// 返回服务状态符号和颜色。
const fn service_status_visual(status: ServiceStatusDto, plain: bool) -> (&'static str, Color) {
    if plain {
        return match status {
            ServiceStatusDto::Running => ("*", Color::Reset),
            ServiceStatusDto::Stopped => ("-", Color::Reset),
            ServiceStatusDto::Failed => ("x", Color::Reset),
        };
    }
    match status {
        ServiceStatusDto::Running => ("●", Color::Green),
        ServiceStatusDto::Stopped => ("■", Color::DarkGray),
        ServiceStatusDto::Failed => ("×", Color::Red),
    }
}

/// 返回服务状态中文标签。
const fn service_status_label(status: ServiceStatusDto) -> &'static str {
    match status {
        ServiceStatusDto::Running => "运行中",
        ServiceStatusDto::Stopped => "已停止",
        ServiceStatusDto::Failed => "失败",
    }
}
