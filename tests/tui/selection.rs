//! 可复用 TUI 选择栏的导航与结果测试。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::tui::{SelectionEvent, SelectionItem, SelectionState};

/// 创建无修饰按键。
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
// 选择栏统一支持上下导航、边界限制和确认返回值。
fn selection_state_navigates_and_returns_selected_value() {
    let mut state = SelectionState::new(vec![
        SelectionItem::new("全局", "后台运行", 1),
        SelectionItem::new("临时", "随界面退出", 2),
    ]);

    assert_eq!(state.handle_key(key(KeyCode::Up)), SelectionEvent::Pending);
    assert_eq!(state.selected(), 0);
    state.handle_key(key(KeyCode::Down));
    state.handle_key(key(KeyCode::Down));
    assert_eq!(state.selected(), 1);
    assert_eq!(
        state.handle_key(key(KeyCode::Enter)),
        SelectionEvent::Selected(2)
    );
}

#[test]
// Esc与q都通过统一取消事件退出选择。
fn selection_state_supports_consistent_cancellation() {
    let mut state = SelectionState::new(vec![SelectionItem::new("继续", "执行动作", ())]);

    assert_eq!(
        state.handle_key(key(KeyCode::Esc)),
        SelectionEvent::Cancelled
    );
    assert_eq!(
        state.handle_key(key(KeyCode::Char('q'))),
        SelectionEvent::Cancelled
    );
}
