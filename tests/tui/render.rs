//! TUI 宽屏、紧凑和非正常状态的渲染测试。

use crossterm::event::KeyCode;
use procora::core::TaskId;
use procora::protocol::{
    ResourceUsageDto, SnapshotSourceDto, TaskDiagnosticDto, TaskDiagnosticKindDto,
};
use procora::tui::App;
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Cell,
    style::{Color, Modifier},
};
use std::time::Duration;
use std::{fmt::Write as _, str::FromStr};

use crate::support;

/// 把测试终端缓冲转换成便于断言的文本。
fn render_text(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "")
}

/// 返回 ASCII 标记在测试终端中的所有起始列。
fn marker_columns(app: &App, width: u16, height: u16, marker: &str) -> Vec<u16> {
    assert!(marker.is_ascii());
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let symbols = marker
        .chars()
        .map(|character| character.to_string())
        .collect::<Vec<_>>();
    let marker_width = u16::try_from(symbols.len()).expect("测试标记长度应适合终端宽度");
    let mut columns = Vec::new();
    for y in 0..height {
        for x in 0..=width.saturating_sub(marker_width) {
            let matches = symbols.iter().enumerate().all(|(offset, expected)| {
                let index = usize::from(y) * usize::from(width) + usize::from(x) + offset;
                buffer.content[index].symbol() == expected
            });
            if matches {
                columns.push(x);
            }
        }
    }
    columns
}

/// 返回 ASCII 标记最靠右的起始列。
fn rightmost_marker_column(app: &App, width: u16, height: u16, marker: &str) -> u16 {
    marker_columns(app, width, height, marker)
        .into_iter()
        .max()
        .unwrap_or_else(|| panic!("未找到渲染标记：{marker}"))
}

#[test]
// Task详情中的英文和中文字段值从同一终端列开始。
fn task_detail_values_align_in_terminal_columns() {
    let mut snapshot = support::snapshot();
    snapshot.tasks[0].resources = Some(ResourceUsageDto {
        cpu_tenths_percent: Some(237),
        memory_bytes: Some(64 * 1024 * 1024),
    });
    let app = App::new(snapshot);

    let command_column = rightmost_marker_column(&app, 100, 24, "postgres");
    let cpu_column = rightmost_marker_column(&app, 100, 24, "23.7%");
    let memory_column = rightmost_marker_column(&app, 100, 24, "64.0 MiB");

    assert_eq!(cpu_column, command_column);
    assert_eq!(memory_column, command_column);
}

#[test]
// 宽屏任务页显示任务详情和连接状态。
fn wide_task_view_shows_details_and_connection_state() {
    let mut app = App::new(support::snapshot());
    app.handle_key(KeyCode::Down);

    let text = render_text(&app, 100, 24);

    assert!(text.contains("预览"));
    assert!(text.contains("api"));
    assert!(text.contains("等待database启动"));
}

#[test]
// Task运行诊断在详情页汇总为综合分析并给出处理建议。
fn task_details_render_diagnostic_analysis() {
    let mut snapshot = support::snapshot();
    snapshot.tasks[0].diagnostics = vec![TaskDiagnosticDto {
        kind: TaskDiagnosticKindDto::Spawn,
        message: "Task 启动失败：未找到文件或目录".to_owned(),
        suggestion: Some("检查 command 和工作目录".to_owned()),
        occurrences: 2,
    }];
    let app = App::new(snapshot);

    let text = render_text(&app, 100, 24);

    assert!(text.contains("综合分析·1类·2次"));
    assert!(text.contains("启动Task启动失败：未找到文件或目录（2次）"));
    assert!(text.contains("建议检查command和工作目录"));
}

#[test]
// 水平移动只影响溢出的长字段，未折叠的字段保持原位。
fn horizontal_scroll_only_moves_overflowing_task_text() {
    let mut snapshot = support::snapshot();
    snapshot.tasks[0].command = format!("BEGIN-{}-END", "x".repeat(100));
    snapshot.tasks[0].message = Some("短说明".to_owned());
    let mut app = App::new(snapshot);

    let initial = render_text(&app, 80, 20);
    for _ in 0..8 {
        app.handle_key(KeyCode::Right);
    }
    let shifted = render_text(&app, 80, 20);

    assert!(initial.contains("BEGIN-"));
    assert!(!shifted.contains("BEGIN-"));
    assert!(shifted.contains("任务database"));
    assert!(shifted.contains("健康未配置"));
    assert!(shifted.contains("说明短说明"));
}

#[test]
// F3自动滚动会移动全局所有溢出文本，包括未高亮的列表项。
fn automatic_horizontal_scroll_moves_non_selected_overflowing_text() {
    let mut snapshot = support::snapshot();
    snapshot.tasks[1].task_id = TaskId::from_str(&format!("prefix-{}", "x".repeat(70))).unwrap();
    let mut app = App::new(snapshot);

    for _ in 0..8 {
        app.handle_key(KeyCode::Right);
    }
    assert!(render_text(&app, 80, 20).contains("prefix-"));

    app.handle_key(KeyCode::F(3));
    assert!(app.advance_auto_scroll(Duration::from_secs(2)));
    assert!(!render_text(&app, 80, 20).contains("prefix-"));
}

