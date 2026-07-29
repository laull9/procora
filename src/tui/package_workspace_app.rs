//! Procora 包工作台的纯交互状态与动作意图。

mod actions;

use std::{path::PathBuf, time::Duration};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;

use super::{help_ui::HelpVisibility, package_workspace_ui, text_view};

/// 包工作台的顶层视图。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageWorkspaceTab {
    /// 本地和托管目录中可直接使用的 `.pcpkg`。
    Packages,
    /// 已安装 Service 的 release 与恢复状态。
    Installed,
}

impl PackageWorkspaceTab {
    /// 返回用户可见标签。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Packages => "包文件",
            Self::Installed => "已安装",
        }
    }
}

/// 工作台中一个可读取或损坏的包文件。
#[derive(Clone, Debug)]
pub struct PackageWorkspaceEntry {
    /// 包文件路径。
    pub path: PathBuf,
    /// 可读取时的轻量包信息。
    pub info: Option<crate::package::PackageInfo>,
    /// 清单损坏或不兼容时的诊断。
    pub error: Option<String>,
}

/// 工作台退出后由 CLI 执行的动作。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PackageWorkspaceExit {
    /// 返回服务总览。
    Back,
    /// 重新扫描包文件和安装状态。
    Refresh,
    /// 打开路径选择器加入一个已有包。
    OpenPackage,
    /// 从当前上下文 Service 构建包。
    BuildPackage,
    /// 完整验证选中包。
    Verify(PathBuf),
    /// 安装选中包。
    Install(PathBuf),
    /// 临时运行选中包。
    Run(PathBuf),
    /// 解包选中包。
    Extract(PathBuf),
    /// 裸机部署选中包。
    Deploy(PathBuf),
    /// 推送选中包的命名导出项。
    PushExport {
        /// 要物化导出的包文件。
        package: PathBuf,
        /// 清单中的命名导出项。
        entry: String,
    },
    /// 永久删除选中的本地包文件。
    DeletePackage(PathBuf),
    /// 回滚到最近一个非活动 release。
    Rollback(String),
    /// 恢复中断的 pending 安装。
    Recover(String),
    /// 从 Center 解除并永久清理一个包安装。
    Purge(String),
    /// 从 Center 解除包安装但保留 release 与原始包。
    Uninstall(String),
}

/// 包工作台的可测试 UI 状态。
#[derive(Clone, Debug)]
pub struct PackageWorkspaceApp {
    packages: Vec<PackageWorkspaceEntry>,
    installed: Vec<crate::package::InstalledService>,
    selected_package: usize,
    selected_installed: usize,
    tab: PackageWorkspaceTab,
    context_source: Option<PathBuf>,
    control_allowed: bool,
    help_visibility: HelpVisibility,
    export_picker: Option<ExportPicker>,
    package_delete_confirmation: Option<PathBuf>,
    purge_confirmation: Option<String>,
    uninstall_confirmation: Option<String>,
    horizontal_scroll: text_view::HorizontalScroll,
    feedback: Option<String>,
    exit: Option<PackageWorkspaceExit>,
    plain_mode: bool,
}

/// 多个导出项的内联选择状态。
#[derive(Clone, Debug)]
struct ExportPicker {
    package: PathBuf,
    entries: Vec<String>,
    selected: usize,
}

impl PackageWorkspaceApp {
    /// 创建包文件与安装状态并列的工作台。
    pub fn new(
        packages: Vec<PackageWorkspaceEntry>,
        installed: Vec<crate::package::InstalledService>,
        context_source: Option<PathBuf>,
    ) -> Self {
        Self {
            packages,
            installed,
            selected_package: 0,
            selected_installed: 0,
            tab: PackageWorkspaceTab::Packages,
            context_source,
            control_allowed: false,
            help_visibility: HelpVisibility::default(),
            export_picker: None,
            package_delete_confirmation: None,
            purge_confirmation: None,
            uninstall_confirmation: None,
            horizontal_scroll: text_view::HorizontalScroll::default(),
            feedback: None,
            exit: None,
            plain_mode: super::ui_environment::terminal_plain_mode(),
        }
    }

    /// 绘制当前包工作台。
    pub fn render(&self, frame: &mut Frame<'_>) {
        package_workspace_ui::render(frame, self);
    }

