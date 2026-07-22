use std::time::Duration;

use crate::protocol::{ResourceUsageDto, ServiceViewDto};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;

use super::{
    overview_collection::{self, OverviewSort},
    overview_ui, text_view,
    ui_environment::terminal_plain_mode,
};

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
    /// 打开新建托管服务向导。
    CreateService,
}

/// 全局中心服务总览的交互状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverviewApp {
    services: Vec<ServiceViewDto>,
    visible_services: Vec<ServiceViewDto>,
    selected: usize,
    exit: Option<OverviewExit>,
    pending_action: Option<(String, OverviewAction)>,
    remove_confirmation: Option<String>,
    feedback: Option<String>,
    control_allowed: bool,
    plain_mode: bool,
    filter_query: String,
    filter_input: Option<String>,
    filter_before_input: String,
    sort: OverviewSort,
    sort_descending: bool,
    horizontal_scroll: text_view::HorizontalScroll,
}

impl OverviewApp {
    /// 根据服务摘要列表创建总览页面。
    pub fn new(services: Vec<ServiceViewDto>) -> Self {
        let mut app = Self {
            services,
            visible_services: Vec::new(),
            selected: 0,
            exit: None,
            pending_action: None,
            remove_confirmation: None,
            feedback: None,
            control_allowed: false,
            plain_mode: terminal_plain_mode(),
            filter_query: String::new(),
            filter_input: None,
            filter_before_input: String::new(),
            sort: OverviewSort::default(),
            sort_descending: false,
            horizontal_scroll: text_view::HorizontalScroll::default(),
        };
        app.rebuild_visible(None);
        app
    }

    /// 将总览状态绘制到终端帧。
    pub fn render(&self, frame: &mut Frame<'_>) {
        overview_ui::render(frame, self);
    }

    /// 处理一次不带额外语义的按键。
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        if self.filter_input.is_some() {
            return self.handle_filter_input(key);
        }
        let previous = self.clone();
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
            KeyCode::Char('n') if self.control_allowed => {
                self.exit = Some(OverviewExit::CreateService);
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Left => self.scroll_horizontal(false),
            KeyCode::Right => self.scroll_horizontal(true),
            KeyCode::F(3) => self.horizontal_scroll.toggle_auto(),
            KeyCode::Char('/') => self.begin_filter_input(),
            KeyCode::Char('o') => self.next_sort(),
            KeyCode::Char('O') => self.sort_descending = !self.sort_descending,
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
        if matches!(key, KeyCode::Char('o' | 'O')) {
            let selected_name = self.selected_service().map(|service| service.name.clone());
            self.rebuild_visible(selected_name.as_deref());
            self.horizontal_scroll.reset_position();
        }
        *self != previous
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
    pub fn replace_services(&mut self, services: Vec<ServiceViewDto>) -> bool {
        if self.services == services {
            return false;
        }
        let selected_name = self.selected_service().map(|service| service.name.clone());
        self.services = services;
        self.rebuild_visible(selected_name.as_deref());
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
            .visible_services
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

    /// 返回经过当前筛选和排序的可见服务列表。
    pub fn visible_services(&self) -> &[ServiceViewDto] {
        &self.visible_services
    }

    /// 返回筛选前的服务总数。
    pub const fn all_service_count(&self) -> usize {
        self.services.len()
    }

    /// 返回当前可见 Service 的聚合资源占用。
    pub fn visible_resources(&self) -> Option<ResourceUsageDto> {
        overview_collection::aggregate_resources(&self.visible_services)
    }

    /// 返回当前服务选择索引。
    pub const fn selected_index(&self) -> usize {
        self.selected
    }

    /// 返回当前选中服务。
    pub fn selected_service(&self) -> Option<&ServiceViewDto> {
        self.visible_services.get(self.selected)
    }

    /// 返回最近一次操作反馈。
    pub fn feedback(&self) -> Option<&str> {
        self.feedback.as_deref()
    }

    /// 返回已经应用的筛选文本。
    pub fn filter_query(&self) -> &str {
        &self.filter_query
    }

    /// 返回正在编辑的筛选文本。
    pub fn filter_input(&self) -> Option<&str> {
        self.filter_input.as_deref()
    }

    /// 返回当前排序字段。
    pub const fn sort(&self) -> OverviewSort {
        self.sort
    }

    /// 返回当前是否倒序排列。
    pub const fn sort_descending(&self) -> bool {
        self.sort_descending
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
        if !self.visible_services.is_empty() {
            self.selected = (self.selected + 1) % self.visible_services.len();
            self.horizontal_scroll.reset_position();
        }
    }

    /// 选择上一个服务并在开头回到末尾。
    fn select_previous(&mut self) {
        if !self.visible_services.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.visible_services.len() - 1);
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

    /// 开始编辑筛选文本并记录取消时要恢复的值。
    fn begin_filter_input(&mut self) {
        self.filter_before_input.clone_from(&self.filter_query);
        self.filter_input = Some(self.filter_query.clone());
    }

    /// 处理筛选输入并实时重建可见列表。
    fn handle_filter_input(&mut self, key: KeyCode) -> bool {
        let previous = self.clone();
        match key {
            KeyCode::Esc => {
                self.filter_query.clone_from(&self.filter_before_input);
                self.filter_input = None;
            }
            KeyCode::Enter => self.filter_input = None,
            KeyCode::Backspace => {
                if let Some(input) = self.filter_input.as_mut() {
                    input.pop();
                    self.filter_query.clone_from(input);
                }
            }
            KeyCode::Char(character) => {
                if let Some(input) = self.filter_input.as_mut() {
                    input.push(character);
                    self.filter_query.clone_from(input);
                }
            }
            _ => {}
        }
        self.rebuild_visible(None);
        self.horizontal_scroll.reset_position();
        *self != previous
    }

    /// 循环排序字段并采用该字段的默认方向。
    fn next_sort(&mut self) {
        self.sort = self.sort.next();
        self.sort_descending = self.sort.default_descending();
    }

    /// 按当前筛选和排序重新生成可见服务，并恢复稳定选择。
    fn rebuild_visible(&mut self, selected_name: Option<&str>) {
        self.visible_services = overview_collection::visible_services(
            &self.services,
            &self.filter_query,
            self.sort,
            self.sort_descending,
        );
        self.selected = selected_name
            .and_then(|name| {
                self.visible_services
                    .iter()
                    .position(|service| service.name == name)
            })
            .unwrap_or_else(|| {
                self.selected
                    .min(self.visible_services.len().saturating_sub(1))
            });
    }
}
