//! TUI 键盘导航和选择状态测试。

use std::str::FromStr;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use procora::core::TaskId;
use procora::protocol::ServiceActionDto;
use procora::tui::{ActiveTab, App};

use crate::support;

#[test]
// 可以循环选择任务并切换页面。
fn task_selection_and_tabs_wrap_around() {
    let mut app = App::new(support::snapshot());

    app.handle_key(KeyCode::Down);
    assert_eq!(app.selected_index(), 1);
    app.handle_key(KeyCode::Down);
    assert_eq!(app.selected_index(), 0);

    app.handle_key(KeyCode::Tab);
    assert_eq!(app.active_tab(), ActiveTab::Dependencies);
    app.handle_key(KeyCode::Char('3'));
    assert_eq!(app.active_tab(), ActiveTab::Logs);
}

#[test]
// 左右键只移动当前文本视口，不再切换主页面。
fn horizontal_keys_do_not_change_tabs() {
    let mut app = App::new(support::snapshot());

    app.handle_key(KeyCode::Right);
    assert_eq!(app.active_tab(), ActiveTab::Tasks);
    assert_eq!(app.horizontal_offset(), 1);
    app.handle_key(KeyCode::Left);
    assert_eq!(app.active_tab(), ActiveTab::Tasks);
    assert_eq!(app.horizontal_offset(), 0);
}

#[test]
// 触控板横向滚动与左右方向键使用相同的文本横移行为。
fn horizontal_mouse_scroll_moves_text_viewport() {
    let mut app = App::new(support::snapshot());

    app.handle_mouse(mouse(MouseEventKind::ScrollRight));
    assert_eq!(app.horizontal_offset(), 1);
    app.handle_mouse(mouse(MouseEventKind::ScrollLeft));
    assert_eq!(app.horizontal_offset(), 0);
}

#[test]
// F3切换全局折叠文本自动滚动并可按固定步进推进。
fn f3_toggles_global_auto_scroll() {
    let mut app = App::new(support::snapshot());

    assert!(!app.auto_scroll_enabled());
    assert!(app.handle_key(KeyCode::F(3)));
    assert!(app.auto_scroll_enabled());
    assert!(app.advance_auto_scroll(Duration::from_millis(250)));
    assert!(app.handle_key(KeyCode::F(3)));
    assert!(!app.auto_scroll_enabled());
    assert!(!app.advance_auto_scroll(Duration::from_secs(1)));
}

#[test]
// 手动横移会冻结高亮文本十秒，同时全局自动滚动继续推进。
fn manual_scroll_freezes_selected_text_for_ten_seconds() {
    let mut app = App::new(support::snapshot());
    app.handle_key(KeyCode::F(3));
    assert!(app.advance_auto_scroll(Duration::from_secs(1)));

    app.handle_key(KeyCode::Right);
    assert_eq!(app.horizontal_offset(), 5);
    assert!(app.manual_scroll_frozen());
    assert!(app.advance_auto_scroll(Duration::from_secs(9)));
    assert!(app.manual_scroll_frozen());
    assert!(app.advance_auto_scroll(Duration::from_secs(1)));
    assert!(!app.manual_scroll_frozen());
}

#[test]
// 服务控制键会形成一次可消费动作。
fn service_control_key_creates_single_consumable_action() {
    let mut app = App::new(support::snapshot());
    app.set_control_allowed(true);
    app.handle_key(KeyCode::Char('r'));
    assert_eq!(app.take_pending_action(), Some(ServiceActionDto::Restart));
    assert_eq!(app.take_pending_action(), None);
}

#[test]
// 无控制权限时忽略服务动作键。
fn service_action_keys_are_ignored_without_permission() {
    let mut app = App::new(support::snapshot());
    app.handle_key(KeyCode::Char('x'));
    assert_eq!(app.take_pending_action(), None);
}

#[test]
// 只有具有本地配置入口的中心服务才响应内嵌编辑快捷键。
fn config_edit_key_requires_explicit_capability() {
    let mut app = App::new(support::snapshot());

    app.handle_key(KeyCode::Char('e'));
    assert!(!app.take_pending_config_edit());

    app.set_config_edit_allowed(true);
    app.handle_key(KeyCode::Char('e'));
    assert!(app.take_pending_config_edit());
    assert!(!app.take_pending_config_edit());
}

#[test]
// 退出键只改变本地退出状态。
fn quit_key_only_changes_local_exit_state() {
    let mut app = App::new(support::snapshot());

    app.handle_key(KeyCode::Char('q'));

    assert!(app.should_quit());
}

#[test]
// ctrl_c会请求正常退出。
fn ctrl_c_requests_clean_exit() {
    let mut app = App::new(support::snapshot());

    app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(app.should_quit());
}

#[test]
// task日志会保留间隙标记和最新内容。
fn task_logs_keep_gap_marker_and_latest_content() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("api").unwrap();

    app.append_log(task_id.clone(), b"first\n", true);
    app.append_log(task_id.clone(), b"second\n", false);

    assert_eq!(app.log_text(&task_id).as_deref(), Some("first\nsecond\n"));
    assert!(app.has_log_gap(&task_id));
}

