use crossterm::event::{KeyCode, KeyEvent};

pub(super) use super::config_form_field::{
    DialogField, choice_field, directory_field, field, map_field,
};
pub(super) use super::config_form_value::{
    args_text, dependencies_text, map_text, optional, parse_args, parse_dependencies,
    parse_duration, parse_i32_list, parse_map, parse_u32, replace_entry, required_value,
};
use super::{
    config_dependency_dialog,
    config_form::{FormConfig, FormDependency, FormTask},
    config_health_dialog,
    config_map_dialog::MapEditor,
    config_profile::{self, FormProfile},
    config_task_defaults, config_task_dialog,
};
use super::{
    config_directory_picker::DirectoryPicker,
    config_form_field::{DialogFieldKind, char_to_byte},
};

/// 弹窗正在编辑的实体类型。
#[derive(Clone, Debug)]
enum DialogKind {
    Project,
    Profile(Option<String>),
    Task(Option<String>, Box<FormTask>),
    Health(String),
    DependencyBasic(Option<String>, Box<FormDependency>),
    DependencyAdvanced(String, Box<FormDependency>),
}

/// 表单输入弹窗。
#[derive(Clone, Debug)]
pub(crate) struct Dialog {
    kind: DialogKind,
    pub(super) fields: Vec<DialogField>,
    initial_values: Vec<String>,
    pub(super) selected: usize,
    map_editor: Option<MapEditor>,
    pub(super) directory_picker: Option<DirectoryPicker>,
}

impl Dialog {
    /// 从实体类型和字段快照创建可探测本轮修改的弹窗。
    fn new(kind: DialogKind, fields: Vec<DialogField>) -> Self {
        let initial_values = fields.iter().map(|field| field.value.clone()).collect();
        Self {
            kind,
            fields,
            initial_values,
            selected: 0,
            map_editor: None,
            directory_picker: None,
        }
    }

    /// 创建项目基础信息弹窗。
    pub(crate) fn project(config: &FormConfig) -> Self {
        Self::new(
            DialogKind::Project,
            config_task_defaults::project_fields(config),
        )
    }

    /// 创建空白 Task 弹窗。
    pub(crate) fn new_task(config: &FormConfig) -> Self {
        Self::task(None, &config.new_task_value())
    }

    /// 创建空白 profile 弹窗。
    pub(crate) fn new_profile(config: &FormConfig) -> Self {
        Self::profile(None, &FormProfile::default(), config)
    }

    /// 创建空白管理依赖弹窗。
    pub(crate) fn new_dependency() -> Self {
        Self::dependency(None, &FormDependency::default_value())
    }

    /// 创建 Task 编辑弹窗。
    pub(crate) fn task(original: Option<&str>, task: &FormTask) -> Self {
        Self::new(
            DialogKind::Task(original.map(str::to_owned), Box::new(task.clone())),
            config_task_dialog::fields(original, task),
        )
    }

    /// 创建命名 profile 编辑弹窗。
    pub(crate) fn profile(
        original: Option<&str>,
        profile: &FormProfile,
        config: &FormConfig,
    ) -> Self {
        Self::new(
            DialogKind::Profile(original.map(str::to_owned)),
            config_profile::fields(original, profile, config),
        )
    }

    /// 创建当前 Task 的健康检查编辑弹窗。
    pub(crate) fn health(task_name: &str, task: &FormTask) -> Self {
        Self::new(
            DialogKind::Health(task_name.to_owned()),
            config_health_dialog::fields(task),
        )
    }

    /// 创建管理依赖编辑弹窗。
    pub(crate) fn dependency(original: Option<&str>, dependency: &FormDependency) -> Self {
        let version = if dependency.version == "source" {
            ""
        } else {
            dependency.version.as_str()
        };
        Self::new(
            DialogKind::DependencyBasic(original.map(str::to_owned), Box::new(dependency.clone())),
            vec![
                field("依赖名称", original.unwrap_or(""), &[]),
                field("来源（HTTP / SSH / SCP / 本地）", &dependency.source, &[]),
                field("版本（可空，默认 source）", version, &[]),
                field(
                    "SHA-256（可空）",
                    dependency.checksum.as_deref().unwrap_or(""),
                    &[],
                ),
                field(
                    "类型",
                    &dependency.kind,
                    &["auto", "binary", "file", "directory"],
                ),
                field(
                    "归档内路径（可空）",
                    dependency.path.as_deref().unwrap_or(""),
                    &[],
                ),
            ],
        )
    }

