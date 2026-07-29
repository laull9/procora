//! Procora 包工作台的主从布局、空态和操作提示。

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{List, ListItem, ListState, Paragraph},
};

use super::{
    PackageWorkspaceApp, PackageWorkspaceTab, package_workspace_controls, text_view,
    ui_support::{bordered_for, detail_label, detail_label_width, display_color_for, format_bytes},
};

const ACCENT: Color = Color::Magenta;

/// 绘制完整包工作台和可选导出项弹层。
pub(super) fn render(frame: &mut Frame<'_>, app: &PackageWorkspaceApp) {
    let area = frame.area();
    if area.width < 48 || area.height < 16 {
        package_workspace_controls::render_compact(frame, area, app);
    } else {
        render_full(frame, area, app);
    }
    if app.help_visible() {
        package_workspace_controls::render_help(frame, area, app);
    } else if let Some((entries, selected)) = app.export_picker() {
        package_workspace_controls::render_export_picker(frame, area, entries, selected, app);
    }
}

/// 绘制页头、主从内容和双行反馈底栏。
fn render_full(frame: &mut Frame<'_>, area: Rect, app: &PackageWorkspaceApp) {
    let rows = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .split(area);
    render_header(frame, rows[0], app);
    let panes = Layout::default()
        .direction(if rows[1].width >= 82 {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(rows[1]);
    render_list(frame, panes[0], app);
    render_details(frame, panes[1], app);
    package_workspace_controls::render_footer(frame, rows[2], app);
}

/// 绘制工作台身份、上下文和两个稳定视图。
fn render_header(frame: &mut Frame<'_>, area: Rect, app: &PackageWorkspaceApp) {
    let block = bordered_for(app.plain_mode()).title("Procora · 包工作台");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner);
    let tabs = [
        PackageWorkspaceTab::Packages,
        PackageWorkspaceTab::Installed,
    ]
    .into_iter()
    .map(|tab| {
        let label = format!(
            " {} {} ",
            if app.tab() == tab { "●" } else { "○" },
            tab.label()
        );
        Span::styled(
            label,
            if app.tab() == tab {
                Style::default()
                    .fg(display_color_for(app.plain_mode(), ACCENT))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        )
    })
    .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(Line::from(tabs)), rows[0]);
    let context = app.context_source().map_or_else(
        || "未绑定 Service；按 b 选择来源构建".to_owned(),
        |path| format!("上下文 Service：{}", path.display()),
    );
    frame.render_widget(
        Paragraph::new(text_view::clipped(&context, 0, usize::from(rows[1].width))),
        rows[1],
    );
}

/// 绘制包文件或安装项列表。
fn render_list(frame: &mut Frame<'_>, area: Rect, app: &PackageWorkspaceApp) {
    match app.tab() {
        PackageWorkspaceTab::Packages => {
            let items = app
                .packages()
                .iter()
                .map(|entry| {
                    let label = entry.info.as_ref().map_or_else(
                        || {
                            format!(
                                "× {} · 清单损坏",
                                entry.path.file_name().unwrap_or_default().to_string_lossy()
                            )
                        },
                        |info| {
                            format!(
                                "◆ {} · {}",
                                info.manifest.project,
                                format_bytes(info.package_bytes)
                            )
                        },
                    );
                    ListItem::new(text_view::clipped(
                        &label,
                        0,
                        usize::from(area.width.saturating_sub(4)),
                    ))
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default().with_selected(Some(app.selected_package_index()));
            frame.render_stateful_widget(
                List::new(if items.is_empty() {
                    vec![ListItem::new("尚无包文件 · b 构建 / o 打开")]
                } else {
                    items
                })
                .highlight_symbol("› ")
                .highlight_style(highlight(app))
                .block(bordered_for(app.plain_mode()).title("包文件")),
                area,
                &mut state,
            );
        }
        PackageWorkspaceTab::Installed => {
            let items = app
                .installed()
                .iter()
                .map(|service| {
                    let state = if service.error.is_some() {
                        "损坏"
                    } else if service.pending_release.is_some() {
                        "待恢复"
                    } else if service.active_release.is_some() {
                        "活动"
                    } else {
                        "未激活"
                    };
                    ListItem::new(text_view::clipped(
                        &format!(
                            "▣ {} · {} · {} release",
                            service.project,
                            state,
                            service.releases.len()
                        ),
                        0,
                        usize::from(area.width.saturating_sub(4)),
                    ))
                })
                .collect::<Vec<_>>();
            let mut state =
                ListState::default().with_selected(Some(app.selected_installed_index()));
            frame.render_stateful_widget(
                List::new(if items.is_empty() {
                    vec![ListItem::new("尚未安装包 · 切到包文件按 i")]
                } else {
                    items
                })
                .highlight_symbol("› ")
                .highlight_style(highlight(app))
                .block(bordered_for(app.plain_mode()).title("已安装")),
                area,
                &mut state,
            );
        }
    }
}

/// 绘制当前包清单或安装状态详情。
fn render_details(frame: &mut Frame<'_>, area: Rect, app: &PackageWorkspaceApp) {
    let content = match app.tab() {
        PackageWorkspaceTab::Packages => package_details(app, area.width),
        PackageWorkspaceTab::Installed => installed_details(app, area.width),
    };
    frame.render_widget(
        Paragraph::new(content).block(bordered_for(app.plain_mode()).title("详情")),
        area,
    );
}

/// 生成当前包的可扫描摘要。
fn package_details(app: &PackageWorkspaceApp, width: u16) -> Text<'static> {
    let Some(entry) = app.selected_package() else {
        return Text::from(vec![
            Line::from("还没有可用包。"),
            Line::from(""),
            Line::from("按 b 从 Service 构建，或按 o 打开已有 .pcpkg。"),
        ]);
    };
    let Some(info) = &entry.info else {
        return Text::from(vec![
            Line::styled("包清单不可读", Style::default().fg(Color::Red)),
            Line::from(text_view::clipped(
                &entry.path.display().to_string(),
                0,
                usize::from(width.saturating_sub(2)),
            )),
            Line::from(""),
            Line::from(text_view::clipped(
                entry.error.as_deref().unwrap_or_default(),
                0,
                usize::from(width.saturating_sub(2)),
            )),
        ]);
    };
    let variants = info
        .manifest
        .binaries
        .values()
        .map(|binary| binary.variants.len())
        .sum::<usize>();
    let platforms = info
        .manifest
        .binaries
        .values()
        .flat_map(|binary| binary.variants.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    Text::from(vec![
        detail("Service", &info.manifest.project, width),
        detail("格式", &info.manifest.format, width),
        detail("大小", &format_bytes(info.package_bytes), width),
        detail("文件", &info.manifest.files.len().to_string(), width),
        detail(
            "二进制",
            &format!("{} / {variants} 变体", info.manifest.binaries.len()),
            width,
        ),
        detail(
            "平台",
            &if platforms.is_empty() {
                "无平台二进制".to_owned()
            } else {
                platforms.join("、")
            },
            width,
        ),
        detail(
            "导出",
            &if info.manifest.exports.is_empty() {
                "无".to_owned()
            } else {
                info.manifest
                    .exports
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("、")
            },
            width,
        ),
        Line::from(""),
        detail("Package", &short_digest(&info.package_digest), width),
        detail("路径", &entry.path.display().to_string(), width),
    ])
}

/// 生成当前安装项及最近 release 摘要。
fn installed_details(app: &PackageWorkspaceApp, width: u16) -> Text<'static> {
    let Some(service) = app.selected_installed() else {
        return Text::from(vec![
            Line::from("当前用户尚未安装 Procora 包。"),
            Line::from("切换到“包文件”，选择包后按 i 安装。"),
        ]);
    };
    let mut lines = vec![
        detail("Service", &service.project, width),
        detail(
            "Active",
            service.active_release.as_deref().unwrap_or("-"),
            width,
        ),
        detail(
            "Pending",
            service.pending_release.as_deref().unwrap_or("-"),
            width,
        ),
        detail("Releases", &service.releases.len().to_string(), width),
        detail("Packages", &service.packages.len().to_string(), width),
        detail("目录", &service.root.display().to_string(), width),
    ];
    if let Some(error) = &service.error {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            text_view::clipped(
                &format!("状态错误：{error}"),
                0,
                usize::from(width.saturating_sub(2)),
            ),
            Style::default().fg(Color::Red),
        ));
    } else if !service.releases.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "最近 release",
            Style::default().add_modifier(Modifier::BOLD),
        ));
        for release in service.releases.iter().take(5) {
            let marker = if release.active {
                "● active"
            } else if release.pending {
                "◐ pending"
            } else {
                "○ inactive"
            };
            let platform = release
                .target_platform
                .as_ref()
                .map_or_else(|| "-".to_owned(), crate::config::DeployPlatform::key);
            lines.push(Line::from(text_view::clipped(
                &format!("{marker:10} {} · {platform}", release.id),
                0,
                usize::from(width.saturating_sub(2)),
            )));
        }
    }
    Text::from(lines)
}

/// 生成统一详情标签行。
fn detail(label: &str, value: &str, width: u16) -> Line<'static> {
    let value_width = usize::from(width.saturating_sub(2)).saturating_sub(detail_label_width());
    Line::from(vec![
        Span::styled(detail_label(label), Style::default().fg(Color::DarkGray)),
        Span::raw(text_view::clipped(value, 0, value_width)),
    ])
}

/// 缩短长摘要但保留算法语义。
fn short_digest(digest: &str) -> String {
    digest.strip_prefix("sha256:").map_or_else(
        || digest.to_owned(),
        |value| format!("sha256:{}", value.get(..16).unwrap_or(value)),
    )
}

/// 返回当前列表高亮样式。
fn highlight(app: &PackageWorkspaceApp) -> Style {
    Style::default()
        .fg(display_color_for(app.plain_mode(), ACCENT))
        .add_modifier(Modifier::BOLD)
}
