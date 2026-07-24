//! 服务详情页的选择、页签切换、帮助层与转场导航。

use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::layout::Rect;

use super::{ActiveTab, App};
use crate::tui::transition::TransitionDirection;

impl App {
    /// 新按键进入前取消与其无关的日志清空确认。
    pub(super) fn prepare_key(&mut self, key: KeyCode) {
        if key != KeyCode::Char('C') {
            self.cancel_log_clear_confirmation();
        }
    }

    /// 推进短时页面转场。
    pub fn advance_transition(&mut self, elapsed: Duration) -> bool {
        self.transition.advance(elapsed)
    }

    /// 从右侧开始一次页面进入转场。
    pub(crate) const fn begin_entry_transition(&mut self) {
        self.transition.start(TransitionDirection::Forward);
    }

    /// 返回当前帧主内容应使用的转场区域。
    pub(crate) fn transition_area(&self, area: Rect) -> Rect {
        self.transition.content_area(area)
    }

    /// 返回快捷键帮助是否正在显示。
    pub const fn help_visible(&self) -> bool {
        self.help_visibility.visible()
    }

    /// 返回页面转场是否尚未结束。
    pub const fn transition_active(&self) -> bool {
        self.transition.active()
    }

    /// 选择下一个任务并在末尾回到开头。
    pub(super) fn select_next(&mut self) {
        self.cancel_log_clear_confirmation();
        if !self.snapshot.tasks.is_empty() {
            self.selected = (self.selected + 1) % self.snapshot.tasks.len();
            self.horizontal_scroll.reset_position();
        }
    }

    /// 选择上一个任务并在开头回到末尾。
    pub(super) fn select_previous(&mut self) {
        self.cancel_log_clear_confirmation();
        if !self.snapshot.tasks.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.snapshot.tasks.len() - 1);
            self.horizontal_scroll.reset_position();
        }
    }

    /// 切换页签并让新页面从文本起点开始显示。
    pub(super) fn switch_tab(&mut self, tab: ActiveTab, direction: TransitionDirection) {
        self.cancel_log_clear_confirmation();
        if self.active_tab == tab {
            return;
        }
        self.active_tab = tab;
        self.horizontal_scroll.reset_position();
        self.transition.start(direction);
    }

    /// 循环切换相邻页签并匹配进入方向。
    pub(super) fn switch_adjacent_tab(&mut self, forward: bool) {
        if forward {
            self.switch_tab(self.active_tab.next(), TransitionDirection::Forward);
        } else {
            self.switch_tab(self.active_tab.previous(), TransitionDirection::Backward);
        }
    }

    /// 依据目标页签相对位置选择转场方向。
    pub(super) fn switch_to_tab(&mut self, tab: ActiveTab) {
        let direction = if tab.index() >= self.active_tab.index() {
            TransitionDirection::Forward
        } else {
            TransitionDirection::Backward
        };
        self.switch_tab(tab, direction);
    }

    /// 在当前页面最长的相关文本范围内移动水平视口。
    pub(super) fn scroll_horizontal(&mut self, forward: bool) {
        let maximum = crate::tui::app_horizontal::page_text_maximum(self, false).saturating_sub(1);
        self.horizontal_scroll.scroll_manual(forward, maximum);
    }

    /// 切换全局自动滚动并让新模式从文本起点开始。
    pub(super) fn toggle_auto_scroll(&mut self) {
        self.horizontal_scroll.toggle_auto();
    }

    /// 取消尚未二次确认的日志清空操作和对应反馈。
    pub(super) fn cancel_log_clear_confirmation(&mut self) {
        if self.log_clear_confirmation.take().is_some() {
            self.feedback = None;
        }
    }

    /// 帮助浮层打开时仅处理关闭键。
    pub(super) fn close_help_with(&mut self, key: KeyCode) -> bool {
        if matches!(key, KeyCode::Char('?' | 'q') | KeyCode::Esc) {
            self.help_visibility.hide();
            true
        } else {
            false
        }
    }
}