    /// 创建管理依赖高级传输策略弹窗。
    pub(crate) fn dependency_advanced(name: &str, dependency: &FormDependency) -> Self {
        let verify = dependency.verify.as_ref();
        Self::new(
            DialogKind::DependencyAdvanced(name.to_owned(), Box::new(dependency.clone())),
            vec![
                field("镜像（JSON 数组）", &args_text(&dependency.mirrors), &[]),
                field("解包", &dependency.unpack, &["auto", "never"]),
                field(
                    "失败重试次数",
                    &dependency.download.retries.to_string(),
                    &[],
                ),
                field(
                    "单次总超时",
                    &crate::config::format_duration(dependency.download.timeout_ms),
                    &[],
                ),
                field(
                    "最大下载字节",
                    &dependency.download.max_bytes.to_string(),
                    &[],
                ),
                map_field(
                    "HTTP 请求头（按 F4 编辑键值表）",
                    &dependency.download.headers,
                ),
                field(
                    "SSH 私钥（可空）",
                    dependency.ssh.identity_file.as_deref().unwrap_or(""),
                    &[],
                ),
                field(
                    "SSH known_hosts（可空）",
                    dependency.ssh.known_hosts_file.as_deref().unwrap_or(""),
                    &[],
                ),
                field(
                    "验证命令（可空）",
                    verify
                        .and_then(|value| value.command.as_deref())
                        .unwrap_or(""),
                    &[],
                ),
                field(
                    "验证参数（空格分隔）",
                    &verify.map_or_else(|| "[]".to_owned(), |value| args_text(&value.args)),
                    &[],
                ),
                field(
                    "验证输出包含（可空）",
                    verify
                        .and_then(|value| value.contains.as_deref())
                        .unwrap_or(""),
                    &[],
                ),
            ],
        )
    }