#[test]
// 开启自动横移后，未溢出的短字段仍保持原位。
fn automatic_horizontal_scroll_keeps_non_overflowing_fields_fixed() {
    let mut snapshot = support::snapshot();
    snapshot.tasks[0].command = format!("BEGIN-{}-END", "x".repeat(100));
    snapshot.tasks[0].message = Some("SHORT-STABLE".to_owned());
    let mut app = App::new(snapshot);

    app.handle_key(KeyCode::F(3));
    assert!(app.advance_auto_scroll(Duration::from_secs(2)));
    let shifted = render_text(&app, 80, 20);

    assert!(!shifted.contains("BEGIN-"));
    assert!(shifted.contains("说明SHORT-STABLE"));
    assert!(shifted.contains("任务database"));
}

#[test]
// 紧凑终端仍显示任务列表和详情。
fn compact_terminal_shows_task_list_and_details() {
    let app = App::new(support::snapshot());

    let text = render_text(&app, 40, 12);

    assert!(text.contains("database"));
    assert!(text.contains("详情"));
}

#[test]
// 日志页简洁说明预览模式不提供日志。
fn log_tab_explains_preview_mode_without_logs() {
    let mut app = App::new(support::snapshot());
    app.handle_key(KeyCode::Char('3'));

    let text = render_text(&app, 80, 20);

    assert!(text.contains("预览模式不提供日志"));
}

#[test]
// 小终端摘要保留项目来源task状态和退出提示。
fn minimal_terminal_preserves_project_source_task_and_exit() {
    let mut app = App::new(support::snapshot());
    app.handle_key(KeyCode::Down);

    let text = render_text(&app, 24, 6);

    assert!(text.contains("Procora·demo"));
    assert!(text.contains("预览"));
    assert!(text.contains("api·依赖阻断"));
    assert!(text.contains("q/Esc退出"));
}

#[test]
// 窄底栏优先保留帮助与退出入口，且帮助快捷键能打开完整说明。
fn narrow_footer_keeps_help_and_renders_overlay() {
    let mut app = App::new(support::snapshot());

    let narrow = render_text(&app, 32, 12);
    assert!(narrow.contains("?帮助"));
    assert!(narrow.contains("q退出"));

    app.handle_key(KeyCode::Char('?'));
    let help = render_text(&app, 80, 20);
    assert!(help.contains("快捷键帮助·任务"));
    assert!(help.contains("直达任务、依赖、日志页"));
    assert!(help.contains("关闭帮助"));
}

#[test]
// 从总览进入的服务详情把退出键提示为返回上一级。
fn overview_detail_uses_back_navigation_label() {
    let mut app = App::new(support::snapshot());
    app.set_back_navigation(true);

    let text = render_text(&app, 80, 20);

    assert!(text.contains("q/Esc返回"));
    assert!(!text.contains("q/Esc退出"));
}

#[test]
// 极小终端至少保留产品名和恢复原因。
fn tiny_terminal_preserves_product_and_recovery_reason() {
    let app = App::new(support::snapshot());

    let text = render_text(&app, 12, 3);

    assert!(text.contains("Procora"));
    assert!(text.contains("终端过小"));
}

#[test]
// 日志页默认显示尾部并可翻到历史内容。
fn log_tab_follows_tail_and_pages_to_history() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    let mut content = String::new();
    for line in 1..=40 {
        writeln!(content, "record-{line:02}").unwrap();
    }
    app.append_log(task_id, content.as_bytes(), false);
    app.handle_key(KeyCode::Char('3'));

    let tail = render_text(&app, 80, 16);
    assert!(tail.contains("跟随尾部"));
    assert!(tail.contains("record-40"));
    assert!(!tail.contains("record-01"));

    app.handle_key(KeyCode::Home);
    let history = render_text(&app, 80, 16);
    assert!(history.contains("已上翻40行"));
    assert!(history.contains("record-01"));
    assert!(!history.contains("record-40"));
}

#[test]
// ANSI彩色日志会被解析为终端样式而不是显示原始转义字符。
fn ansi_log_colors_render_as_terminal_styles() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    app.append_log(task_id, b"plain \x1b[31mRED\x1b[0m\n", false);
    app.set_plain_mode(false);
    app.handle_key(KeyCode::Char('3'));
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer
            .content
            .iter()
            .any(|cell| cell.symbol() == "R" && cell.fg == ratatui::style::Color::Red)
    );
    assert!(!render_text(&app, 80, 16).contains("[31m"));
}

