use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent};

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

/// 表单字段的可编辑值和可选枚举值。
#[derive(Clone, Debug)]
pub(super) struct DialogField {
    pub(super) label: &'static str,
    pub(super) value: String,
    pub(super) choices: Vec<String>,
    cursor: usize,
    kind: DialogFieldKind,
}

/// 弹窗字段采用的输入控件类型。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DialogFieldKind {
    Text,
    Choice,
    Map,
}

/// 表单输入弹窗。
#[derive(Clone, Debug)]
pub(crate) struct Dialog {
    kind: DialogKind,
    fields: Vec<DialogField>,
    selected: usize,
    map_editor: Option<MapEditor>,
}

impl Dialog {
    /// 创建项目基础信息弹窗。
    pub(crate) fn project(config: &FormConfig) -> Self {
        Self {
            kind: DialogKind::Project,
            fields: config_task_defaults::project_fields(config),
            selected: 0,
            map_editor: None,
        }
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
        Self {
            kind: DialogKind::Task(original.map(str::to_owned), Box::new(task.clone())),
            fields: config_task_dialog::fields(original, task),
            selected: 0,
            map_editor: None,
        }
    }

    /// 创建命名 profile 编辑弹窗。
    pub(crate) fn profile(
        original: Option<&str>,
        profile: &FormProfile,
        config: &FormConfig,
    ) -> Self {
        Self {
            kind: DialogKind::Profile(original.map(str::to_owned)),
            fields: config_profile::fields(original, profile, config),
            selected: 0,
            map_editor: None,
        }
    }

    /// 创建当前 Task 的健康检查编辑弹窗。
    pub(crate) fn health(task_name: &str, task: &FormTask) -> Self {
        Self {
            kind: DialogKind::Health(task_name.to_owned()),
            fields: config_health_dialog::fields(task),
            selected: 0,
            map_editor: None,
        }
    }

    /// 创建管理依赖编辑弹窗。
    pub(crate) fn dependency(original: Option<&str>, dependency: &FormDependency) -> Self {
        let version = if dependency.version == "source" {
            ""
        } else {
            dependency.version.as_str()
        };
        Self {
            kind: DialogKind::DependencyBasic(
                original.map(str::to_owned),
                Box::new(dependency.clone()),
            ),
            fields: vec![
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
            selected: 0,
            map_editor: None,
        }
    }

    /// 创建管理依赖高级传输策略弹窗。
    pub(crate) fn dependency_advanced(name: &str, dependency: &FormDependency) -> Self {
        let verify = dependency.verify.as_ref();
        Self {
            kind: DialogKind::DependencyAdvanced(name.to_owned(), Box::new(dependency.clone())),
            fields: vec![
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
            selected: 0,
            map_editor: None,
        }
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

    /// 返回弹窗字段供界面绘制。
    pub(crate) fn fields(&self) -> impl Iterator<Item = (&str, &str, bool)> {
        self.fields
            .iter()
            .enumerate()
            .map(|(index, field)| (field.label, field.value.as_str(), index == self.selected))
    }

    /// 返回字段数量与当前选择，供窄终端滚动显示。
    pub(crate) const fn field_position(&self) -> (usize, usize) {
        (self.fields.len(), self.selected)
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
        if self.map_editor.is_some() {
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

    /// 移动弹窗字段选择。
    pub(crate) fn next_field(&mut self, forward: bool) {
        self.selected = if forward {
            (self.selected + 1) % self.fields.len()
        } else {
            self.selected
                .checked_sub(1)
                .unwrap_or(self.fields.len() - 1)
        };
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
}

/// 创建一个弹窗字段。
pub(super) fn field(
    label: &'static str,
    value: &str,
    choices: &'static [&'static str],
) -> DialogField {
    DialogField {
        label,
        value: value.to_owned(),
        choices: choices.iter().map(|value| (*value).to_owned()).collect(),
        cursor: value.chars().count(),
        kind: if choices.is_empty() {
            DialogFieldKind::Text
        } else {
            DialogFieldKind::Choice
        },
    }
}

/// 创建运行期生成选项的弹窗字段。
pub(super) fn choice_field(label: &'static str, value: &str, choices: Vec<String>) -> DialogField {
    DialogField {
        label,
        value: value.to_owned(),
        choices,
        cursor: value.chars().count(),
        kind: DialogFieldKind::Choice,
    }
}

/// 创建使用键值表子弹窗编辑的映射字段。
pub(super) fn map_field(label: &'static str, values: &BTreeMap<String, String>) -> DialogField {
    let value = map_text(values);
    DialogField {
        label,
        cursor: value.chars().count(),
        value,
        choices: Vec::new(),
        kind: DialogFieldKind::Map,
    }
}

/// 把字符序号转换为 UTF-8 字节位置。
fn char_to_byte(value: &str, index: usize) -> usize {
    value
        .char_indices()
        .nth(index)
        .map_or(value.len(), |(byte, _)| byte)
}
