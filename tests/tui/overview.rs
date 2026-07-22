//! 全局服务总览的交互与渲染测试。

use std::path::PathBuf;

use crossterm::event::KeyCode;
use procora::protocol::{ResourceUsageDto, ServiceStatusDto, ServiceViewDto};
use procora::tui::{OverviewAction, OverviewApp, OverviewExit, OverviewSort};
use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

/// 创建总览测试使用的服务摘要。
fn service(name: &str, status: ServiceStatusDto) -> ServiceViewDto {
    ServiceViewDto {
        name: name.to_owned(),
        root: PathBuf::from(format!("/services/{name}")),
        config_path: PathBuf::from(format!("/services/{name}/procora.yaml")),
        status,
        task_count: 3,
        resources: (status == ServiceStatusDto::Running).then_some(ResourceUsageDto {
            cpu_tenths_percent: Some(125),
            memory_bytes: Some(64 * 1024 * 1024),
        }),
        message: (status == ServiceStatusDto::Failed).then(|| "配置加载失败".to_owned()),
    }
}

/// 向总览输入一段筛选文本。
fn type_filter(app: &mut OverviewApp, query: &str) {
    app.handle_key(KeyCode::Char('/'));
    for character in query.chars() {
        app.handle_key(KeyCode::Char(character));
    }
    app.handle_key(KeyCode::Enter);
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

/// 返回 ASCII 标记在总览测试终端中的所有起始列。
fn marker_columns(app: &OverviewApp, width: u16, height: u16, marker: &str) -> Vec<u16> {
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
// 总览的新建快捷键只在允许控制时打开创建向导。
fn create_key_requires_control_capability() {
    let mut app = OverviewApp::new(Vec::new());

    app.handle_key(KeyCode::Char('n'));
    assert_eq!(app.take_exit(), None);

    app.set_control_allowed(true);
    app.handle_key(KeyCode::Char('n'));
    assert_eq!(app.take_exit(), Some(OverviewExit::CreateService));
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
// 筛选实时匹配名称、路径和状态，排序可循环字段并切换方向。
fn filter_and_sort_cover_service_fields_and_resources() {
    let mut api = service("api", ServiceStatusDto::Running);
    api.resources = Some(ResourceUsageDto {
        cpu_tenths_percent: Some(50),
        memory_bytes: Some(32 * 1024 * 1024),
    });
    let mut worker = service("worker", ServiceStatusDto::Running);
    worker.resources = Some(ResourceUsageDto {
        cpu_tenths_percent: Some(300),
        memory_bytes: Some(128 * 1024 * 1024),
    });
    let mut app = OverviewApp::new(vec![worker, api]);

    type_filter(&mut app, "API");
    assert_eq!(app.visible_services().len(), 1);
    assert_eq!(app.selected_service().unwrap().name, "api");
    assert_eq!(app.filter_query(), "API");

    app.handle_key(KeyCode::Char('/'));
    for _ in 0..3 {
        app.handle_key(KeyCode::Backspace);
    }
    app.handle_key(KeyCode::Enter);
    assert_eq!(app.visible_services().len(), 2);
    let totals = app.visible_resources().unwrap();
    assert_eq!(totals.cpu_tenths_percent, Some(350));
    assert_eq!(totals.memory_bytes, Some(160 * 1024 * 1024));

    app.handle_key(KeyCode::Char('/'));
    for character in "missing".chars() {
        app.handle_key(KeyCode::Char(character));
    }
    assert!(app.visible_services().is_empty());
    app.handle_key(KeyCode::Esc);
    assert_eq!(app.visible_services().len(), 2);
    assert!(app.filter_query().is_empty());

    app.handle_key(KeyCode::Char('o'));
    app.handle_key(KeyCode::Char('o'));
    assert_eq!(app.sort(), OverviewSort::Cpu);
    assert!(app.sort_descending());
    assert_eq!(app.visible_services()[0].name, "worker");
    app.handle_key(KeyCode::Char('O'));
    assert_eq!(app.visible_services()[0].name, "api");
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
    assert!(text.contains("显示2/2·运行1·失败1"));
    assert!(text.contains("CPU12.5%"));
    assert!(text.contains("内存64.0MiB"));
    assert!(text.contains("目录/services/api"));
    assert!(text.contains("Enter详情"));
    assert!(text.contains("/筛选"));
    assert!(text.contains("o排序"));
}

#[test]
// 服务详情中的Task、CPU和内存数据从同一终端列开始。
fn overview_detail_values_align_in_terminal_columns() {
    let mut api = service("api", ServiceStatusDto::Running);
    api.task_count = 37;
    let app = OverviewApp::new(vec![api]);

    let task_column = marker_columns(&app, 110, 24, "37")
        .into_iter()
        .max()
        .expect("应渲染 Task 数量");
    let cpu_columns = marker_columns(&app, 110, 24, "12.5%");
    let memory_columns = marker_columns(&app, 110, 24, "64.0 MiB");

    assert!(cpu_columns.contains(&task_column));
    assert!(memory_columns.contains(&task_column));
}

#[test]
// 空列表和紧凑终端都保留清晰恢复路径。
fn empty_and_compact_overview_keep_recovery_hints() {
    let mut app = OverviewApp::new(Vec::new());
    app.set_control_allowed(true);

    let wide = render_text(&app, 80, 18);
    assert!(wide.contains("尚未注册服务"));
    assert!(wide.contains("按n选择托管目录"));

    let compact = render_text(&app, 29, 7);
    assert!(compact.contains("服务总览"));
    assert!(compact.contains("n新建"));
    assert!(compact.contains("q/Esc退出"));
}