    /// 处理带修饰键的工作台输入。
    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.exit = Some(PackageWorkspaceExit::Back);
            return true;
        }
        self.handle_key(key.code)
    }

    /// 处理一个不带额外语义的按键。
    pub fn handle_key(&mut self, key: KeyCode) -> bool {
        if self.help_visibility.visible() {
            if matches!(key, KeyCode::Char('?' | 'q') | KeyCode::Esc) {
                self.help_visibility.hide();
                return true;
            }
            return false;
        }
        if self.export_picker.is_some() {
            return self.handle_export_key(key);
        }
        let mut confirmation_cancelled = false;
        if key != KeyCode::Char('D') && self.purge_confirmation.is_some() {
            self.purge_confirmation = None;
            confirmation_cancelled = true;
        }
        if key != KeyCode::Char('U') && self.uninstall_confirmation.is_some() {
            self.uninstall_confirmation = None;
            confirmation_cancelled = true;
        }
        if !matches!(key, KeyCode::Delete | KeyCode::Char('X'))
            && self.package_delete_confirmation.is_some()
        {
            self.package_delete_confirmation = None;
            confirmation_cancelled = true;
        }
        if confirmation_cancelled {
            self.feedback = Some("已取消删除或解除确认".to_owned());
        }
        match key {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.exit = Some(PackageWorkspaceExit::Back);
            }
            KeyCode::Char('?') => self.help_visibility.show(),
            KeyCode::Tab | KeyCode::BackTab => self.switch_tab(),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Left => self.scroll_horizontal(false),
            KeyCode::Right => self.scroll_horizontal(true),
            KeyCode::F(3) => self.horizontal_scroll.toggle_auto(),
            KeyCode::Char('r') => self.exit = Some(PackageWorkspaceExit::Refresh),
            KeyCode::Char('o') => self.exit = Some(PackageWorkspaceExit::OpenPackage),
            KeyCode::Char('b') if self.control_allowed => {
                self.exit = Some(PackageWorkspaceExit::BuildPackage);
            }
            KeyCode::Char('v') => self.package_action(PackageWorkspaceExit::Verify),
            KeyCode::Char('i') if self.control_allowed => {
                self.package_action(PackageWorkspaceExit::Install);
            }
            KeyCode::Char('t') if self.control_allowed => {
                self.package_action(PackageWorkspaceExit::Run);
            }
            KeyCode::Char('x') => self.package_action(PackageWorkspaceExit::Extract),
            KeyCode::Char('d') if self.control_allowed => {
                self.package_action(PackageWorkspaceExit::Deploy);
            }
            KeyCode::Char('u') if self.control_allowed => self.begin_export(),
            KeyCode::Delete | KeyCode::Char('X') if self.control_allowed => {
                self.confirm_package_delete();
            }
            KeyCode::Char('R') if self.control_allowed => self.installed_action(
                |project| PackageWorkspaceExit::Rollback(project.to_owned()),
                "没有可回滚的已安装 Service",
            ),
            KeyCode::Char('c') if self.control_allowed => self.installed_action(
                |project| PackageWorkspaceExit::Recover(project.to_owned()),
                "没有可恢复的已安装 Service",
            ),
            KeyCode::Char('D') if self.control_allowed => self.confirm_purge(),
            KeyCode::Char('U') if self.control_allowed => self.confirm_uninstall(),
            _ => return false,
        }
        true
    }

    /// 处理鼠标滚轮选择和触控板横向移动。
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        let confirmation_cancelled = self.package_delete_confirmation.is_some()
            || self.purge_confirmation.is_some()
            || self.uninstall_confirmation.is_some();
        let previous = (
            self.selected_package,
            self.selected_installed,
            self.horizontal_scroll,
        );
        self.package_delete_confirmation = None;
        self.purge_confirmation = None;
        self.uninstall_confirmation = None;
        if confirmation_cancelled {
            self.feedback = Some("已取消删除或解除确认".to_owned());
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => self.select_previous(),
            MouseEventKind::ScrollDown => self.select_next(),
            MouseEventKind::ScrollLeft => self.scroll_horizontal(false),
            MouseEventKind::ScrollRight => self.scroll_horizontal(true),
            _ => {}
        }
        confirmation_cancelled
            || previous
                != (
                    self.selected_package,
                    self.selected_installed,
                    self.horizontal_scroll,
                )
    }

    /// 取出一次工作台导航或操作意图。
    pub fn take_exit(&mut self) -> Option<PackageWorkspaceExit> {
        self.exit.take()
    }

    /// 更新工作台操作结果。
    pub fn set_feedback(&mut self, feedback: impl Into<String>) {
        self.feedback = Some(feedback.into());
    }

    /// 用重新扫描的模型更新工作台并保持稳定选择。
    pub fn replace_data(
        &mut self,
        packages: Vec<PackageWorkspaceEntry>,
        installed: Vec<crate::package::InstalledService>,
    ) {
        let selected_path = self.selected_package().map(|entry| entry.path.clone());
        let selected_project = self
            .selected_installed()
            .map(|service| service.project.clone());
        self.packages = packages;
        self.installed = installed;
        self.selected_package = selected_path
            .and_then(|path| self.packages.iter().position(|entry| entry.path == path))
            .unwrap_or_else(|| {
                self.selected_package
                    .min(self.packages.len().saturating_sub(1))
            });
        self.selected_installed = selected_project
            .and_then(|project| {
                self.installed
                    .iter()
                    .position(|service| service.project == project)
            })
            .unwrap_or_else(|| {
                self.selected_installed
                    .min(self.installed.len().saturating_sub(1))
            });
        self.horizontal_scroll.reset_position();
    }

    /// 设置当前会话是否允许产生运行副作用。
    pub const fn set_control_allowed(&mut self, allowed: bool) {
        self.control_allowed = allowed;
    }

    /// 设置纯文本兼容显示。
    pub const fn set_plain_mode(&mut self, plain: bool) {
        self.plain_mode = plain;
    }

    /// 返回当前顶层视图。
    pub const fn tab(&self) -> PackageWorkspaceTab {
        self.tab
    }

    /// 返回全部可用包。
    pub fn packages(&self) -> &[PackageWorkspaceEntry] {
        &self.packages
    }

    /// 返回全部已安装项。
    pub fn installed(&self) -> &[crate::package::InstalledService] {
        &self.installed
    }

    /// 返回当前包选择索引。
    pub const fn selected_package_index(&self) -> usize {
        self.selected_package
    }

    /// 返回当前安装项选择索引。
    pub const fn selected_installed_index(&self) -> usize {
        self.selected_installed
    }

    /// 返回当前选中的包。
    pub fn selected_package(&self) -> Option<&PackageWorkspaceEntry> {
        self.packages.get(self.selected_package)
    }

    /// 返回当前选中的安装项。
    pub fn selected_installed(&self) -> Option<&crate::package::InstalledService> {
        self.installed.get(self.selected_installed)
    }

    /// 返回工作台进入时关联的 Service 来源。
    pub fn context_source(&self) -> Option<&std::path::Path> {
        self.context_source.as_deref()
    }

    /// 返回最近一次操作反馈。
    pub fn feedback(&self) -> Option<&str> {
        self.feedback.as_deref()
    }

    /// 返回是否允许执行有副作用动作。
    pub const fn control_allowed(&self) -> bool {
        self.control_allowed
    }

    /// 返回帮助是否可见。
    pub const fn help_visible(&self) -> bool {
        self.help_visibility.visible()
    }

    /// 返回纯文本兼容显示是否启用。
    pub const fn plain_mode(&self) -> bool {
        self.plain_mode
    }

    /// 返回当前导出项选择器。
    pub fn export_picker(&self) -> Option<(&[String], usize)> {
        self.export_picker
            .as_ref()
            .map(|picker| (picker.entries.as_slice(), picker.selected))
    }

    /// 返回当前是否正在等待永久清理的第二次确认。
    pub fn purge_confirmation(&self) -> Option<&str> {
        self.purge_confirmation.as_deref()
    }

    /// 返回当前是否正在等待删除包文件的第二次确认。
    pub fn package_delete_confirmation(&self) -> Option<&std::path::Path> {
        self.package_delete_confirmation.as_deref()
    }

    /// 返回一段折叠文本当前应使用的偏移。
    pub(crate) const fn text_offset(&self, selected: bool) -> usize {
        self.horizontal_scroll.offset(selected)
    }

    /// 返回手动水平偏移，供交互测试和状态提示使用。
    pub const fn horizontal_offset(&self) -> usize {
        self.horizontal_scroll.manual_offset()
    }

    /// 返回自动横移是否启用。
    pub const fn auto_scroll_enabled(&self) -> bool {
        self.horizontal_scroll.auto_enabled()
    }

    /// 返回当前选中项是否处于手动滚动冻结期。
    pub const fn manual_scroll_frozen(&self) -> bool {
        self.horizontal_scroll.manual_frozen()
    }

    /// 推进全部折叠文本的恒速自动横移。
    pub fn advance_auto_scroll(&mut self, elapsed: Duration) -> bool {
        let maximum = self.all_text_maximum();
        self.horizontal_scroll.advance(elapsed, maximum)
    }

    /// 在两个顶层视图之间切换。
    fn switch_tab(&mut self) {
        self.tab = match self.tab {
            PackageWorkspaceTab::Packages => PackageWorkspaceTab::Installed,
            PackageWorkspaceTab::Installed => PackageWorkspaceTab::Packages,
        };
        self.horizontal_scroll.reset_position();
    }

    /// 选择当前视图的下一项。
    fn select_next(&mut self) {
        let (selected, length) = match self.tab {
            PackageWorkspaceTab::Packages => (&mut self.selected_package, self.packages.len()),
            PackageWorkspaceTab::Installed => (&mut self.selected_installed, self.installed.len()),
        };
        if length > 0 {
            *selected = (*selected + 1) % length;
            self.horizontal_scroll.reset_position();
        }
    }

    /// 选择当前视图的上一项。
    fn select_previous(&mut self) {
        let (selected, length) = match self.tab {
            PackageWorkspaceTab::Packages => (&mut self.selected_package, self.packages.len()),
            PackageWorkspaceTab::Installed => (&mut self.selected_installed, self.installed.len()),
        };
        if length > 0 {
            *selected = selected.checked_sub(1).unwrap_or(length - 1);
            self.horizontal_scroll.reset_position();
        }
    }
}
