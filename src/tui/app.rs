use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use crate::core::TaskId;
use crate::protocol::{ProjectSnapshot, ServiceActionDto, TaskView};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;

use super::text_view;
use super::ui;

mod logs;

/// 单次日志翻页移动的逻辑行数。
const LOG_PAGE_LINES: usize = 20;
/// 单次鼠标滚轮移动的日志逻辑行数。
const LOG_WHEEL_LINES: usize = 3;

/// 日志正文是否只保留匹配搜索词的行。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LogFilterMode {
    /// 展示全部日志行。
    #[default]
    All,
    /// 只展示匹配行。
    Matches,
}

impl LogFilterMode {
    /// 返回当前是否启用匹配行过滤。
    const fn enabled(self) -> bool {
        matches!(self, Self::Matches)
    }

    /// 切换全部行与匹配行模式。
    const fn toggle(&mut self) {
        *self = match self {
            Self::All => Self::Matches,
            Self::Matches => Self::All,
        };
    }
}

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
    log_query: String,
    log_search_input: Option<String>,
    log_filter_mode: LogFilterMode,
    log_match_indices: BTreeMap<TaskId, usize>,
    log_clear_confirmation: Option<TaskId>,
    pending_log_clear: Option<TaskId>,
    horizontal_scroll: text_view::HorizontalScroll,
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
            log_query: String::new(),
            log_search_input: None,
            log_filter_mode: LogFilterMode::default(),
            log_match_indices: BTreeMap::new(),
            log_clear_confirmation: None,
            pending_log_clear: None,
            horizontal_scroll: text_view::HorizontalScroll::default(),
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
        if self.log_search_input.is_some() {
            return self.handle_log_search_input(key);
        }
        let previous = (
            self.selected,
            self.active_tab,
            self.should_quit,
            self.pending_action,
            self.current_log_scroll(),
            self.horizontal_scroll,
            self.log_query.clone(),
            self.log_search_input.clone(),
            self.log_filter_mode,
            self.log_clear_confirmation.clone(),
            self.pending_log_clear.clone(),
            self.current_log_match_index(),
        );
        if key != KeyCode::Char('C') {
            self.cancel_log_clear_confirmation();
        }
        match key {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Tab => self.switch_tab(self.active_tab.next()),
            KeyCode::BackTab => self.switch_tab(self.active_tab.previous()),
            KeyCode::Left => self.scroll_horizontal(false),
            KeyCode::Right => self.scroll_horizontal(true),
            KeyCode::F(3) => self.toggle_auto_scroll(),
            KeyCode::Char('1') => self.switch_tab(ActiveTab::Tasks),
            KeyCode::Char('2') => self.switch_tab(ActiveTab::Dependencies),
            KeyCode::Char('3') => self.switch_tab(ActiveTab::Logs),
            KeyCode::PageUp if self.active_tab == ActiveTab::Logs => self.scroll_log_up(page_lines),
            KeyCode::PageDown if self.active_tab == ActiveTab::Logs => {
                self.scroll_log_down(page_lines);
            }
            KeyCode::Home if self.active_tab == ActiveTab::Logs => self.scroll_log_to_start(),
            KeyCode::End if self.active_tab == ActiveTab::Logs => self.scroll_log_to_end(),
            KeyCode::Char('/') if self.active_tab == ActiveTab::Logs => {
                self.log_search_input = Some(self.log_query.clone());
            }
            KeyCode::Char('f') if self.active_tab == ActiveTab::Logs => self.toggle_log_filter(),
            KeyCode::Char('n') if self.active_tab == ActiveTab::Logs => self.select_log_match(true),
            KeyCode::Char('N') if self.active_tab == ActiveTab::Logs => {
                self.select_log_match(false);
            }
            KeyCode::Char('C') if self.active_tab == ActiveTab::Logs => {
                if let Some(task_id) = self.selected_task().map(|task| task.task_id.clone()) {
                    if self.log_clear_confirmation.as_ref() == Some(&task_id) {
                        self.log_clear_confirmation = None;
                        self.pending_log_clear = Some(task_id);
                    } else {
                        self.feedback =
                            Some(format!("再次按 C 确认清空 Task `{task_id}` 的全部日志"));
                        self.log_clear_confirmation = Some(task_id);
                    }
                }
            }
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
                self.horizontal_scroll,
                self.log_query.clone(),
                self.log_search_input.clone(),
                self.log_filter_mode,
                self.log_clear_confirmation.clone(),
                self.pending_log_clear.clone(),
                self.current_log_match_index(),
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
        let confirmation_cancelled = self.log_clear_confirmation.is_some();
        self.cancel_log_clear_confirmation();
        let previous = (
            self.selected,
            self.current_log_scroll(),
            self.horizontal_scroll,
        );
        match (self.active_tab, mouse.kind) {
            (ActiveTab::Logs, MouseEventKind::ScrollUp) => {
                self.scroll_log_up(LOG_WHEEL_LINES);
            }
            (ActiveTab::Logs, MouseEventKind::ScrollDown) => {
                self.scroll_log_down(LOG_WHEEL_LINES);
            }
            (_, MouseEventKind::ScrollUp) => self.select_previous(),
            (_, MouseEventKind::ScrollDown) => self.select_next(),
            (_, MouseEventKind::ScrollLeft) => self.scroll_horizontal(false),
            (_, MouseEventKind::ScrollRight) => self.scroll_horizontal(true),
            _ => {}
        }
        confirmation_cancelled
            || previous
                != (
                    self.selected,
                    self.current_log_scroll(),
                    self.horizontal_scroll,
                )
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

    /// 返回当前页面选中文本的水平字符偏移。
    pub const fn horizontal_offset(&self) -> usize {
        self.horizontal_scroll.manual_offset()
    }

    /// 返回折叠文本全局自动滚动是否开启。
    pub const fn auto_scroll_enabled(&self) -> bool {
        self.horizontal_scroll.auto_enabled()
    }

    /// 返回一段文本应使用的水平偏移；自动模式覆盖所有文本。
    pub(crate) const fn text_offset(&self, selected: bool) -> usize {
        self.horizontal_scroll.offset(selected)
    }

    /// 返回非选中界面文本仅在自动模式下使用的水平偏移。
    pub(crate) const fn automatic_text_offset(&self) -> usize {
        self.horizontal_scroll.automatic_offset()
    }

    /// 推进一次全局折叠文本自动滚动，并在到达末尾后回到起点。
    pub fn advance_auto_scroll(&mut self, elapsed: Duration) -> bool {
        let maximum = super::app_horizontal::page_text_maximum(self, true).saturating_sub(1);
        self.horizontal_scroll.advance(elapsed, maximum)
    }

    /// 返回当前高亮文本是否处于手动滚动冻结期。
    pub const fn manual_scroll_frozen(&self) -> bool {
        self.horizontal_scroll.manual_frozen()
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
        self.cancel_log_clear_confirmation();
        if !self.snapshot.tasks.is_empty() {
            self.selected = (self.selected + 1) % self.snapshot.tasks.len();
            self.horizontal_scroll.reset_position();
        }
    }

    /// 选择上一个任务并在开头回到末尾。
    fn select_previous(&mut self) {
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
    fn switch_tab(&mut self, tab: ActiveTab) {
        self.cancel_log_clear_confirmation();
        self.active_tab = tab;
        self.horizontal_scroll.reset_position();
    }

    /// 在当前页面最长的相关文本范围内移动水平视口。
    fn scroll_horizontal(&mut self, forward: bool) {
        let maximum = super::app_horizontal::page_text_maximum(self, false).saturating_sub(1);
        self.horizontal_scroll.scroll_manual(forward, maximum);
    }

    /// 切换全局自动滚动并让新模式从文本起点开始。
    fn toggle_auto_scroll(&mut self) {
        self.horizontal_scroll.toggle_auto();
    }

    /// 取消尚未二次确认的日志清空操作和对应反馈。
    fn cancel_log_clear_confirmation(&mut self) {
        if self.log_clear_confirmation.take().is_some() {
            self.feedback = None;
        }
    }
}

/// 根据环境变量判断是否启用低能力终端兼容模式。
fn terminal_plain_mode() -> bool {
    std::env::var_os("PROCORA_TUI_PLAIN").is_some()
        || std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").is_ok_and(|term| term.eq_ignore_ascii_case("dumb"))
}
