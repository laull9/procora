//! TUI 宽屏、紧凑和非正常状态的渲染测试。

mod support;

use crossterm::event::KeyCode;
use procora::protocol::SnapshotSourceDto;
use procora::tui::App;
use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

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
fn 宽屏任务页显示任务详情和连接状态() {
    let mut app = App::new(support::snapshot());
    app.handle_key(KeyCode::Down);

    let text = render_text(&app, 100, 24);

    assert!(text.contains("预览"));
    assert!(text.contains("api"));
    assert!(text.contains("等待database启动"));
}

#[test]
fn 紧凑终端仍显示任务列表和详情() {
    let app = App::new(support::snapshot());

    let text = render_text(&app, 40, 12);

    assert!(text.contains("database"));
    assert!(text.contains("详情"));
}

#[test]
fn 日志页简洁说明预览模式不提供日志() {
    let mut app = App::new(support::snapshot());
    app.handle_key(KeyCode::Char('3'));

    let text = render_text(&app, 80, 20);

    assert!(text.contains("预览模式不提供日志"));
}

#[test]
fn 临时服务显示实时控制且不展示开发说明() {
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
fn 纯文本模式使用ascii符号并关闭彩色依赖() {
    let mut app = App::new(support::snapshot());
    app.set_plain_mode(true);

    let text = render_text(&app, 60, 16);

    assert!(text.contains('+'));
    assert!(text.contains('o'));
    assert!(!text.contains('○'));
    assert!(!text.contains('┌'));
}
