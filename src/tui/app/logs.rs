//! TUI 日志缓存、滚动、搜索、过滤与清空交互状态。

use crate::{core::TaskId, tui::log_view};
use crossterm::event::KeyCode;

use super::App;

impl App {
    /// 追加一批 Task 日志并保留本次 TUI 会话读取到的完整历史。
    pub fn append_log(&mut self, task_id: TaskId, bytes: &[u8], gap: bool) -> bool {
        let gap_changed = gap && !self.log_gaps.contains(&task_id);
        if bytes.is_empty() && !gap_changed {
            return false;
        }
        let previous_lines = self.log_total_lines(&task_id);
        let has_gap = gap || self.has_log_gap(&task_id);
        let buffer = self.log_buffers.entry(task_id.clone()).or_default();
        buffer.extend_from_slice(bytes);
        let content_lines =
            log_view::visible_lines(buffer, &self.log_query, self.log_filter_mode.enabled()).len();
        let current_lines = content_lines + usize::from(has_gap) * 2;
        if let Some(distance) = self.log_scrolls.get_mut(&task_id)
            && *distance > 0
        {
            let added_lines = current_lines.saturating_sub(previous_lines);
            *distance = distance.saturating_add(added_lines);
        }
        if gap {
            self.log_gaps.insert(task_id);
        }
        true
    }

    /// 返回指定 Task 当前缓存的有损 UTF-8 日志文本。
    pub fn log_text(&self, task_id: &TaskId) -> Option<String> {
        self.log_buffers
            .get(task_id)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    }

