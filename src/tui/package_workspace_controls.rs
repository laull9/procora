//! 包工作台的自适应底栏、紧凑视图、帮助和选择弹层。

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use super::{
    PackageWorkspaceApp, PackageWorkspaceTab, help_ui, key_hints, text_view,
    ui_support::{bordered_for, display_color_for},
};

const ACCENT: Color = Color::Magenta;

/// 绘制稳定快捷键与最近反馈。
pub(super) fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &PackageWorkspaceApp) {
    let auto_scroll = auto_scroll_label(app);
    let controls = if app.control_allowed() {
        key_hints::adaptive(
            &[
                key_hints::join(&[
                    "Tab 视图",
                    "j/k 选择",
                    "b 构建",
                    "o 打开",
                    "v 验证",
                    "i 安装",
                    "t 运行",
                    "x 解包",
                    "d 部署",
                    "u 导出",
                    "Del Del 删包",
                    "R 回滚",
                    "c 恢复",
                    "U U 解除",
                    "D D 永久删除",
                    "←→ 横移",
                    &format!("F3 自动:{auto_scroll}"),
                    "r 刷新",
                    "? 帮助",
                    "Esc 返回",
                ]),
                key_hints::join(&[
                    "Tab",
                    "j/k",
                    "b构建",
                    "o打开",
                    "v验证",
                    "i安装",
                    "d部署",
                    "u导出",
                    "Del删包",
                    "R回滚",
                    "c恢复",
                    "U解除",
                    "D删除",
                    "←→横移",
                    "?帮助",
                    "Esc返回",
                ]),
                key_hints::join(&[
                    "j/k 选",
                    "b 构建",
                    "o 打开",
                    "Del 删包",
                    "? 帮助",
                    "Esc 返回",
                ]),
                key_hints::join(&["? 帮助", "Esc 返回"]),
            ],
            area.width,
        )
    } else {
        key_hints::adaptive(
            &[
                key_hints::join(&[
                    "Tab 视图",
                    "j/k 选择",
                    "o 打开",
                    "v 验证",
                    "x 解包",
                    "←→ 横移",
                    &format!("F3 自动:{auto_scroll}"),
                    "r 刷新",
                    "? 帮助",
                    "Esc 返回",
                ]),
                key_hints::join(&[
                    "j/k 选",
                    "o 打开",
                    "v 验证",
                    "←→ 横移",
                    "? 帮助",
                    "Esc 返回",
                ]),
                key_hints::join(&["? 帮助", "Esc 返回"]),
            ],
            area.width,
        )
    };
    let width = usize::from(area.width);
    let mut lines = vec![Line::from(text_view::clipped(&controls, 0, width))];
    if let Some(feedback) = app.feedback() {
        lines.push(Line::from(text_view::clipped(
            feedback,
            app.text_offset(true),
            width,
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(display_color_for(app.plain_mode(), Color::DarkGray))),
        area,
    );
}

/// 绘制窄终端中的当前上下文、主操作和恢复入口。
pub(super) fn render_compact(frame: &mut Frame<'_>, area: Rect, app: &PackageWorkspaceApp) {
    let width = usize::from(area.width.saturating_sub(2));
    if area.height < 6 {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(text_view::clipped(
                    "Procora · 包工作台",
                    0,
                    usize::from(area.width),
                )),
                Line::from(text_view::clipped(
                    &format!("{} · ?帮助 · Esc返回", app.tab().label()),
                    0,
                    usize::from(area.width),
                )),
            ]),
            area,
        );
        return;
    }
    let selected = match app.tab() {
        PackageWorkspaceTab::Packages => app.selected_package().map_or_else(
            || "尚无包 · b 构建 / o 打开".to_owned(),
            |entry| {
                entry.info.as_ref().map_or_else(
                    || "当前包清单损坏 · Del 删除".to_owned(),
                    |info| format!("当前包 · {}", info.manifest.project),
                )
            },
        ),
        PackageWorkspaceTab::Installed => app.selected_installed().map_or_else(
            || "尚未安装 · 包文件页按 i".to_owned(),
            |service| {
                let state = if service.error.is_some() {
                    "损坏"
                } else if service.pending_release.is_some() {
                    "待恢复"
                } else if service.active_release.is_some() {
                    "活动"
                } else {
                    "未激活"
                };
                format!("当前安装 · {} · {state}", service.project)
            },
        ),
    };
    let primary = match app.tab() {
        PackageWorkspaceTab::Packages if app.control_allowed() => "b构建 o打开 i安装 Del删包",
        PackageWorkspaceTab::Packages => "o打开 v验证 x解包",
        PackageWorkspaceTab::Installed
            if app.control_allowed() && app.selected_installed().is_some() =>
        {
            "R回滚 c恢复 U解除 D删除"
        }
        PackageWorkspaceTab::Installed if app.selected_installed().is_none() => {
            "Tab到包文件 · ?帮助"
        }
        PackageWorkspaceTab::Installed => "r刷新 ?帮助 Esc返回",
    };
    let mut lines = vec![
        Line::from("Procora · 包工作台"),
        Line::from(format!(
            "{} · 包{} / 安装{}",
            app.tab().label(),
            app.packages().len(),
            app.installed().len()
        )),
        Line::from(text_view::clipped(&selected, app.text_offset(true), width)),
    ];
    if area.height >= 7 {
        lines.push(Line::from(text_view::clipped(primary, 0, width)));
    }
    lines.push(Line::from(text_view::clipped(
        "Tab切换 · ?帮助 · Esc返回",
        0,
        width,
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(bordered_for(app.plain_mode())),
        area,
    );
}

/// 绘制页面级快捷键帮助。
pub(super) fn render_help(frame: &mut Frame<'_>, area: Rect, app: &PackageWorkspaceApp) {
    let mut lines = vec![
        help_ui::key_line("Tab", "切换包文件与已安装状态", app.plain_mode()),
        help_ui::key_line("↑↓ / j k", "选择当前列表项", app.plain_mode()),
        help_ui::key_line("← →", "移动被折叠的当前文本", app.plain_mode()),
        help_ui::key_line("F3", "切换全部折叠文本自动横移", app.plain_mode()),
        help_ui::key_line(
            "o / v / x",
            "打开、完整验证、按当前平台解包",
            app.plain_mode(),
        ),
        help_ui::key_line("r", "重新扫描包与 release 状态", app.plain_mode()),
    ];
    if app.control_allowed() {
        lines.extend([
            help_ui::key_line("b", "从上下文 Service 构建确定性包", app.plain_mode()),
            help_ui::key_line("i / t", "安装为不可变 release / 临时运行", app.plain_mode()),
            help_ui::key_line("d / u", "裸机部署 / 推送命名导出项", app.plain_mode()),
            help_ui::key_line(
                "Delete Delete / X X",
                "二次确认永久删除当前包文件",
                app.plain_mode(),
            ),
            help_ui::key_line(
                "R / c",
                "回滚历史 release / 恢复 pending 安装",
                app.plain_mode(),
            ),
            help_ui::key_line(
                "U U",
                "二次确认解除包托管并保留数据；同名普通 Service 保留",
                app.plain_mode(),
            ),
            help_ui::key_line(
                "D D",
                "二次确认永久删除安装数据；同名普通 Service 保留",
                app.plain_mode(),
            ),
        ]);
    }
    help_ui::render(
        frame,
        area,
        "快捷键帮助 · 包工作台",
        lines,
        app.plain_mode(),
    );
}

/// 返回与主 TUI 一致的自动横移状态标签。
fn auto_scroll_label(app: &PackageWorkspaceApp) -> &'static str {
    if app.auto_scroll_enabled() && app.manual_scroll_frozen() {
        "开·冻结"
    } else if app.auto_scroll_enabled() {
        "开"
    } else {
        "关"
    }
}