#[test]
// TUI会话不再把日志展示截断到64KiB。
fn task_logs_keep_history_beyond_previous_display_limit() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("api").unwrap();
    let content = format!("first-line\n{}\nlast-line\n", "x".repeat(70 * 1024));

    app.append_log(task_id.clone(), content.as_bytes(), false);

    let logs = app.log_text(&task_id).unwrap();
    assert!(logs.starts_with("first-line\n"));
    assert!(logs.ends_with("\nlast-line\n"));
}

#[test]
// 日志搜索、过滤、匹配跳转和清空请求共享当前Task状态。
fn log_search_filter_navigation_and_clear_are_stateful() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    app.append_log(task_id.clone(), b"ok\nERROR one\nerror two\n", false);
    app.handle_key(KeyCode::Char('3'));

    app.handle_key(KeyCode::Char('/'));
    for character in "error".chars() {
        app.handle_key(KeyCode::Char(character));
    }
    app.handle_key(KeyCode::Enter);
    assert_eq!(app.log_query(), "error");
    assert_eq!(app.log_match_position(&task_id), Some((1, 2)));

    app.handle_key(KeyCode::Char('n'));
    assert_eq!(app.log_match_position(&task_id), Some((2, 2)));
    app.handle_key(KeyCode::Char('f'));
    assert!(app.log_filter_enabled());

    app.handle_key(KeyCode::Char('C'));
    assert_eq!(app.take_pending_log_clear(), None);
    app.handle_key(KeyCode::Char('C'));
    assert_eq!(app.take_pending_log_clear(), Some(task_id.clone()));
    assert!(app.clear_log(&task_id));
    assert!(app.log_text(&task_id).is_none());
}

#[test]
// 日志支持翻页首尾和恢复自动跟随。
fn logs_support_paging_boundaries_and_follow_mode() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    let content = "line\n".repeat(60);
    app.append_log(task_id.clone(), content.as_bytes(), false);
    app.handle_key(KeyCode::Char('3'));

    assert_eq!(app.log_scroll_distance(&task_id), 0);
    app.handle_key(KeyCode::PageUp);
    assert_eq!(app.log_scroll_distance(&task_id), 20);
    app.handle_key(KeyCode::PageUp);
    assert_eq!(app.log_scroll_distance(&task_id), 40);
    app.handle_key(KeyCode::PageDown);
    assert_eq!(app.log_scroll_distance(&task_id), 20);
    app.handle_key(KeyCode::Home);
    assert_eq!(app.log_scroll_distance(&task_id), 60);
    app.handle_key(KeyCode::End);
    assert_eq!(app.log_scroll_distance(&task_id), 0);
}

#[test]
// 上翻后新日志不会抢走当前阅读位置。
fn new_logs_preserve_scrolled_reading_position() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    let content = "old\n".repeat(40);
    app.append_log(task_id.clone(), content.as_bytes(), false);
    app.handle_key(KeyCode::Char('3'));
    app.handle_key(KeyCode::PageUp);

    app.append_log(task_id.clone(), b"new-1\nnew-2\n", false);

    assert_eq!(app.log_scroll_distance(&task_id), 22);
    assert_eq!(app.log_scroll_top(&task_id, 10), 10);
}

#[test]
// 日志页滚轮只滚动日志而不切换task。
fn log_wheel_scrolls_content_without_switching_task() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("database").unwrap();
    app.append_log(task_id.clone(), "line\n".repeat(40).as_bytes(), false);
    app.handle_key(KeyCode::Char('3'));

    app.handle_mouse(mouse(MouseEventKind::ScrollUp));

    assert_eq!(app.selected_index(), 0);
    assert_eq!(app.log_scroll_distance(&task_id), 3);
    app.handle_mouse(mouse(MouseEventKind::ScrollDown));
    assert_eq!(app.log_scroll_distance(&task_id), 0);
}

#[test]
// 非日志页滚轮继续切换task。
fn wheel_switches_tasks_outside_log_tab() {
    let mut app = App::new(support::snapshot());

    app.handle_mouse(mouse(MouseEventKind::ScrollDown));

    assert_eq!(app.selected_index(), 1);
}

#[test]
// 相同状态和空日志不会触发重复重绘。
fn unchanged_state_and_empty_logs_do_not_redraw() {
    let snapshot = support::snapshot();
    let mut app = App::new(snapshot.clone());
    let task_id = TaskId::from_str("api").unwrap();

    assert!(!app.replace_snapshot(snapshot));
    assert!(app.set_feedback("连接异常"));
    assert!(!app.set_feedback("连接异常"));
    assert!(!app.append_log(task_id, &[], false));
    assert!(!app.handle_key(KeyCode::Char('z')));
}

/// 创建不依赖真实终端坐标的滚轮事件。
fn mouse(kind: MouseEventKind) -> MouseEvent {
    MouseEvent {
        kind,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    }
}
