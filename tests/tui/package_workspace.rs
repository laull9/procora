//! Procora 包工作台的导航、保护与渲染测试。

use std::path::PathBuf;

use crossterm::event::KeyCode;
use procora::{
    package::{InstalledRelease, InstalledService},
    tui::{PackageWorkspaceApp, PackageWorkspaceExit, PackageWorkspaceTab},
};
use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

/// 创建具有活动与历史 release 的安装项。
fn installed_service() -> InstalledService {
    InstalledService {
        project: "demo".to_owned(),
        root: PathBuf::from("/managed/demo"),
        active_release: Some("new-release".to_owned()),
        pending_release: None,
        releases: vec![
            InstalledRelease {
                id: "new-release".to_owned(),
                sha256: "new".repeat(21),
                config_path: PathBuf::from("procora.yaml"),
                target_platform: None,
                deployed_at_ms: 2,
                active: true,
                pending: false,
            },
            InstalledRelease {
                id: "old-release".to_owned(),
                sha256: "old".repeat(21),
                config_path: PathBuf::from("procora.yaml"),
                target_platform: None,
                deployed_at_ms: 1,
                active: false,
                pending: false,
            },
        ],
        packages: Vec::new(),
        error: None,
    }
}

/// 把测试终端缓冲转换为便于断言的紧凑文本。
fn render_text(app: &PackageWorkspaceApp, width: u16, height: u16) -> String {
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
// 包文件与安装状态始终使用稳定的Tab导航和清晰空态。
fn tabs_and_empty_state_remain_discoverable() {
    let mut app = PackageWorkspaceApp::new(Vec::new(), Vec::new(), None);

    let packages = render_text(&app, 90, 22);
    assert!(packages.contains("包工作台"));
    assert!(packages.contains("尚无包文件"));
    assert!(packages.contains("b构建/o打开"));

    app.handle_key(KeyCode::Tab);
    assert_eq!(app.tab(), PackageWorkspaceTab::Installed);
    let installed = render_text(&app, 90, 22);
    assert!(installed.contains("当前用户尚未安装Procora包"));
}

#[test]
// 修改动作受控制能力保护，但验证、打开和解包保持可用。
fn mutating_actions_require_control_capability() {
    let mut app = PackageWorkspaceApp::new(Vec::new(), vec![installed_service()], None);
    app.handle_key(KeyCode::Tab);

    assert!(!app.handle_key(KeyCode::Char('R')));
    assert_eq!(app.take_exit(), None);

    app.set_control_allowed(true);
    assert!(app.handle_key(KeyCode::Char('R')));
    assert_eq!(
        app.take_exit(),
        Some(PackageWorkspaceExit::Rollback("demo".to_owned()))
    );
}

#[test]
// 永久清理必须连续两次大写D，其他按键会取消确认。
fn purge_requires_consecutive_uppercase_confirmation() {
    let mut app = PackageWorkspaceApp::new(Vec::new(), vec![installed_service()], None);
    app.set_control_allowed(true);
    app.handle_key(KeyCode::Tab);

    app.handle_key(KeyCode::Char('D'));
    assert_eq!(app.take_exit(), None);
    assert_eq!(app.purge_confirmation(), Some("demo"));
    assert!(app.feedback().unwrap().contains("再次按大写 D"));

    app.handle_key(KeyCode::Char('j'));
    assert_eq!(app.purge_confirmation(), None);
    app.handle_key(KeyCode::Char('D'));
    app.handle_key(KeyCode::Char('D'));
    assert_eq!(
        app.take_exit(),
        Some(PackageWorkspaceExit::Purge("demo".to_owned()))
    );
}

#[test]
// 解除安装同样需要连续确认，并与永久清理保持不同动作。
fn uninstall_confirmation_preserves_installed_data_intent() {
    let mut app = PackageWorkspaceApp::new(Vec::new(), vec![installed_service()], None);
    app.set_control_allowed(true);
    app.handle_key(KeyCode::Tab);

    app.handle_key(KeyCode::Char('U'));
    assert!(app.feedback().unwrap().contains("保留 release 和原始包"));
    assert_eq!(app.take_exit(), None);
    app.handle_key(KeyCode::Char('U'));
    assert_eq!(
        app.take_exit(),
        Some(PackageWorkspaceExit::Uninstall("demo".to_owned()))
    );
}

#[test]
// 已安装页显示活动与历史版本，并在帮助中解释恢复动作。
fn installed_details_and_recovery_help_are_visible() {
    let mut app = PackageWorkspaceApp::new(Vec::new(), vec![installed_service()], None);
    app.set_control_allowed(true);
    app.handle_key(KeyCode::Tab);

    let detail = render_text(&app, 110, 25);
    assert!(detail.contains("Activenew-release"));
    assert!(detail.contains("old-release"));

    app.handle_key(KeyCode::Char('?'));
    let help = render_text(&app, 100, 24);
    assert!(help.contains("回滚历史release"));
    assert!(help.contains("恢复pending安装"));
    assert!(help.contains("永久清理安装数据"));
}