    /// 返回弹窗标题。
    pub(crate) fn title(&self) -> &'static str {
        match self.kind {
            DialogKind::Project => "编辑项目",
            DialogKind::Profile(Some(_)) => "编辑 profile",
            DialogKind::Profile(None) => "新建 profile",
            DialogKind::Task(Some(_), _) => "编辑 Task",
            DialogKind::Task(None, _) => "新建 Task",
            DialogKind::Health(_) => "编辑健康检查",
            DialogKind::DependencyBasic(Some(_), _) => "编辑管理依赖",
            DialogKind::DependencyBasic(None, _) => "新建管理依赖",
            DialogKind::DependencyAdvanced(_, _) => "高级下载与 SSH 策略",
        }
    }

    /// 返回本轮字段内容是否相对打开时发生变化。
    pub(crate) fn is_dirty(&self) -> bool {
        self.fields
            .iter()
            .map(|field| field.value.as_str())
            .ne(self.initial_values.iter().map(String::as_str))
    }

    /// 返回当前是否为 Task 主编辑弹窗。
    pub(crate) fn is_task(&self) -> bool {
        matches!(self.kind, DialogKind::Task(_, _))
    }

    /// 返回弹窗字段供界面绘制。
    pub(crate) fn fields(&self) -> impl Iterator<Item = (&str, &str, bool)> {
        self.fields
            .iter()
            .enumerate()
            .filter(|(index, _)| self.field_visible(*index))
            .map(|(index, field)| (field.label, field.value.as_str(), index == self.selected))
    }

    /// 返回字段数量与当前选择，供窄终端滚动显示。
    pub(crate) fn field_position(&self) -> (usize, usize) {
        let mut count = 0;
        let mut selected = 0;
        for index in 0..self.fields.len() {
            if self.field_visible(index) {
                if index == self.selected {
                    selected = count;
                }
                count += 1;
            }
        }
        (count, selected)
    }

    /// 返回当前字段是否为选择器。
    pub(crate) fn selected_is_choice(&self) -> bool {
        self.fields[self.selected].kind == DialogFieldKind::Choice
    }

    /// 返回当前字段是否支持打开键值表。
    pub(crate) fn selected_is_map(&self) -> bool {
        self.fields[self.selected].kind == DialogFieldKind::Map
    }

    /// 返回当前可直接输入的字段，选择器和子弹窗不显示文本光标。
    pub(crate) fn selected_input(&self) -> Option<(&str, &str, usize)> {
        if self.map_editor.is_some() || self.directory_picker.is_some() {
            return None;
        }
        let field = &self.fields[self.selected];
        (field.kind != DialogFieldKind::Choice).then_some((
            field.label,
            field.value.as_str(),
            field.cursor,
        ))
    }

    /// 返回当前打开的键值表。
    pub(super) const fn map_editor(&self) -> Option<&MapEditor> {
        self.map_editor.as_ref()
    }

    /// 返回当前是否正在编辑键值表。
    pub(crate) const fn has_map_editor(&self) -> bool {
        self.map_editor.is_some()
    }

    /// 为当前映射字段打开键值表。
    pub(crate) fn open_map_editor(&mut self) -> Result<(), String> {
        if !self.selected_is_map() {
            return Err("当前字段不是键值表".to_owned());
        }
        let values = parse_map(
            &self.fields[self.selected].value,
            self.fields[self.selected].label,
        )?;
        self.map_editor = Some(MapEditor::new(self.selected, values));
        Ok(())
    }

    /// 处理键值表输入；返回空表示当前没有打开子弹窗。
    pub(crate) fn handle_map_key(&mut self, key: KeyEvent) -> Option<Result<(), String>> {
        let editor = self.map_editor.as_mut()?;
        match key.code {
            KeyCode::Esc => {
                self.map_editor = None;
                Some(Ok(()))
            }
            KeyCode::Enter => {
                let values = match editor.values() {
                    Ok(values) => values,
                    Err(error) => return Some(Err(error)),
                };
                let field = editor.field();
                self.fields[field].value = map_text(&values);
                self.fields[field].cursor = self.fields[field].value.chars().count();
                self.map_editor = None;
                Some(Ok(()))
            }
            _ => {
                editor.handle_key(key);
                Some(Ok(()))
            }
        }
    }

    /// 将已打开键值表的草稿应用回字段，供统一保存快捷键使用。
    pub(crate) fn apply_map_editor(&mut self) -> Result<(), String> {
        let Some(editor) = self.map_editor.as_ref() else {
            return Ok(());
        };
        let values = editor.values()?;
        let field = editor.field();
        self.fields[field].value = map_text(&values);
        self.fields[field].cursor = self.fields[field].value.chars().count();
        self.map_editor = None;
        Ok(())
    }

    /// 移动弹窗字段选择。
    pub(crate) fn next_field(&mut self, forward: bool) {
        let mut next = self.selected;
        loop {
            next = if forward {
                (next + 1) % self.fields.len()
            } else {
                next.checked_sub(1).unwrap_or(self.fields.len() - 1)
            };
            if self.field_visible(next) {
                self.selected = next;
                break;
            }
        }
    }

    /// 循环当前字段的可选值。
    pub(crate) fn cycle_choice(&mut self, forward: bool) {
        let field = &mut self.fields[self.selected];
        let Some(position) = field.choices.iter().position(|value| value == &field.value) else {
            return;
        };
        let next = if forward {
            (position + 1) % field.choices.len()
        } else {
            position.checked_sub(1).unwrap_or(field.choices.len() - 1)
        };
        field.value = field.choices[next].clone();
        field.cursor = field.value.chars().count();
    }

    /// 左右移动普通文本字段的字符光标。
    pub(crate) fn move_cursor(&mut self, forward: bool) {
        let field = &mut self.fields[self.selected];
        if field.kind == DialogFieldKind::Choice {
            self.cycle_choice(forward);
            return;
        }
        field.cursor = if forward {
            (field.cursor + 1).min(field.value.chars().count())
        } else {
            field.cursor.saturating_sub(1)
        };
    }

    /// 将普通文本字段光标移动到行首或行尾。
    pub(crate) fn move_cursor_edge(&mut self, end: bool) {
        let field = &mut self.fields[self.selected];
        if field.kind != DialogFieldKind::Choice {
            field.cursor = if end { field.value.chars().count() } else { 0 };
        }
    }

    /// 删除当前普通文本字段光标前的字符。
    pub(crate) fn backspace(&mut self) {
        let field = &mut self.fields[self.selected];
        if field.kind != DialogFieldKind::Choice && field.cursor > 0 {
            let start = char_to_byte(&field.value, field.cursor - 1);
            let end = char_to_byte(&field.value, field.cursor);
            field.value.replace_range(start..end, "");
            field.cursor -= 1;
        }
    }

    /// 向当前普通文本字段光标处插入字符。
    pub(crate) fn insert(&mut self, character: char) {
        let field = &mut self.fields[self.selected];
        if field.kind != DialogFieldKind::Choice {
            let byte = char_to_byte(&field.value, field.cursor);
            field.value.insert(byte, character);
            field.cursor += 1;
        }
    }

    /// 校验弹窗字段并提交到表单模型。
    pub(crate) fn commit(&self, config: &mut FormConfig) -> Result<bool, String> {
        match &self.kind {
            DialogKind::Project => {
                let previous = config.active_profile.clone();
                let previous_vars = config.vars.clone();
                config_task_defaults::commit_project(&self.fields, config)?;
                return Ok(previous != config.active_profile || previous_vars != config.vars);
            }
            DialogKind::Profile(original) => {
                config_profile::commit(original.as_deref(), &self.fields, config)?;
                return Ok(true);
            }
            DialogKind::Task(original, baseline) => {
                config_task_dialog::commit(original.as_deref(), baseline, &self.fields, config)?;
            }
            DialogKind::Health(task_name) => {
                config_health_dialog::commit(task_name, &self.fields, config)?;
            }
            DialogKind::DependencyBasic(original, baseline) => {
                config_dependency_dialog::commit_basic(
                    original.as_deref(),
                    baseline,
                    &self.fields,
                    config,
                )?;
            }
            DialogKind::DependencyAdvanced(name, baseline) => {
                config_dependency_dialog::commit_advanced(name, baseline, &self.fields, config)?;
            }
        }
        Ok(false)
    }

    /// 返回字段在当前弹窗模式下是否应该显示和参与导航。
    fn field_visible(&self, index: usize) -> bool {
        if !matches!(&self.kind, DialogKind::Health(_)) {
            return true;
        }
        matches!(
            (self.fields[0].value.as_str(), index),
            (_, 0 | 10..=14) | ("exec", 1..=3) | ("http", 4..=9)
        )
    }
}
