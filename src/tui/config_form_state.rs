use crossterm::event::{KeyCode, KeyEvent};

use super::config_form::{FormConfig, FormPane};
use super::config_form_dialog::Dialog;

/// 表单模式的选择状态与弹窗状态。
#[derive(Clone, Debug)]
pub(crate) struct FormState {
    config: FormConfig,
    pane: FormPane,
    selected: usize,
    dialog: Option<Dialog>,
    pending_delete: Option<DeleteTarget>,
}

/// 表单按键处理后交给编辑器的结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FormEvent {
    /// 没有发生内容变更。
    None,
    /// 表单内容已变更，需要重新生成配置文本。
    Changed,
    /// profile 已切换，需要重编译并刷新有效值预览。
    Reload,
    /// 需要显示提示。
    Message(String),
}

/// 等待确认删除的条目。
#[derive(Clone, Debug)]
enum DeleteTarget {
    Profile(String),
    Task(String),
    Dependency(String),
}

impl FormState {
    /// 从已校验配置创建默认聚焦项目区的表单状态。
    pub(crate) fn new(config: FormConfig) -> Self {
        Self {
            config,
            pane: FormPane::Project,
            selected: 0,
            dialog: None,
            pending_delete: None,
        }
    }

    /// 返回当前表单配置。
    pub(crate) const fn config(&self) -> &FormConfig {
        &self.config
    }

    /// 返回当前聚焦区域。
    pub(crate) const fn pane(&self) -> FormPane {
        self.pane
    }

    /// 返回当前列表选中序号。
    pub(crate) const fn selected(&self) -> usize {
        self.selected
    }

    /// 返回当前弹窗，供界面绘制。
    pub(crate) fn dialog(&self) -> Option<&Dialog> {
        self.dialog.as_ref()
    }

    /// 返回等待确认删除的名称。
    pub(crate) fn pending_delete_name(&self) -> Option<&str> {
        self.pending_delete.as_ref().map(|target| match target {
            DeleteTarget::Profile(name)
            | DeleteTarget::Task(name)
            | DeleteTarget::Dependency(name) => name.as_str(),
        })
    }

    /// 返回当前是否正在等待删除确认。
    pub(crate) const fn has_pending_delete(&self) -> bool {
        self.pending_delete.is_some()
    }

    /// 处理结构化页面中的一次按键。
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> FormEvent {
        if self.dialog.is_some() {
            return self.handle_dialog(key);
        }
        if self.pending_delete.is_some() {
            return self.handle_delete(key);
        }
        match key.code {
            KeyCode::Left | KeyCode::BackTab => self.move_pane(false),
            KeyCode::Right | KeyCode::Tab => self.move_pane(true),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(false),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(true),
            KeyCode::Enter => self.open_current(),
            KeyCode::Char('h') => self.open_health(),
            KeyCode::Char('a') => self.open_dependency_advanced(),
            KeyCode::Char('n') => self.open_new(),
            KeyCode::Char('d') => self.request_delete(),
            _ => FormEvent::None,
        }
    }

    /// 为当前选中 Task 打开独立健康检查编辑弹窗。
    fn open_health(&mut self) -> FormEvent {
        if self.pane != FormPane::Tasks {
            return FormEvent::Message("请先切换到 Task 区域".to_owned());
        }
        self.dialog = self.task_name().and_then(|name| {
            self.config
                .tasks
                .get(&name)
                .map(|task| Dialog::health(&name, task))
        });
        if self.dialog.is_some() {
            FormEvent::None
        } else {
            FormEvent::Message("当前列表为空；请先新建 Task".to_owned())
        }
    }

    /// 为当前管理依赖打开高级下载与 SSH 策略。
    fn open_dependency_advanced(&mut self) -> FormEvent {
        if self.pane != FormPane::Dependencies {
            return FormEvent::Message("请先切换到管理依赖区域".to_owned());
        }
        self.dialog = self.dependency_name().and_then(|name| {
            self.config
                .dependencies
                .get(&name)
                .map(|dependency| Dialog::dependency_advanced(&name, dependency))
        });
        if self.dialog.is_some() {
            FormEvent::None
        } else {
            FormEvent::Message("当前列表为空；请先新建管理依赖".to_owned())
        }
    }

    /// 切换聚焦区域。
    fn move_pane(&mut self, forward: bool) -> FormEvent {
        self.pane = match (self.pane, forward) {
            (FormPane::Project, true) | (FormPane::Dependencies, false) => FormPane::Tasks,
            (FormPane::Tasks, true) | (FormPane::Profiles, false) => FormPane::Dependencies,
            (FormPane::Dependencies, true) | (FormPane::Project, false) => FormPane::Profiles,
            (FormPane::Profiles, true) | (FormPane::Tasks, false) => FormPane::Project,
        };
        self.selected = 0;
        FormEvent::None
    }

    /// 移动当前列表项选择。
    fn move_selection(&mut self, forward: bool) -> FormEvent {
        let count = self.item_count();
        if count > 0 {
            self.selected = if forward {
                (self.selected + 1) % count
            } else {
                self.selected.checked_sub(1).unwrap_or(count - 1)
            };
        }
        FormEvent::None
    }