/// 绘制多个命名导出项的局部选择。
pub(super) fn render_export_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    entries: &[String],
    selected: usize,
    app: &PackageWorkspaceApp,
) {
    let width = area.width.saturating_sub(4).clamp(20, 64);
    let height = u16::try_from(entries.len().saturating_add(4))
        .unwrap_or(u16::MAX)
        .min(area.height.saturating_sub(2))
        .max(5)
        .min(area.height);
    let popup = centered(width, height, area);
    frame.render_widget(Clear, popup);
    let item_width = usize::from(popup.width.saturating_sub(4));
    let items = entries
        .iter()
        .map(|entry| ListItem::new(text_view::clipped(entry, 0, item_width)))
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(selected));
    let hint = if popup.width >= 42 {
        "↑↓/jk 选择 · Enter 推送 · Esc 取消"
    } else {
        "Enter推送 · Esc取消"
    };
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol("› ")
            .highlight_style(
                Style::default()
                    .fg(display_color_for(app.plain_mode(), ACCENT))
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .title("选择包导出项")
                    .title_bottom(hint)
                    .borders(Borders::ALL),
            ),
        popup,
        &mut state,
    );
}

/// 在终端区域中构造不会越界的居中矩形。
fn centered(width: u16, height: u16, area: Rect) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}
