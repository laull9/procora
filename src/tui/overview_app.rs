use std::time::Duration;

use crate::protocol::ServiceViewDto;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;

use super::{app::terminal_plain_mode, overview_ui, text_view};

/// 总览页支持的服务管理动作。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverviewAction {
    /// 启动服务。
    Start,
    /// 停止服务。
    Stop,
    /// 重启服务。
    Restart,
    /// 移除服务注册。
    Remove,
}

/// 总览页退出后应执行的导航动作。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverviewExit {
    /// 退出 Procora。
    Quit,
    /// 打开指定服务的详情页。
    OpenService(String),
}

/// 全局中心服务总览的交互状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverviewApp {
    services: Vec<ServiceViewDto>,
    selected: usize,
    exit: Option<OverviewExit>,
    pending_action: Option<(String, OverviewAction)>,
    remove_confirmation: Option<String>,
    feedback: Option<String>,
    control_allowed: bool,
    plain_mode: bool,
    horizontal_scroll: text_view::HorizontalScroll,
}

impl OverviewApp {
    /// 根据服务摘要列表创建总览页面。
    pub fn new(mut services: Vec<ServiceViewDto>) -> Self {
        services.sort_by(|left, right| left.name.cmp(&right.name));
        Self {
            services,
            selected: 0,
            exit: None,
            pending_action: None,
            remove_confirmation: None,
            feedback: None,
            control_allowed: false,
            plain_mode: terminal_plain_mode(),
            horizontal_scroll: text_view::HorizontalScroll::default(),
        }
    }

    /// 将总览状态绘制到终端帧。
    pub fn render(&self, frame: &mut Frame<'_>) {
        overview_ui::render(frame, self);
    }

    /// 处理一次不带额外语义的按键。
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        let previous = (
            self.selected,
            self.exit.clone(),
            self.pending_action.clone(),
            self.remove_confirmation.clone(),
            self.horizontal_scroll,
        );
        if key != KeyCode::Char('d') {
            self.cancel_remove_confirmation();
        }
        match key {
            KeyCode::Char('q') | KeyCode::Esc => self.exit = Some(OverviewExit::Quit),
            KeyCode::Enter => {
                self.exit = self
                    .selected_service()
                    .map(|service| OverviewExit::OpenService(service.name.clone()));
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Left => self.scroll_horizontal(false),
            KeyCode::Right => self.scroll_horizontal(true),
            KeyCode::F(3) => self.horizontal_scroll.toggle_auto(),
            KeyCode::Char('s') if self.control_allowed => {
                self.queue_action(OverviewAction::Start);
            }
            KeyCode::Char('x') if self.control_allowed => {
                self.queue_action(OverviewAction::Stop);
            }
            KeyCode::Char('r') if self.control_allowed => {
                self.queue_action(OverviewAction::Restart);
            }
            KeyCode::Char('d') if self.control_allowed => self.confirm_remove(),
            _ => {}
        }
        previous
            != (
                self.selected,
                self.exit.clone(),
                self.pending_action.clone(),
                self.remove_confirmation.clone(),
                self.horizontal_scroll,
            )
    }

