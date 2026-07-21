//! 全局服务总览的交互与渲染测试。

use std::path::PathBuf;

use crossterm::event::KeyCode;
use procora::protocol::{ServiceStatusDto, ServiceViewDto};
use procora::tui::{OverviewAction, OverviewApp, OverviewExit};
use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

/// 创建总览测试使用的服务摘要。
fn service(name: &str, status: ServiceStatusDto) -> ServiceViewDto {
    ServiceViewDto {
        name: name.to_owned(),
        root: PathBuf::from(format!("/services/{name}")),
        config_path: PathBuf::from(format!("/services/{name}/procora.yaml")),
        status,
        task_count: 3,
        message: (status == ServiceStatusDto::Failed).then(|| "配置加载失败".to_owned()),
    }
}

/// 把测试终端缓冲转换为便于断言的文本。
fn render_text(app: &OverviewApp, width: u16, height: u16) -> String {
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
// 服务按稳定名称排序，选择可循环并通过Enter进入详情。
fn selection_sorts_wraps_and_opens_details() {
    let mut app = OverviewApp::new(vec![
        service("worker", ServiceStatusDto::Stopped),
        service("api", ServiceStatusDto::Running),
    ]);

    assert_eq!(app.selected_service().unwrap().name, "api");
    app.handle_key(KeyCode::Up);
    assert_eq!(app.selected_service().unwrap().name, "worker");
    app.handle_key(KeyCode::Enter);
    assert_eq!(
        app.take_exit(),
        Some(OverviewExit::OpenService("worker".to_owned()))
    );
}

#[test]
// 总览控制键生成一次动作，移除必须连续确认两次。
fn management_keys_queue_actions_with_remove_confirmation() {
    let mut app = OverviewApp::new(vec![service("api", ServiceStatusDto::Running)]);
    app.set_control_allowed(true);

    app.handle_key(KeyCode::Char('x'));
    assert_eq!(
        app.take_pending_action(),
        Some(("api".to_owned(), OverviewAction::Stop))
    );
    app.handle_key(KeyCode::Char('d'));
    assert_eq!(app.take_pending_action(), None);
    assert!(app.feedback().unwrap().contains("再次按 d"));
    app.handle_key(KeyCode::Char('d'));
    assert_eq!(
        app.take_pending_action(),
        Some(("api".to_owned(), OverviewAction::Remove))
    );
}

#[test]
// 服务刷新后按稳定名称保持选择，已删除项回退到有效索引。
fn refresh_preserves_stable_selection() {
    let mut app = OverviewApp::new(vec![
        service("api", ServiceStatusDto::Running),
        service("worker", ServiceStatusDto::Stopped),
    ]);
    app.handle_key(KeyCode::Down);

    assert!(app.replace_services(vec![
        service("worker", ServiceStatusDto::Running),
        service("queue", ServiceStatusDto::Stopped),
    ]));
    assert_eq!(app.selected_service().unwrap().name, "worker");
    assert!(app.replace_services(vec![service("queue", ServiceStatusDto::Stopped)]));
    assert_eq!(app.selected_service().unwrap().name, "queue");
}

#[test]
// 宽屏总览显示全部状态摘要、选中服务详情和管理提示。
fn wide_overview_shows_status_details_and_controls() {
    let mut app = OverviewApp::new(vec![
        service("api", ServiceStatusDto::Running),
        service("broken", ServiceStatusDto::Failed),
    ]);
    app.set_control_allowed(true);

    let text = render_text(&app, 110, 24);

    assert!(text.contains("服务总览"));
    assert!(text.contains("共2个·运行1·失败1"));
    assert!(text.contains("目录/services/api"));
    assert!(text.contains("Enter打开详情"));
    assert!(text.contains("d移除"));
}

#[test]
// 空列表和紧凑终端都保留清晰恢复路径。
fn empty_and_compact_overview_keep_recovery_hints() {
    let app = OverviewApp::new(Vec::new());

    let wide = render_text(&app, 80, 18);
    assert!(wide.contains("尚未注册服务"));
    assert!(wide.contains("procoraadd<path>"));

    let compact = render_text(&app, 29, 7);
    assert!(compact.contains("服务总览"));
    assert!(compact.contains("q/Esc退出"));
}
