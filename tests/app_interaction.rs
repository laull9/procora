//! TUI 键盘导航和选择状态测试。

mod support;

use std::str::FromStr;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use procora::core::TaskId;
use procora::protocol::ServiceActionDto;
use procora::tui::{ActiveTab, App};

#[test]
fn 可以循环选择任务并切换页面() {
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
fn 服务控制键会形成一次可消费动作() {
    let mut app = App::new(support::snapshot());
    app.set_control_allowed(true);
    app.handle_key(KeyCode::Char('r'));
    assert_eq!(app.take_pending_action(), Some(ServiceActionDto::Restart));
    assert_eq!(app.take_pending_action(), None);
}

#[test]
fn 无控制权限时忽略服务动作键() {
    let mut app = App::new(support::snapshot());
    app.handle_key(KeyCode::Char('x'));
    assert_eq!(app.take_pending_action(), None);
}

#[test]
fn 退出键只改变本地退出状态() {
    let mut app = App::new(support::snapshot());

    app.handle_key(KeyCode::Char('q'));

    assert!(app.should_quit());
}

#[test]
fn ctrl_c会请求正常退出() {
    let mut app = App::new(support::snapshot());

    app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(app.should_quit());
}

#[test]
fn task日志会保留间隙标记和最新内容() {
    let mut app = App::new(support::snapshot());
    let task_id = TaskId::from_str("api").unwrap();

    app.append_log(task_id.clone(), b"first\n", true);
    app.append_log(task_id.clone(), b"second\n", false);

    assert_eq!(app.log_text(&task_id).as_deref(), Some("first\nsecond\n"));
    assert!(app.has_log_gap(&task_id));
}

#[test]
fn 日志支持翻页首尾和恢复自动跟随() {
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
fn 上翻后新日志不会抢走当前阅读位置() {
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
fn 日志页滚轮只滚动日志而不切换task() {
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
fn 非日志页滚轮继续切换task() {
    let mut app = App::new(support::snapshot());

    app.handle_mouse(mouse(MouseEventKind::ScrollDown));

    assert_eq!(app.selected_index(), 1);
}

#[test]
fn 相同状态和空日志不会触发重复重绘() {
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