    /// 处理带修饰键的按键，并统一支持 Ctrl-C 退出。
    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let changed = self.exit.as_ref() != Some(&OverviewExit::Quit);
            self.exit = Some(OverviewExit::Quit);
            changed
        } else {
            self.handle_key(key.code)
        }
    }

    /// 处理鼠标滚轮选择和水平滚动。
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        let confirmation_cancelled = self.remove_confirmation.is_some();
        let previous = (self.selected, self.horizontal_scroll);
        self.cancel_remove_confirmation();
        match mouse.kind {
            MouseEventKind::ScrollUp => self.select_previous(),
            MouseEventKind::ScrollDown => self.select_next(),
            MouseEventKind::ScrollLeft => self.scroll_horizontal(false),
            MouseEventKind::ScrollRight => self.scroll_horizontal(true),
            _ => {}
        }
        confirmation_cancelled || previous != (self.selected, self.horizontal_scroll)
    }

    /// 替换服务列表，同时尽量保持稳定名称对应的选择。
    pub fn replace_services(&mut self, mut services: Vec<ServiceViewDto>) -> bool {
        services.sort_by(|left, right| left.name.cmp(&right.name));
        if self.services == services {
            return false;
        }
        let selected_name = self.selected_service().map(|service| service.name.clone());
        self.services = services;
        self.selected = selected_name
            .and_then(|name| {
                self.services
                    .iter()
                    .position(|service| service.name == name)
            })
            .unwrap_or_else(|| self.selected.min(self.services.len().saturating_sub(1)));
        true
    }

    /// 取出一次待执行的导航结果。
    pub fn take_exit(&mut self) -> Option<OverviewExit> {
        self.exit.take()
    }

    /// 取出一次待执行的服务管理动作。
    pub fn take_pending_action(&mut self) -> Option<(String, OverviewAction)> {
        self.pending_action.take()
    }

    /// 更新最近一次操作反馈。
    pub fn set_feedback(&mut self, feedback: impl Into<String>) -> bool {
        let feedback = feedback.into();
        if self.feedback.as_deref() == Some(feedback.as_str()) {
            return false;
        }
        self.feedback = Some(feedback);
        true
    }

    /// 推进折叠文本自动横移。
    pub fn advance_auto_scroll(&mut self, elapsed: Duration) -> bool {
        let maximum = self
            .services
            .iter()
            .map(|service| {
                service
                    .name
                    .chars()
                    .count()
                    .max(service.root.to_string_lossy().chars().count())
                    .max(service.config_path.to_string_lossy().chars().count())
                    .max(
                        service
                            .message
                            .as_deref()
                            .map_or(0, |message| message.chars().count()),
                    )
            })
            .max()
            .unwrap_or(0)
            .saturating_sub(1);
        self.horizontal_scroll.advance(elapsed, maximum)
    }

    /// 设置当前会话是否允许控制服务。
    pub const fn set_control_allowed(&mut self, allowed: bool) {
        self.control_allowed = allowed;
    }

    /// 设置纯文本兼容显示。
    pub const fn set_plain_mode(&mut self, plain: bool) {
        self.plain_mode = plain;
    }

    /// 返回服务列表。
    pub fn services(&self) -> &[ServiceViewDto] {
        &self.services
    }

    /// 返回当前服务选择索引。
    pub const fn selected_index(&self) -> usize {
        self.selected
    }

    /// 返回当前选中服务。
    pub fn selected_service(&self) -> Option<&ServiceViewDto> {
        self.services.get(self.selected)
    }

    /// 按稳定名称恢复服务选择，不存在时保持当前选择。
    pub(crate) fn select_service_named(&mut self, name: &str) {
        if let Some(index) = self
            .services
            .iter()
            .position(|service| service.name == name)
        {
            self.selected = index;
        }
    }

    /// 返回最近一次操作反馈。
    pub fn feedback(&self) -> Option<&str> {
        self.feedback.as_deref()
    }

    /// 返回是否允许控制服务。
    pub const fn control_allowed(&self) -> bool {
        self.control_allowed
    }

    /// 返回是否使用纯文本兼容显示。
    pub const fn plain_mode(&self) -> bool {
        self.plain_mode
    }

    /// 返回一段折叠文本当前应使用的偏移。
    pub(crate) const fn text_offset(&self, selected: bool) -> usize {
        self.horizontal_scroll.offset(selected)
    }

    /// 返回非选中文本的自动偏移。
    pub(crate) const fn automatic_text_offset(&self) -> usize {
        self.horizontal_scroll.automatic_offset()
    }

    /// 返回自动横移是否启用。
    pub const fn auto_scroll_enabled(&self) -> bool {
        self.horizontal_scroll.auto_enabled()
    }

    /// 返回当前选中项是否处于手动滚动冻结期。
    pub const fn manual_scroll_frozen(&self) -> bool {
        self.horizontal_scroll.manual_frozen()
    }

    /// 选择下一个服务并在末尾回到开头。
    fn select_next(&mut self) {
        if !self.services.is_empty() {
            self.selected = (self.selected + 1) % self.services.len();
            self.horizontal_scroll.reset_position();
        }
    }

    /// 选择上一个服务并在开头回到末尾。
    fn select_previous(&mut self) {
        if !self.services.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.services.len() - 1);
            self.horizontal_scroll.reset_position();
        }
    }

    /// 水平移动当前高亮文本。
    fn scroll_horizontal(&mut self, forward: bool) {
        let maximum = self
            .selected_service()
            .map_or(0, |service| {
                service
                    .name
                    .chars()
                    .count()
                    .max(service.root.to_string_lossy().chars().count())
                    .max(service.config_path.to_string_lossy().chars().count())
                    .max(
                        service
                            .message
                            .as_deref()
                            .map_or(0, |message| message.chars().count()),
                    )
            })
            .saturating_sub(1);
        self.horizontal_scroll.scroll_manual(forward, maximum);
    }

    /// 为当前服务排队一次管理动作。
    fn queue_action(&mut self, action: OverviewAction) {
        self.pending_action = self
            .selected_service()
            .map(|service| (service.name.clone(), action));
    }

    /// 对移除服务执行二次按键确认。
    fn confirm_remove(&mut self) {
        let Some(name) = self.selected_service().map(|service| service.name.clone()) else {
            return;
        };
        if self.remove_confirmation.as_deref() == Some(name.as_str()) {
            self.remove_confirmation = None;
            self.pending_action = Some((name, OverviewAction::Remove));
        } else {
            self.feedback = Some(format!("再次按 d 确认移除服务 `{name}`（不会删除目录）"));
            self.remove_confirmation = Some(name);
        }
    }

    /// 取消尚未确认的移除动作。
    fn cancel_remove_confirmation(&mut self) {
        if self.remove_confirmation.take().is_some() {
            self.feedback = None;
        }
    }
}
