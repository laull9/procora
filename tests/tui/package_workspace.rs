//! Procora 包工作台的导航、保护与渲染测试。

use std::{path::PathBuf, time::Duration};

use crossterm::event::KeyCode;
use procora::{
    package::{InstalledRelease, InstalledService},
    tui::{PackageWorkspaceApp, PackageWorkspaceEntry, PackageWorkspaceExit, PackageWorkspaceTab},
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

/// 创建即使清单损坏也必须允许删除的本地包项。
fn broken_package() -> PackageWorkspaceEntry {
    PackageWorkspaceEntry {
        path: PathBuf::from("/downloads/损坏的超长包文件名称.pcpkg"),
        info: None,
        error: Some("清单损坏：无法读取包内 procora-package.json".to_owned()),
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
// 构建部署使用独立快捷键，且继续受控制能力和包文件视图约束。
fn build_and_deploy_preserves_permissions_and_tab_scope() {
    let mut app = PackageWorkspaceApp::new(Vec::new(), vec![installed_service()], None);

    assert!(!app.handle_key(KeyCode::Char('B')));
    assert_eq!(app.take_exit(), None);

    app.set_control_allowed(true);
    assert!(app.handle_key(KeyCode::Char('B')));
    assert_eq!(app.take_exit(), Some(PackageWorkspaceExit::BuildAndDeploy));

    app.handle_key(KeyCode::Tab);
    assert!(app.handle_key(KeyCode::Char('B')));
    assert_eq!(app.take_exit(), None);
    assert!(app.feedback().unwrap().contains("切换到“包文件”"));
}

#[test]
// 构建或打开新包后可精确选中新路径，并恢复包文件视图。
fn selecting_new_package_path_avoids_redeploying_old_package() {
    let first = broken_package();
    let mut second = broken_package();
    second.path = PathBuf::from("/downloads/demo-2.pcpkg");
    let expected = second.path.clone();
    let mut app = PackageWorkspaceApp::new(vec![first, second], vec![installed_service()], None);
    app.handle_key(KeyCode::Tab);

    assert!(app.select_package_path(&expected));
    assert_eq!(app.tab(), PackageWorkspaceTab::Packages);
    assert_eq!(app.selected_package().unwrap().path, expected);
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
// 损坏包仍可直接删除，且必须连续两次按下明确的删除键。
fn broken_package_delete_requires_consecutive_confirmation() {
    let path = broken_package().path;
    let mut app = PackageWorkspaceApp::new(vec![broken_package()], Vec::new(), None);
    app.set_control_allowed(true);

    app.handle_key(KeyCode::Delete);
    assert_eq!(app.package_delete_confirmation(), Some(path.as_path()));
    assert!(app.feedback().unwrap().contains("永久删除包文件"));
    assert_eq!(app.take_exit(), None);

    app.handle_key(KeyCode::Char('j'));
    assert_eq!(app.package_delete_confirmation(), None);
    app.handle_key(KeyCode::Char('X'));
    app.handle_key(KeyCode::Char('X'));
    assert_eq!(
        app.take_exit(),
        Some(PackageWorkspaceExit::DeletePackage(path))
    );
}

#[test]
// 包工作台与主TUI共享左右移动和F3自动横移语义。
fn workspace_supports_manual_and_automatic_horizontal_movement() {
    let mut service = installed_service();
    service.root =
        PathBuf::from("/managed/包含很长中文目录名称/以及更多无法在详情面板一次显示的路径/demo");
    service.error = Some("状态文件损坏，而且错误说明需要通过左右移动才能完整阅读".to_owned());
    let mut app = PackageWorkspaceApp::new(Vec::new(), vec![service], None);
    app.handle_key(KeyCode::Tab);

    app.handle_key(KeyCode::Right);
    app.handle_key(KeyCode::Right);
    assert_eq!(app.horizontal_offset(), 2);
    app.handle_key(KeyCode::Left);
    assert_eq!(app.horizontal_offset(), 1);

    app.handle_key(KeyCode::F(3));
    assert!(app.auto_scroll_enabled());
    assert!(app.advance_auto_scroll(Duration::from_secs(1)));

    let error_state = render_text(&app, 100, 24);
    assert!(error_state.contains("UU解除包托管"));
    assert!(error_state.contains("DD永久删除安装数据"));
    assert!(error_state.contains("同名普通Service始终保留"));
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
    assert!(help.contains("永久删除安装数据"));
    assert!(help.contains("永久删除当前包文件"));
    assert!(help.contains("移动被折叠的当前文本"));
}

#[test]
// 窄屏包工作台保留当前上下文、主操作和稳定返回路径。
fn compact_workspace_keeps_primary_actions_and_navigation() {
    let mut app = PackageWorkspaceApp::new(Vec::new(), vec![installed_service()], None);
    app.set_control_allowed(true);

    let packages = render_text(&app, 36, 10);
    assert!(packages.contains("尚无包"));
    assert!(packages.contains("B构建部署"));
    assert!(packages.contains("b构建"));
    assert!(packages.contains("Tab切换"));
    assert!(packages.contains("?帮助"));
    assert!(packages.contains("Esc返回"));

    app.handle_key(KeyCode::Tab);
    let installed = render_text(&app, 36, 10);
    assert!(installed.contains("当前安装"));
    assert!(installed.contains("demo"));
    assert!(installed.contains("R回滚"));
    assert!(installed.contains("c恢复"));
    assert!(installed.contains("D删除"));
}

#[test]
// 窄屏帮助层改为单行操作说明，并让关闭方法始终可见。
fn compact_workspace_help_reflows_instead_of_hiding_actions() {
    let mut app = PackageWorkspaceApp::new(Vec::new(), Vec::new(), None);
    app.set_control_allowed(true);
    app.handle_key(KeyCode::Char('?'));

    let help = render_text(&app, 36, 14);
    assert!(help.contains("Tab·切换包文件"));
    assert!(help.contains("b·从上下文Service"));
    assert!(help.contains("B·构建包"));
    assert!(help.contains("?/Esc关闭"));
}

#[test]
// 包工作台及帮助层在连续Resize到极小尺寸时都不会产生布局异常。
fn workspace_resize_matrix_is_safe() {
    let mut app = PackageWorkspaceApp::new(Vec::new(), vec![installed_service()], None);
    app.set_control_allowed(true);
    app.handle_key(KeyCode::Char('?'));

    for width in [8, 12, 16, 24, 32, 47, 48, 72] {
        for height in [2, 3, 4, 6, 10, 15, 16] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| app.render(frame)).unwrap();
        }
    }
}
