use std::collections::{BTreeMap, BTreeSet};

use crate::core::TaskId;
use crate::protocol::{ProjectSnapshot, ServiceActionDto, TaskView};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;

use super::ui;

/// 单次日志翻页移动的逻辑行数。
const LOG_PAGE_LINES: usize = 20;
/// 单次鼠标滚轮移动的日志逻辑行数。
const LOG_WHEEL_LINES: usize = 3;

/// TUI 主区域当前显示的页面。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ActiveTab {
    /// 任务列表与当前任务详情。
    #[default]
    Tasks,
    /// 任务依赖边的只读视图。
    Dependencies,
    /// 当前任务的日志观察视图。
    Logs,
}

impl ActiveTab {
    /// 返回页签索引。
    pub const fn index(self) -> usize {
        match self {
            Self::Tasks => 0,
            Self::Dependencies => 1,
            Self::Logs => 2,
        }
    }

    /// 返回下一个循环页签。
    const fn next(self) -> Self {
        match self {
            Self::Tasks => Self::Dependencies,
            Self::Dependencies => Self::Logs,
            Self::Logs => Self::Tasks,
        }
    }

    /// 返回上一个循环页签。
    const fn previous(self) -> Self {
        match self {
            Self::Tasks => Self::Logs,
            Self::Dependencies => Self::Tasks,
            Self::Logs => Self::Dependencies,
        }
    }
}

/// 日志页应展示的终端键位提示类别。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyHintPlatform {
    /// 使用 PageUp、PageDown、Home 和 End 名称。
    Standard,
    /// 使用 macOS 键盘上的 Fn 组合键名称。
    MacOs,
}

/// TUI 持有的协议快照与本地交互状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct App {
    snapshot: ProjectSnapshot,
    selected: usize,
    active_tab: ActiveTab,
    should_quit: bool,
    pending_action: Option<ServiceActionDto>,
    feedback: Option<String>,
    control_allowed: bool,
    plain_mode: bool,
    key_hint_platform: KeyHintPlatform,
    log_buffers: BTreeMap<TaskId, Vec<u8>>,
    log_gaps: BTreeSet<TaskId>,
    log_scrolls: BTreeMap<TaskId, usize>,
}

impl App {
    /// 根据一致性项目快照创建观察界面。
    pub fn new(snapshot: ProjectSnapshot) -> Self {
        Self {
            snapshot,
            selected: 0,
            active_tab: ActiveTab::default(),
            should_quit: false,
            pending_action: None,
            feedback: None,
            control_allowed: false,
            plain_mode: terminal_plain_mode(),
            key_hint_platform: if cfg!(target_os = "macos") {
                KeyHintPlatform::MacOs
            } else {
                KeyHintPlatform::Standard
            },
            log_buffers: BTreeMap::new(),
            log_gaps: BTreeSet::new(),
            log_scrolls: BTreeMap::new(),
        }
    }

    /// 将当前应用状态绘制到终端帧。
    pub fn render(&self, frame: &mut Frame<'_>) {
        ui::render(frame, self);
    }

