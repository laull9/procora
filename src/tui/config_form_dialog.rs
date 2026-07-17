use std::collections::BTreeMap;

use super::{
    config_form::{FormConfig, FormDependency, FormTask, FormTaskDependency, FormVerify},
    config_health_dialog,
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
    Dependency(Option<String>),
}

/// 表单字段的可编辑值和可选枚举值。
#[derive(Clone, Debug)]
pub(super) struct DialogField {
    pub(super) label: &'static str,
    pub(super) value: String,
    pub(super) choices: Vec<String>,
}

/// 表单输入弹窗。
#[derive(Clone, Debug)]
pub(crate) struct Dialog {
    kind: DialogKind,
    fields: Vec<DialogField>,
    selected: usize,
}

impl Dialog {
    /// 创建项目基础信息弹窗。
    pub(crate) fn project(config: &FormConfig) -> Self {
        Self {
            kind: DialogKind::Project,
            fields: config_task_defaults::project_fields(config),
            selected: 0,
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
        }
    }

    /// 创建当前 Task 的健康检查编辑弹窗。
    pub(crate) fn health(task_name: &str, task: &FormTask) -> Self {
        Self {
            kind: DialogKind::Health(task_name.to_owned()),
            fields: config_health_dialog::fields(task),
            selected: 0,
        }
    }

