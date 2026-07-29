//! 可复用 TUI 选择栏的导航与结果测试。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use procora::tui::{SelectionEvent, SelectionItem, SelectionState};
use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

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

#[test]
// 窄选择栏截断次要说明，但始终保留确认和取消入口。
fn compact_selection_keeps_labels_and_recovery_keys() {
    let state = SelectionState::new(vec![
        SelectionItem::new("全局服务", "由 Center 持续托管并在后台运行", 1),
        SelectionItem::new("临时服务", "只在当前终端会话内运行", 2),
    ]);
    let backend = TestBackend::new(24, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| state.render(frame, frame.area(), "选择运行方式", "请选择服务运行方式"))
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(Cell::symbol)
        .collect::<String>()
        .replace(' ', "");

    assert!(text.contains("全局服务"));
    assert!(text.contains("临时服务"));
    assert!(text.contains("Enter确认"));
    assert!(text.contains("Esc取消"));
}
