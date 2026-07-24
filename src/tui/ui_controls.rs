//! 服务详情页面的自适应底栏与快捷键帮助。

use crate::protocol::SnapshotSourceDto;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::Paragraph,
};

use super::{ActiveTab, App, help_ui, key_hints, text_view, ui_support::display_color};

/// 绘制按宽度逐级收敛的键盘操作提示。
pub(super) fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let controls = if app.active_tab() == ActiveTab::Logs {
        log_controls(app, area.width)
    } else {
        task_controls(app, area.width)
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
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(display_color(app, Color::DarkGray))),
        area,
    );
}

/// 返回任务与依赖页按可用宽度逐级收敛的按键提示。
fn task_controls(app: &App, width: u16) -> String {
    let live = matches!(
        app.snapshot().source,
        SnapshotSourceDto::EmbeddedLive | SnapshotSourceDto::CenterLive
    );
    let exit = if app.back_navigation() {
        "q/Esc 返回"
    } else {
        "q/Esc 退出"
    };
    let short_exit = if app.back_navigation() {
        "q 返回"
    } else {
        "q 退出"
    };
    let auto_scroll = auto_scroll_label(app);
    let detailed = if live && app.control_allowed() && app.config_edit_allowed() {
        key_hints::join(&[
            "↑↓/jk 选择",
            "Tab 切页",
            "e 编辑",
            "s 启动",
            "x 停止",
            "r 重启",
            "←→ 横移",
            &format!("F3 自动:{auto_scroll}"),
            "? 帮助",
            exit,
        ])
    } else if live && app.control_allowed() {
        key_hints::join(&[
            "↑↓/jk 选择",
            "Tab 切页",
            "s 启动",
            "x 停止",
            "r 重启",
            "←→ 横移",
            &format!("F3 自动:{auto_scroll}"),
            "? 帮助",
            exit,
        ])
    } else {
        key_hints::join(&[
            "↑↓/jk 选择",
            "Tab 切页",
            "1/2/3 直达",
            "←→ 横移",
            &format!("F3 自动:{auto_scroll}"),
            "? 帮助",
            exit,
        ])
    };
    let action = if live && app.control_allowed() {
        Some("s/x/r 控制")
    } else {
        None
    };
    let standard = if live && app.control_allowed() && app.config_edit_allowed() {
        key_hints::join(&[
            "↑↓/jk选择",
            "Tab切页",
            "e编辑",
            "s启动",
            "x停止",
            "r重启",
            "?帮助",
            exit,
        ])
    } else if live && app.control_allowed() {
        key_hints::join(&[
            "↑↓/jk选择",
            "Tab切页",
            "s启动",
            "x停止",
            "r重启",
            "?帮助",
            exit,
        ])
    } else {
        key_hints::join(&["↑↓/jk 选择", "Tab 切页", "1/2/3 直达", "? 帮助", exit])
    };
    let mut medium = vec!["j/k 选择", "Tab 切页"];
    if app.config_edit_allowed() {
        medium.push("e 编辑");
    }
    if let Some(action) = action {
        medium.push(action);
    }
    medium.extend(["? 帮助", short_exit]);
    key_hints::adaptive(
        &[
            detailed,
            standard,
            key_hints::join(&medium),
            key_hints::join(&["j/k 选", "Tab 页", "? 帮助", short_exit]),
            key_hints::join(&["? 帮助", short_exit]),
        ],
        width,
    )
}

/// 返回日志页的平台键位与鼠标操作提示。
fn log_controls(app: &App, width: u16) -> String {
    let (page, boundary) = if app.mac_key_hints() {
        ("Fn+↑/↓ 翻页", "Fn+←/→ 首尾")
    } else {
        ("PgUp/PgDn 翻页", "Home/End 首尾")
    };
    let exit = if app.back_navigation() {
        "q/Esc 返回"
    } else {
        "q/Esc 退出"
    };
    let short_exit = if app.back_navigation() {
        "q 返回"
    } else {
        "q 退出"
    };
    key_hints::adaptive(
        &[
            key_hints::join(&[
                "↑↓/jk 换任务",
                "←→ 横移",
                page,
                boundary,
                "滚轮滚动",
                "/ 搜索",
                "n/N 匹配",
                "f 过滤",
                "v 来源",
                "C 清空",
                "? 帮助",
                exit,
            ]),
            key_hints::join(&[
                "j/k 换任务",
                page,
                "/ 搜索",
                "n/N 匹配",
                "f 过滤",
                "v 来源",
                "? 帮助",
                short_exit,
            ]),
            key_hints::join(&[page, "/ 搜索", "? 帮助", short_exit]),
            key_hints::join(&["? 帮助", short_exit]),
        ],
        width,
    )
}

/// 返回自动横移当前状态的短标签。
fn auto_scroll_label(app: &App) -> &'static str {
    if app.auto_scroll_enabled() && app.manual_scroll_frozen() {
        "开·冻结"
    } else if app.auto_scroll_enabled() {
        "开"
    } else {
        "关"
    }
}

/// 绘制与当前页签和能力相关的快捷键帮助。
pub(super) fn render_help(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines = vec![
        help_ui::key_line("↑↓ / j k", "切换 Task", app.plain_mode()),
        help_ui::key_line("Tab / Shift-Tab", "前后切换页面", app.plain_mode()),
        help_ui::key_line("1 / 2 / 3", "直达任务、依赖、日志页", app.plain_mode()),
        help_ui::key_line("← / →", "水平移动折叠文本", app.plain_mode()),
        help_ui::key_line("F3", "切换全局自动横移", app.plain_mode()),
    ];
    if app.active_tab() == ActiveTab::Logs {
        let page = if app.mac_key_hints() {
            "Fn+↑↓ / Fn+←→"
        } else {
            "PgUp/PgDn/Home/End"
        };
        lines.extend([
            help_ui::key_line(page, "日志翻页与首尾跳转", app.plain_mode()),
            help_ui::key_line("/ · n/N · f", "搜索、匹配跳转与过滤", app.plain_mode()),
            help_ui::key_line("v", "切换全部、Procora、子进程日志", app.plain_mode()),
            help_ui::key_line("C C", "二次确认清空当前 Task 日志", app.plain_mode()),
        ]);
    }
    if app.config_edit_allowed() {
        lines.push(help_ui::key_line(
            "e",
            "打开结构化配置编辑器",
            app.plain_mode(),
        ));
    }
    if app.control_allowed() {
        lines.push(help_ui::key_line(
            "s / x / r",
            "启动、停止、重启服务",
            app.plain_mode(),
        ));
    }
    let title = match app.active_tab() {
        ActiveTab::Tasks => "快捷键帮助 · 任务",
        ActiveTab::Dependencies => "快捷键帮助 · 依赖",
        ActiveTab::Logs => "快捷键帮助 · 日志",
    };
    help_ui::render(frame, area, title, lines, app.plain_mode());
}