    /// 返回指定 Task 当前可见日志行的最大字符宽度。
    pub(crate) fn log_maximum_width(&self, task_id: &TaskId) -> usize {
        self.log_buffers.get(task_id).map_or(0, |bytes| {
            log_view::visible_lines(bytes, &self.log_query, self.log_filter_mode.enabled())
                .iter()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0)
        })
    }

    /// 返回指定 Task 按 ANSI 样式、搜索和过滤状态构造的日志文本。
    pub(crate) fn styled_log_text(&self, task_id: &TaskId) -> Option<ratatui::text::Text<'static>> {
        let active_match = self.active_log_match_line(task_id);
        self.log_buffers.get(task_id).map(|bytes| {
            log_view::styled_text(
                bytes,
                &self.log_query,
                self.log_filter_mode.enabled(),
                active_match,
                self.plain_mode,
            )
        })
    }

    /// 返回当前日志搜索输入；空值表示不在输入模式。
    pub fn log_search_input(&self) -> Option<&str> {
        self.log_search_input.as_deref()
    }

    /// 返回已经应用的日志搜索词。
    pub fn log_query(&self) -> &str {
        &self.log_query
    }

    /// 返回日志是否只显示匹配行。
    pub const fn log_filter_enabled(&self) -> bool {
        self.log_filter_mode.enabled()
    }

    /// 返回当前 Task 的搜索匹配位置与匹配总数。
    pub fn log_match_position(&self, task_id: &TaskId) -> Option<(usize, usize)> {
        let matches = self.log_match_lines(task_id);
        if matches.is_empty() {
            return None;
        }
        let index = self
            .log_match_indices
            .get(task_id)
            .copied()
            .unwrap_or(0)
            .min(matches.len() - 1);
        Some((index + 1, matches.len()))
    }

    /// 取出一次等待执行的持久日志清空请求。
    pub fn take_pending_log_clear(&mut self) -> Option<TaskId> {
        self.pending_log_clear.take()
    }

    /// 清空当前会话内指定 Task 的日志视图状态。
    pub fn clear_log(&mut self, task_id: &TaskId) -> bool {
        if self.log_clear_confirmation.as_ref() == Some(task_id) {
            self.log_clear_confirmation = None;
        }
        self.log_buffers.remove(task_id).is_some()
            || self.log_gaps.remove(task_id)
            || self.log_scrolls.remove(task_id).is_some()
            || self.log_match_indices.remove(task_id).is_some()
    }

    /// 返回指定 Task 是否曾跨越不可恢复的日志间隙。
    pub fn has_log_gap(&self, task_id: &TaskId) -> bool {
        self.log_gaps.contains(task_id)
    }

    /// 返回指定 Task 距离日志尾部的逻辑行数，零表示自动跟随。
    pub fn log_scroll_distance(&self, task_id: &TaskId) -> usize {
        self.log_scrolls.get(task_id).copied().unwrap_or(0)
    }

    /// 根据内容与可见高度计算 `Paragraph` 使用的顶部滚动行。
    pub fn log_scroll_top(&self, task_id: &TaskId, viewport_lines: usize) -> usize {
        let total_lines = self.log_total_lines(task_id);
        let maximum = total_lines.saturating_sub(viewport_lines.max(1));
        maximum.saturating_sub(self.log_scroll_distance(task_id).min(maximum))
    }

    /// 返回当前任务的日志滚动距离。
    pub(super) fn current_log_scroll(&self) -> Option<usize> {
        self.selected_task()
            .map(|task| self.log_scroll_distance(&task.task_id))
    }

    /// 返回当前任务的搜索匹配索引。
    pub(super) fn current_log_match_index(&self) -> Option<usize> {
        self.selected_task()
            .and_then(|task| self.log_match_indices.get(&task.task_id).copied())
    }

    /// 把当前日志向历史方向移动一页。
    pub(super) fn scroll_log_up(&mut self, page_lines: usize) {
        let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) else {
            return;
        };
        let maximum = self.log_total_lines(&task_id);
        let distance = self.log_scrolls.entry(task_id).or_default();
        *distance = distance.saturating_add(page_lines.max(1)).min(maximum);
    }

    /// 把当前日志向尾部方向移动一页。
    pub(super) fn scroll_log_down(&mut self, page_lines: usize) {
        let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) else {
            return;
        };
        let distance = self.log_scrolls.entry(task_id).or_default();
        *distance = distance.saturating_sub(page_lines.max(1));
    }

    /// 跳到当前日志的第一行。
    pub(super) fn scroll_log_to_start(&mut self) {
        let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) else {
            return;
        };
        let maximum = self.log_total_lines(&task_id);
        self.log_scrolls.insert(task_id, maximum);
    }

    /// 回到当前日志尾部并恢复自动跟随。
    pub(super) fn scroll_log_to_end(&mut self) {
        let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) else {
            return;
        };
        self.log_scrolls.insert(task_id, 0);
    }

    /// 处理日志搜索输入框中的一次按键。
    pub(super) fn handle_log_search_input(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Esc => self.log_search_input = None,
            KeyCode::Enter => {
                self.log_query = self.log_search_input.take().unwrap_or_default();
                self.log_match_indices.clear();
                if let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) {
                    self.focus_log_match(&task_id);
                }
            }
            KeyCode::Backspace => {
                if let Some(input) = &mut self.log_search_input {
                    input.pop();
                }
            }
            KeyCode::Char(character) => {
                if let Some(input) = &mut self.log_search_input {
                    input.push(character);
                }
            }
            _ => return false,
        }
        true
    }

    /// 切换仅显示匹配行，并重置到当前匹配位置。
    pub(super) fn toggle_log_filter(&mut self) {
        if self.log_query.is_empty() {
            return;
        }
        self.log_filter_mode.toggle();
        self.log_scrolls.clear();
        if let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) {
            self.focus_log_match(&task_id);
        }
    }

    /// 循环选择当前 Task 的下一条或上一条匹配行。
    pub(super) fn select_log_match(&mut self, forward: bool) {
        let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) else {
            return;
        };
        let count = self.log_match_lines(&task_id).len();
        if count == 0 {
            return;
        }
        let current = self.log_match_indices.entry(task_id.clone()).or_default();
        *current = if forward {
            (*current + 1) % count
        } else {
            current.checked_sub(1).unwrap_or(count - 1)
        };
        self.focus_log_match(&task_id);
    }

    /// 返回日志正文和间隙提示合计占用的逻辑行数。
    fn log_total_lines(&self, task_id: &TaskId) -> usize {
        let content_lines = self.log_buffers.get(task_id).map_or(0, |buffer| {
            log_view::visible_lines(buffer, &self.log_query, self.log_filter_mode.enabled()).len()
        });
        content_lines + usize::from(self.has_log_gap(task_id)) * 2
    }

    /// 把日志纵向视口移动到当前搜索匹配行。
    fn focus_log_match(&mut self, task_id: &TaskId) {
        let matches = self.log_match_lines(task_id);
        if matches.is_empty() {
            self.log_scrolls.insert(task_id.clone(), 0);
            return;
        }
        let index = self.log_match_indices.entry(task_id.clone()).or_default();
        *index = (*index).min(matches.len() - 1);
        let line = matches[*index] + usize::from(self.has_log_gap(task_id)) * 2;
        let distance = self.log_total_lines(task_id).saturating_sub(line + 1);
        self.log_scrolls.insert(task_id.clone(), distance);
    }

    /// 返回当前条件下指定 Task 的全部匹配行。
    fn log_match_lines(&self, task_id: &TaskId) -> Vec<usize> {
        self.log_buffers
            .get(task_id)
            .map_or_else(Vec::new, |bytes| {
                log_view::match_lines(bytes, &self.log_query, self.log_filter_mode.enabled())
            })
    }

    /// 返回当前选中匹配对应的可见行。
    fn active_log_match_line(&self, task_id: &TaskId) -> Option<usize> {
        let matches = self.log_match_lines(task_id);
        let index = self.log_match_indices.get(task_id).copied().unwrap_or(0);
        matches.get(index).copied()
    }
}