    /// 打开当前项目、Task 或依赖的编辑弹窗。
    fn open_current(&mut self) -> FormEvent {
        self.dialog = match self.pane {
            FormPane::Project => Some(Dialog::project(&self.config)),
            FormPane::Profiles => self.profile_name().and_then(|name| {
                self.config
                    .profiles
                    .get(&name)
                    .map(|profile| Dialog::profile(Some(&name), profile, &self.config))
            }),
            FormPane::Tasks => self.task_name().and_then(|name| {
                self.config
                    .tasks
                    .get(&name)
                    .map(|task| Dialog::task(Some(&name), task))
            }),
            FormPane::Dependencies => self.dependency_name().and_then(|name| {
                self.config
                    .dependencies
                    .get(&name)
                    .map(|dependency| Dialog::dependency(Some(&name), dependency))
            }),
        };
        if self.dialog.is_some() {
            FormEvent::None
        } else {
            FormEvent::Message("当前列表为空；按 n 新建".to_owned())
        }
    }

    /// 为当前列表创建一条默认条目的编辑弹窗。
    fn open_new(&mut self) -> FormEvent {
        self.dialog = Some(match self.pane {
            FormPane::Project => Dialog::project(&self.config),
            FormPane::Profiles => Dialog::new_profile(&self.config),
            FormPane::Tasks => Dialog::new_task(&self.config),
            FormPane::Dependencies => Dialog::new_dependency(),
        });
        FormEvent::None
    }

    /// 请求二次确认删除当前 Task 或管理依赖。
    fn request_delete(&mut self) -> FormEvent {
        self.pending_delete = match self.pane {
            FormPane::Project => None,
            FormPane::Profiles => self.profile_name().map(DeleteTarget::Profile),
            FormPane::Tasks => self.task_name().map(DeleteTarget::Task),
            FormPane::Dependencies => self.dependency_name().map(DeleteTarget::Dependency),
        };
        self.pending_delete.as_ref().map_or_else(
            || FormEvent::Message("没有可删除的条目".to_owned()),
            |target| {
                let name = match target {
                    DeleteTarget::Profile(name)
                    | DeleteTarget::Task(name)
                    | DeleteTarget::Dependency(name) => name,
                };
                FormEvent::Message(format!("再次按 d 删除 `{name}`，Esc 取消"))
            },
        )
    }

    /// 处理删除确认。
    fn handle_delete(&mut self, key: KeyEvent) -> FormEvent {
        match key.code {
            KeyCode::Esc => {
                self.pending_delete = None;
                FormEvent::Message("已取消删除".to_owned())
            }
            KeyCode::Char('d') => {
                match self.pending_delete.take().expect("删除确认状态存在") {
                    DeleteTarget::Profile(name) => {
                        let dependents = self.config.profile_dependents(&name);
                        if !dependents.is_empty() {
                            return FormEvent::Message(format!(
                                "profile `{name}` 仍被 {} 继承，不能删除",
                                dependents.join("、")
                            ));
                        }
                        self.config.remove_profile(&name);
                        self.clamp_selection();
                        return FormEvent::Reload;
                    }
                    DeleteTarget::Task(name) => {
                        self.config.tasks.remove(&name);
                    }
                    DeleteTarget::Dependency(name) => {
                        self.config.dependencies.remove(&name);
                    }
                }
                self.clamp_selection();
                FormEvent::Changed
            }
            _ => FormEvent::None,
        }
    }

    /// 处理弹窗内的输入、选项切换、确认和取消。
    fn handle_dialog(&mut self, key: KeyEvent) -> FormEvent {
        let dialog = self.dialog.as_mut().expect("弹窗状态存在");
        match key.code {
            KeyCode::Esc => {
                self.dialog = None;
                FormEvent::Message("已取消编辑".to_owned())
            }
            KeyCode::Tab | KeyCode::Down => {
                dialog.next_field(true);
                FormEvent::None
            }
            KeyCode::BackTab | KeyCode::Up => {
                dialog.next_field(false);
                FormEvent::None
            }
            KeyCode::Left => {
                dialog.cycle_choice(false);
                FormEvent::None
            }
            KeyCode::Right => {
                dialog.cycle_choice(true);
                FormEvent::None
            }
            KeyCode::Backspace => {
                dialog.backspace();
                FormEvent::None
            }
            KeyCode::Char(character) => {
                dialog.insert(character);
                FormEvent::None
            }
            KeyCode::Enter => self.commit_dialog(),
            _ => FormEvent::None,
        }
    }

    /// 将弹窗草稿提交到结构化配置。
    fn commit_dialog(&mut self) -> FormEvent {
        let dialog = self.dialog.take().expect("弹窗状态存在");
        match dialog.commit(&mut self.config) {
            Ok(reload) => {
                self.clamp_selection();
                if reload {
                    FormEvent::Reload
                } else {
                    FormEvent::Changed
                }
            }
            Err(message) => {
                self.dialog = Some(dialog);
                FormEvent::Message(message)
            }
        }
    }

    /// 返回当前选中 profile 的名称。
    fn profile_name(&self) -> Option<String> {
        self.config.profiles.keys().nth(self.selected).cloned()
    }

    /// 返回当前选中 Task 的名称。
    fn task_name(&self) -> Option<String> {
        self.config.tasks.keys().nth(self.selected).cloned()
    }

    /// 返回当前选中管理依赖的名称。
    fn dependency_name(&self) -> Option<String> {
        self.config.dependencies.keys().nth(self.selected).cloned()
    }

    /// 返回当前聚焦列表的条目总数。
    fn item_count(&self) -> usize {
        match self.pane {
            FormPane::Project => 1,
            FormPane::Profiles => self.config.profiles.len(),
            FormPane::Tasks => self.config.tasks.len(),
            FormPane::Dependencies => self.config.dependencies.len(),
        }
    }

    /// 使列表选择序号保持在有效范围内。
    fn clamp_selection(&mut self) {
        self.selected = self.selected.min(self.item_count().saturating_sub(1));
    }
}
