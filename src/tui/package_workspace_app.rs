//! Procora 包工作台的纯交互状态与动作意图。

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;

use super::{help_ui::HelpVisibility, package_workspace_ui};

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
    purge_confirmation: Option<String>,
    uninstall_confirmation: Option<String>,
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
            purge_confirmation: None,
            uninstall_confirmation: None,
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
        if key != KeyCode::Char('D') {
            self.purge_confirmation = None;
        }
        if key != KeyCode::Char('U') {
            self.uninstall_confirmation = None;
        }
        match key {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.exit = Some(PackageWorkspaceExit::Back);
            }
            KeyCode::Char('?') => self.help_visibility.show(),
            KeyCode::Tab | KeyCode::BackTab => self.switch_tab(),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
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

    /// 在两个顶层视图之间切换。
    fn switch_tab(&mut self) {
        self.tab = match self.tab {
            PackageWorkspaceTab::Packages => PackageWorkspaceTab::Installed,
            PackageWorkspaceTab::Installed => PackageWorkspaceTab::Packages,
        };
    }

    /// 选择当前视图的下一项。
    fn select_next(&mut self) {
        let (selected, length) = match self.tab {
            PackageWorkspaceTab::Packages => (&mut self.selected_package, self.packages.len()),
            PackageWorkspaceTab::Installed => (&mut self.selected_installed, self.installed.len()),
        };
        if length > 0 {
            *selected = (*selected + 1) % length;
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
        }
    }

    /// 对当前选中包生成一个单路径动作。
    fn package_action(&mut self, action: fn(PathBuf) -> PackageWorkspaceExit) {
        if self.tab != PackageWorkspaceTab::Packages {
            self.feedback = Some("切换到“包文件”后再执行该操作".to_owned());
            return;
        }
        let Some(entry) = self.selected_package() else {
            self.feedback = Some("没有可操作的包；按 o 打开，或按 b 构建".to_owned());
            return;
        };
        if entry.info.is_none() {
            self.feedback = Some("包清单不可读；只能重新选择或查看诊断".to_owned());
            return;
        }
        self.exit = Some(action(entry.path.clone()));
    }

    /// 单个导出直接执行，多个导出进入内联选择。
    fn begin_export(&mut self) {
        if self.tab != PackageWorkspaceTab::Packages {
            self.feedback = Some("切换到“包文件”后再选择导出项".to_owned());
            return;
        }
        let Some(entry) = self.selected_package() else {
            self.feedback = Some("没有可导出的包".to_owned());
            return;
        };
        let Some(info) = &entry.info else {
            self.feedback = Some("包清单不可读，无法列出导出项".to_owned());
            return;
        };
        let entries = info.manifest.exports.keys().cloned().collect::<Vec<_>>();
        match entries.as_slice() {
            [] => self.feedback = Some("该包没有命名导出项".to_owned()),
            [only] => {
                self.exit = Some(PackageWorkspaceExit::PushExport {
                    package: entry.path.clone(),
                    entry: only.clone(),
                });
            }
            _ => {
                self.export_picker = Some(ExportPicker {
                    package: entry.path.clone(),
                    entries,
                    selected: 0,
                });
            }
        }
    }

    /// 对当前已安装 Service 生成管理动作。
    fn installed_action(
        &mut self,
        action: impl FnOnce(&str) -> PackageWorkspaceExit,
        empty_message: &str,
    ) {
        if self.tab != PackageWorkspaceTab::Installed {
            self.feedback = Some("切换到“已安装”后再执行 release 管理".to_owned());
            return;
        }
        let Some(service) = self.selected_installed() else {
            self.feedback = Some(empty_message.to_owned());
            return;
        };
        if service.error.is_some() {
            self.feedback = Some("安装状态损坏；请先根据详情修复 state.json".to_owned());
            return;
        }
        self.exit = Some(action(&service.project));
    }

    /// 用两次大写 D 确认永久删除安装数据。
    fn confirm_purge(&mut self) {
        if self.tab != PackageWorkspaceTab::Installed {
            self.feedback = Some("切换到“已安装”后再清理安装数据".to_owned());
            return;
        }
        let Some(project) = self
            .selected_installed()
            .map(|service| service.project.clone())
        else {
            self.feedback = Some("没有可清理的已安装 Service".to_owned());
            return;
        };
        if self.purge_confirmation.as_deref() == Some(project.as_str()) {
            self.purge_confirmation = None;
            self.exit = Some(PackageWorkspaceExit::Purge(project));
        } else {
            self.purge_confirmation = Some(project.clone());
            self.feedback = Some(format!(
                "永久清理会删除 `{project}` 的全部 release 和原始包；再次按大写 D 确认"
            ));
        }
    }

    /// 用两次大写 U 确认仅解除 Center 注册并保留安装数据。
    fn confirm_uninstall(&mut self) {
        if self.tab != PackageWorkspaceTab::Installed {
            self.feedback = Some("切换到“已安装”后再解除安装".to_owned());
            return;
        }
        let Some(project) = self
            .selected_installed()
            .map(|service| service.project.clone())
        else {
            self.feedback = Some("没有可解除的已安装 Service".to_owned());
            return;
        };
        if self.uninstall_confirmation.as_deref() == Some(project.as_str()) {
            self.uninstall_confirmation = None;
            self.exit = Some(PackageWorkspaceExit::Uninstall(project));
        } else {
            self.uninstall_confirmation = Some(project.clone());
            self.feedback = Some(format!(
                "将从 Center 解除 `{project}`，但保留 release 和原始包；再次按大写 U 确认"
            ));
        }
    }

    /// 处理导出项选择弹层。
    fn handle_export_key(&mut self, key: KeyCode) -> bool {
        let Some(picker) = self.export_picker.as_mut() else {
            return false;
        };
        match key {
            KeyCode::Esc | KeyCode::Char('q') => self.export_picker = None,
            KeyCode::Down | KeyCode::Char('j') => {
                picker.selected = (picker.selected + 1) % picker.entries.len();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                picker.selected = picker
                    .selected
                    .checked_sub(1)
                    .unwrap_or(picker.entries.len() - 1);
            }
            KeyCode::Enter => {
                self.exit = Some(PackageWorkspaceExit::PushExport {
                    package: picker.package.clone(),
                    entry: picker.entries[picker.selected].clone(),
                });
                self.export_picker = None;
            }
            _ => return false,
        }
        true
    }
}