    /// 创建管理依赖编辑弹窗。
    pub(crate) fn dependency(original: Option<&str>, dependency: &FormDependency) -> Self {
        let verify = dependency.verify.as_ref();
        Self {
            kind: DialogKind::Dependency(original.map(str::to_owned)),
            fields: vec![
                field("依赖名称", original.unwrap_or(""), &[]),
                field("来源", &dependency.source, &[]),
                field("版本", &dependency.version, &[]),
                field(
                    "SHA-256（可空）",
                    dependency.checksum.as_deref().unwrap_or(""),
                    &[],
                ),
                field("解包", &dependency.unpack, &["auto", "never"]),
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
            DialogKind::Dependency(Some(_)) => "编辑管理依赖",
            DialogKind::Dependency(None) => "新建管理依赖",
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
        !self.fields[self.selected].choices.is_empty()
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
    }

    /// 删除当前普通文本字段的最后一个字符。
    pub(crate) fn backspace(&mut self) {
        let field = &mut self.fields[self.selected];
        if field.choices.is_empty() {
            field.value.pop();
        }
    }

    /// 向当前普通文本字段追加字符。
    pub(crate) fn insert(&mut self, character: char) {
        let field = &mut self.fields[self.selected];
        if field.choices.is_empty() {
            field.value.push(character);
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
            DialogKind::Dependency(original) => {
                let name = self.fields[0].value.trim();
                require(name, "依赖名称")?;
                let command = optional(&self.fields[7].value);
                let args = parse_args(&self.fields[8].value, "验证参数")?;
                let contains = optional(&self.fields[9].value);
                let dependency = FormDependency {
                    source: required_value(&self.fields[1].value, "来源")?,
                    version: required_value(&self.fields[2].value, "版本")?,
                    checksum: optional(&self.fields[3].value),
                    unpack: self.fields[4].value.clone(),
                    kind: self.fields[5].value.clone(),
                    path: optional(&self.fields[6].value),
                    verify: (command.is_some() || !args.is_empty() || contains.is_some())
                        .then_some(FormVerify {
                            command,
                            args,
                            contains,
                        }),
                };
                replace_entry(
                    &mut config.dependencies,
                    original.as_deref(),
                    name,
                    dependency,
                    "管理依赖",
                )?;
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
    }
}

/// 创建运行期生成选项的弹窗字段。
pub(super) fn choice_field(label: &'static str, value: &str, choices: Vec<String>) -> DialogField {
    DialogField {
        label,
        value: value.to_owned(),
        choices,
    }
}

/// 将环境变量映射转换为弹窗文本。
pub(super) fn map_text(values: &BTreeMap<String, String>) -> String {
    serde_json::to_string(values).expect("字符串映射序列化不会失败")
}

/// 将依赖映射转换为弹窗文本。
pub(super) fn dependencies_text(values: &BTreeMap<String, FormTaskDependency>) -> String {
    values
        .iter()
        .map(|(name, dependency)| format!("{name}:{}", dependency.condition))
        .collect::<Vec<_>>()
        .join(",")
}

/// 解析逗号分隔的环境变量集合。
pub(super) fn parse_map(value: &str, label: &str) -> Result<BTreeMap<String, String>, String> {
    let value = value.trim();
    if value.starts_with('{') {
        return serde_json::from_str(value).map_err(|error| format!("{label} JSON 无效：{error}"));
    }
    value
        .split(',')
        .filter(|item| !item.trim().is_empty())
        .map(|item| {
            let Some((key, value)) = item.split_once('=') else {
                return Err(format!("{label} 必须使用 KEY=VALUE 格式"));
            };
            let key = key.trim();
            require(key, label)?;
            Ok((key.to_owned(), value.trim().to_owned()))
        })
        .collect()
}

/// 解析逗号分隔的 Task 依赖集合。
pub(super) fn parse_dependencies(
    value: &str,
) -> Result<BTreeMap<String, FormTaskDependency>, String> {
    let mut dependencies = BTreeMap::new();
    for item in value.split(',').filter(|item| !item.trim().is_empty()) {
        let (name, condition) = item.split_once(':').unwrap_or((item, "started"));
        let name = name.trim();
        require(name, "依赖 Task")?;
        let condition = match condition.trim() {
            "started" | "process_started" => "started",
            "healthy" | "process_healthy" => "healthy",
            "completed_successfully" | "process_completed_successfully" => "completed_successfully",
            _ => {
                return Err("依赖条件只能是 started、healthy 或 completed_successfully".to_owned());
            }
        };
        if dependencies
            .insert(
                name.to_owned(),
                FormTaskDependency {
                    condition: condition.to_owned(),
                },
            )
            .is_some()
        {
            return Err(format!("依赖 Task `{name}` 重复出现"));
        }
    }
    Ok(dependencies)
}

/// 替换或新增一个带名称的配置条目，并防止意外覆盖。
pub(super) fn replace_entry<T>(
    entries: &mut BTreeMap<String, T>,
    original: Option<&str>,
    name: &str,
    value: T,
    label: &str,
) -> Result<(), String> {
    if original != Some(name) && entries.contains_key(name) {
        return Err(format!("{label} 名称 `{name}` 已存在"));
    }
    if let Some(original) = original {
        entries.remove(original);
    }
    entries.insert(name.to_owned(), value);
    Ok(())
}

/// 确保必填字段包含非空文本。
fn require(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} 不能为空"))
    } else {
        Ok(())
    }
}

/// 读取必填字段，并移除首尾空白。
pub(super) fn required_value(value: &str, label: &str) -> Result<String, String> {
    require(value, label)?;
    Ok(value.trim().to_owned())
}

/// 把可空文本转为可选字段。
pub(super) fn optional(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_owned())
}

/// 将空白分隔的参数文本转换为参数数组。
pub(super) fn args_text(values: &[String]) -> String {
    serde_json::to_string(values).expect("字符串数组序列化不会失败")
}

/// 解析精确 JSON 参数数组，并兼容旧版空格分隔输入。
pub(super) fn parse_args(value: &str, label: &str) -> Result<Vec<String>, String> {
    let value = value.trim();
    if value.starts_with('[') {
        return serde_json::from_str(value).map_err(|error| format!("{label} JSON 无效：{error}"));
    }
    Ok(value.split_whitespace().map(str::to_owned).collect())
}

/// 解析一个带单位的紧凑时长字段。
pub(super) fn parse_duration(value: &str, label: &str) -> Result<u64, String> {
    crate::config::parse_duration(value).map_err(|error| format!("{label}无效：{error}"))
}

/// 解析表单中的非负 32 位整数。
pub(super) fn parse_u32(value: &str, label: &str) -> Result<u32, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{label} 必须是非负整数"))
}

/// 解析精确 JSON 整数数组，并兼容逗号或空白分隔输入。
pub(super) fn parse_i32_list(value: &str, label: &str) -> Result<Vec<i32>, String> {
    let value = value.trim();
    let values = if value.starts_with('[') {
        serde_json::from_str(value).map_err(|error| format!("{label} JSON 无效：{error}"))?
    } else {
        value
            .split([',', ' '])
            .filter(|item| !item.trim().is_empty())
            .map(|item| {
                item.trim()
                    .parse()
                    .map_err(|_| format!("{label} 必须是整数数组"))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    if values.iter().any(|value| *value < 0) {
        return Err(format!("{label}不能包含负数"));
    }
    Ok(values)
}