    /// 处理一次已经确认的按键输入。
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        self.handle_key_with_log_page(key, LOG_PAGE_LINES)
    }

    /// 使用当前终端的实际日志页高处理一次按键输入。
    pub(crate) fn handle_key_with_log_page(&mut self, key: KeyCode, page_lines: usize) -> bool {
        let previous = (
            self.selected,
            self.active_tab,
            self.should_quit,
            self.pending_action,
            self.current_log_scroll(),
        );
        match key {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Tab | KeyCode::Right => self.active_tab = self.active_tab.next(),
            KeyCode::BackTab | KeyCode::Left => self.active_tab = self.active_tab.previous(),
            KeyCode::Char('1') => self.active_tab = ActiveTab::Tasks,
            KeyCode::Char('2') => self.active_tab = ActiveTab::Dependencies,
            KeyCode::Char('3') => self.active_tab = ActiveTab::Logs,
            KeyCode::PageUp if self.active_tab == ActiveTab::Logs => self.scroll_log_up(page_lines),
            KeyCode::PageDown if self.active_tab == ActiveTab::Logs => {
                self.scroll_log_down(page_lines);
            }
            KeyCode::Home if self.active_tab == ActiveTab::Logs => self.scroll_log_to_start(),
            KeyCode::End if self.active_tab == ActiveTab::Logs => self.scroll_log_to_end(),
            KeyCode::Char('s') if self.control_allowed => {
                self.pending_action = Some(ServiceActionDto::Start);
            }
            KeyCode::Char('x') if self.control_allowed => {
                self.pending_action = Some(ServiceActionDto::Stop);
            }
            KeyCode::Char('r') if self.control_allowed => {
                self.pending_action = Some(ServiceActionDto::Restart);
            }
            _ => {}
        }
        previous
            != (
                self.selected,
                self.active_tab,
                self.should_quit,
                self.pending_action,
                self.current_log_scroll(),
            )
    }

    /// 处理带修饰键的终端按键，并把 Ctrl-C 统一解释为正常退出请求。
    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        self.handle_key_event_with_log_page(key, LOG_PAGE_LINES)
    }

    /// 使用实际日志页高处理带修饰键的终端按键。
    pub(crate) fn handle_key_event_with_log_page(
        &mut self,
        key: KeyEvent,
        page_lines: usize,
    ) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let changed = !self.should_quit;
            self.should_quit = true;
            changed
        } else {
            self.handle_key_with_log_page(key.code, page_lines)
        }
    }

    /// 处理鼠标滚轮；日志页滚动正文，其他页面保持 Task 选择行为。
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        let previous = (self.selected, self.current_log_scroll());
        match (self.active_tab, mouse.kind) {
            (ActiveTab::Logs, MouseEventKind::ScrollUp) => {
                self.scroll_log_up(LOG_WHEEL_LINES);
            }
            (ActiveTab::Logs, MouseEventKind::ScrollDown) => {
                self.scroll_log_down(LOG_WHEEL_LINES);
            }
            (_, MouseEventKind::ScrollUp) => self.select_previous(),
            (_, MouseEventKind::ScrollDown) => self.select_next(),
            _ => {}
        }
        previous != (self.selected, self.current_log_scroll())
    }

    /// 使用服务器新快照替换内容并保持合理的任务选择位置。
    pub fn replace_snapshot(&mut self, snapshot: ProjectSnapshot) -> bool {
        if self.snapshot == snapshot {
            return false;
        }
        self.snapshot = snapshot;
        if self.selected >= self.snapshot.tasks.len() {
            self.selected = self.snapshot.tasks.len().saturating_sub(1);
        }
        true
    }

    /// 取出一次等待执行的服务生命周期动作。
    pub fn take_pending_action(&mut self) -> Option<ServiceActionDto> {
        self.pending_action.take()
    }

    /// 更新供页脚展示的最近一次操作结果。
    pub fn set_feedback(&mut self, feedback: impl Into<String>) -> bool {
        let feedback = feedback.into();
        if self.feedback.as_deref() == Some(feedback.as_str()) {
            return false;
        }
        self.feedback = Some(feedback);
        true
    }

    /// 返回最近一次操作或连接反馈。
    pub fn feedback(&self) -> Option<&str> {
        self.feedback.as_deref()
    }

    /// 设置当前中心会话是否允许提交控制动作。
    pub const fn set_control_allowed(&mut self, allowed: bool) {
        self.control_allowed = allowed;
    }

    /// 返回当前中心会话是否允许提交控制动作。
    pub const fn control_allowed(&self) -> bool {
        self.control_allowed
    }

    /// 设置是否使用适合低能力终端的纯文本显示。
    pub const fn set_plain_mode(&mut self, plain: bool) {
        self.plain_mode = plain;
    }

    /// 返回当前是否使用纯文本显示。
    pub const fn plain_mode(&self) -> bool {
        self.plain_mode
    }

    /// 设置是否展示 macOS 的 Fn 组合键提示。
    pub const fn set_mac_key_hints(&mut self, enabled: bool) {
        self.key_hint_platform = if enabled {
            KeyHintPlatform::MacOs
        } else {
            KeyHintPlatform::Standard
        };
    }

    /// 返回是否展示 macOS 的 Fn 组合键提示。
    pub const fn mac_key_hints(&self) -> bool {
        matches!(self.key_hint_platform, KeyHintPlatform::MacOs)
    }

    /// 追加一批 Task 日志，并把内存展示限制在最后 64 KiB。
    pub fn append_log(&mut self, task_id: TaskId, bytes: &[u8], gap: bool) -> bool {
        const DISPLAY_LIMIT: usize = 64 * 1024;
        let gap_changed = gap && !self.log_gaps.contains(&task_id);
        if bytes.is_empty() && !gap_changed {
            return false;
        }
        let previous_lines = self.log_total_lines(&task_id);
        let has_gap = gap || self.has_log_gap(&task_id);
        let buffer = self.log_buffers.entry(task_id.clone()).or_default();
        buffer.extend_from_slice(bytes);
        if buffer.len() > DISPLAY_LIMIT {
            buffer.drain(..buffer.len() - DISPLAY_LIMIT);
        }
        let content_lines = display_line_count(buffer);
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

    /// 返回当前项目快照。
    pub const fn snapshot(&self) -> &ProjectSnapshot {
        &self.snapshot
    }

    /// 返回当前选中的任务索引。
    pub const fn selected_index(&self) -> usize {
        self.selected
    }

    /// 返回当前选中的任务，不存在任务时返回空值。
    pub fn selected_task(&self) -> Option<&TaskView> {
        self.snapshot.tasks.get(self.selected)
    }

    /// 返回当前活动页签。
    pub const fn active_tab(&self) -> ActiveTab {
        self.active_tab
    }

    /// 返回输入循环是否应该退出。
    pub const fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// 选择下一个任务并在末尾回到开头。
    fn select_next(&mut self) {
        if !self.snapshot.tasks.is_empty() {
            self.selected = (self.selected + 1) % self.snapshot.tasks.len();
        }
    }

    /// 选择上一个任务并在开头回到末尾。
    fn select_previous(&mut self) {
        if !self.snapshot.tasks.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.snapshot.tasks.len() - 1);
        }
    }

    /// 返回当前任务的日志滚动距离。
    fn current_log_scroll(&self) -> Option<usize> {
        self.selected_task()
            .map(|task| self.log_scroll_distance(&task.task_id))
    }

    /// 把当前日志向历史方向移动一页。
    fn scroll_log_up(&mut self, page_lines: usize) {
        let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) else {
            return;
        };
        let maximum = self.log_total_lines(&task_id);
        let distance = self.log_scrolls.entry(task_id).or_default();
        *distance = distance.saturating_add(page_lines.max(1)).min(maximum);
    }

    /// 把当前日志向尾部方向移动一页。
    fn scroll_log_down(&mut self, page_lines: usize) {
        let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) else {
            return;
        };
        let distance = self.log_scrolls.entry(task_id).or_default();
        *distance = distance.saturating_sub(page_lines.max(1));
    }

    /// 跳到当前日志的第一行。
    fn scroll_log_to_start(&mut self) {
        let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) else {
            return;
        };
        let maximum = self.log_total_lines(&task_id);
        self.log_scrolls.insert(task_id, maximum);
    }

    /// 回到当前日志尾部并恢复自动跟随。
    fn scroll_log_to_end(&mut self) {
        let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) else {
            return;
        };
        self.log_scrolls.insert(task_id, 0);
    }

    /// 返回日志正文和间隙提示合计占用的逻辑行数。
    fn log_total_lines(&self, task_id: &TaskId) -> usize {
        let content_lines = self
            .log_buffers
            .get(task_id)
            .map_or(0, |buffer| display_line_count(buffer));
        content_lines + usize::from(self.has_log_gap(task_id)) * 2
    }
}

/// 计算原始日志缓冲在不折行时占用的逻辑行数。
fn display_line_count(buffer: &[u8]) -> usize {
    if buffer.is_empty() {
        0
    } else {
        buffer.split(|byte| *byte == b'\n').count() - usize::from(buffer.last() == Some(&b'\n'))
    }
}

/// 根据环境变量判断是否启用低能力终端兼容模式。
fn terminal_plain_mode() -> bool {
    std::env::var_os("PROCORA_TUI_PLAIN").is_some()
        || std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").is_ok_and(|term| term.eq_ignore_ascii_case("dumb"))
}