#[test]
// 日志来源标签低调着色，v键会过滤内容且保留诊断斜体样式。
fn log_source_filters_separate_procora_and_child_output() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    app.append_log(
        task_id,
        b"CHILD-ONLY\n\x1b[2;3;91m[Procora \xe8\xaf\x8a\xe6\x96\xad \xc2\xb7 \xe5\x90\xaf\xe5\x8a\xa8] PROCORA-ONLY\x1b[0m\n",
        false,
    );
    app.set_plain_mode(false);
    app.handle_key(KeyCode::Char('3'));

    let all = render_text(&app, 120, 16);
    assert!(all.contains("CHILD-ONLY"));
    assert!(all.contains("PROCORA-ONLY"));

    app.handle_key(KeyCode::Char('v'));
    let procora = render_text(&app, 120, 16);
    assert!(!procora.contains("CHILD-ONLY"));
    assert!(procora.contains("PROCORA-ONLY"));
    let backend = TestBackend::new(120, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    assert!(terminal.backend().buffer().content.iter().any(|cell| {
        cell.symbol() == "P"
            && cell.fg == Color::LightRed
            && cell.modifier.contains(Modifier::ITALIC)
    }));

    app.handle_key(KeyCode::Char('v'));
    let child = render_text(&app, 120, 16);
    assert!(child.contains("CHILD-ONLY"));
    assert!(!child.contains("PROCORA-ONLY"));
    let backend = TestBackend::new(120, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    assert!(terminal.backend().buffer().content.iter().any(|cell| {
        cell.symbol() == "子" && cell.fg == Color::Green && cell.modifier.contains(Modifier::BOLD)
    }));
}

#[test]
// 搜索会标记匹配行，过滤模式会隐藏不匹配行。
fn log_search_filter_hides_non_matching_lines() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    app.append_log(task_id, b"keep error\nhide ok\n", false);
    app.handle_key(KeyCode::Char('3'));
    app.handle_key(KeyCode::Char('/'));
    for character in "error".chars() {
        app.handle_key(KeyCode::Char(character));
    }
    app.handle_key(KeyCode::Enter);
    app.handle_key(KeyCode::Char('f'));

    let text = render_text(&app, 100, 16);
    assert!(text.contains("keeperror"));
    assert!(!text.contains("hideok"));
    assert!(text.contains("过滤`error`1/1"));
}

#[test]
// 日志是统一的大文本视口，短行会与长行使用相同的水平偏移。
fn log_horizontal_scroll_moves_the_whole_text_viewport() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    let content = format!("short-line\nBEGIN-{}-END\n", "x".repeat(100));
    app.append_log(task_id, content.as_bytes(), false);
    app.handle_key(KeyCode::Char('3'));

    for _ in 0..8 {
        app.handle_key(KeyCode::Right);
    }
    let shifted = render_text(&app, 50, 16);

    assert!(!shifted.contains("short-line"));
    assert!(!shifted.contains("BEGIN-"));
}

#[test]
// 日志自动横移与手动横移保持相同的整块文本视口语义。
fn automatic_log_horizontal_scroll_moves_the_whole_text_viewport() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    let content = format!("short-line\nBEGIN-{}-END\n", "x".repeat(100));
    app.append_log(task_id, content.as_bytes(), false);
    app.handle_key(KeyCode::Char('3'));

    app.handle_key(KeyCode::F(3));
    assert!(app.advance_auto_scroll(Duration::from_secs(2)));
    let shifted = render_text(&app, 50, 16);

    assert!(!shifted.contains("short-line"));
    assert!(!shifted.contains("BEGIN-"));
}

#[test]
// mac日志页展示fn键位和宽屏task切换提示。
fn mac_log_view_shows_fn_keys_and_task_switch_hint() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    app.append_log(task_id, b"log\n", false);
    app.set_plain_mode(false);
    app.set_mac_key_hints(true);
    app.handle_key(KeyCode::Char('3'));

    let wide = render_text(&app, 120, 16);
    assert!(wide.contains("Fn+↑/↓翻页"));
    assert!(wide.contains("Fn+←/→首尾"));
    assert!(wide.contains("滚轮滚动"));
    assert!(wide.contains("↑/↓切换任务日志"));

    let narrow = render_text(&app, 35, 16);
    assert!(!narrow.contains("↑/↓切换任务日志"));
}

#[test]
// 临时服务显示实时控制且不展示开发说明。
fn embedded_service_shows_live_controls_without_dev_copy() {
    let mut snapshot = support::snapshot();
    snapshot.source = SnapshotSourceDto::EmbeddedLive;
    let mut app = App::new(snapshot);
    app.set_control_allowed(true);

    let text = render_text(&app, 100, 20);

    assert!(text.contains("临时服务"));
    assert!(text.contains("s启动"));
    assert!(!text.contains("中心前端模式"));
}

#[test]
// 纯文本模式使用ascii符号并关闭彩色依赖。
fn plain_mode_uses_ascii_without_colored_dependencies() {
    let mut app = App::new(support::snapshot());
    app.set_plain_mode(true);

    let text = render_text(&app, 60, 16);

    assert!(text.contains('+'));
    assert!(text.contains('o'));
    assert!(!text.contains('○'));
    assert!(!text.contains('┌'));
}
