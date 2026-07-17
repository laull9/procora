//! TUI 宽屏、紧凑和非正常状态的渲染测试。

mod support;

use crossterm::event::KeyCode;
use procora::core::TaskId;
use procora::protocol::SnapshotSourceDto;
use procora::tui::App;
use ratatui::{Terminal, backend::TestBackend, buffer::Cell};
use std::{fmt::Write as _, str::FromStr};

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
