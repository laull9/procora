use std::collections::BTreeMap;

use super::config_form::{FormConfig, FormDependency, FormTask, FormTaskDependency, FormVerify};

/// 弹窗正在编辑的实体类型。
#[derive(Clone, Debug)]
enum DialogKind {
    Project,
    Task(Option<String>),
    Dependency(Option<String>),
}

/// 表单字段的可编辑值和可选枚举值。
#[derive(Clone, Debug)]
struct DialogField {
    label: &'static str,
    value: String,
    choices: &'static [&'static str],
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
            fields: vec![field("项目名称", &config.project, &[])],
            selected: 0,
        }
    }

    /// 创建空白 Task 弹窗。
    pub(crate) fn new_task() -> Self {
        Self::task(None, &FormTask::default_value())
    }

    /// 创建空白管理依赖弹窗。
    pub(crate) fn new_dependency() -> Self {
        Self::dependency(None, &FormDependency::default_value())
    }

    /// 创建 Task 编辑弹窗。
    pub(crate) fn task(original: Option<&str>, task: &FormTask) -> Self {
        Self {
            kind: DialogKind::Task(original.map(str::to_owned)),
            fields: vec![
                field("Task 名称", original.unwrap_or(""), &[]),
                field("命令", &task.command, &[]),
                field("参数（空格分隔）", &task.args.join(" "), &[]),
                field("工作目录（可空）", task.cwd.as_deref().unwrap_or(""), &[]),
                field(
                    "环境变量（KEY=VALUE，逗号分隔）",
                    &pairs_text(&task.env),
                    &[],
                ),
                field(
                    "依赖（task:条件，逗号分隔）",
                    &dependencies_text(&task.depends_on),
                    &[],
                ),
                field(
                    "重启策略",
                    &task.restart,
                    &["never", "on-failure", "always"],
                ),
                field("重启等待毫秒", &task.restart_delay_ms.to_string(), &[]),
                field("停止超时毫秒", &task.shutdown_timeout_ms.to_string(), &[]),
            ],
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
                    &verify.map_or_else(String::new, |value| value.args.join(" ")),
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
            DialogKind::Task(Some(_)) => "编辑 Task",
            DialogKind::Task(None) => "新建 Task",
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
        let Some(position) = field.choices.iter().position(|value| *value == field.value) else {
            return;
        };
        let next = if forward {
            (position + 1) % field.choices.len()
        } else {
            position.checked_sub(1).unwrap_or(field.choices.len() - 1)
        };
        field.value = field.choices[next].to_owned();
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
    pub(crate) fn commit(&self, config: &mut FormConfig) -> Result<(), String> {
        match &self.kind {
            DialogKind::Project => {
                require(&self.fields[0].value, "项目名称")?;
                config.project.clone_from(&self.fields[0].value);
            }
            DialogKind::Task(original) => {
                let name = self.fields[0].value.trim();
                require(name, "Task 名称")?;
                let healthcheck = original
                    .as_ref()
                    .and_then(|name| config.tasks.get(name))
                    .and_then(|task| task.healthcheck.clone());
                let success_exit_codes = original
                    .as_ref()
                    .and_then(|name| config.tasks.get(name))
                    .map_or_else(|| vec![0], |task| task.success_exit_codes.clone());
                let task = FormTask {
                    command: required_value(&self.fields[1].value, "命令")?,
                    args: words(&self.fields[2].value),
                    cwd: optional(&self.fields[3].value),
                    env: parse_pairs(&self.fields[4].value, "环境变量")?,
                    healthcheck,
                    success_exit_codes,
                    depends_on: parse_dependencies(&self.fields[5].value)?,
                    restart: self.fields[6].value.clone(),
                    restart_delay_ms: parse_u64(&self.fields[7].value, "重启等待毫秒")?,
                    shutdown_timeout_ms: parse_u64(&self.fields[8].value, "停止超时毫秒")?,
                };
                replace_entry(&mut config.tasks, original.as_deref(), name, task, "Task")?;
            }
            DialogKind::Dependency(original) => {
                let name = self.fields[0].value.trim();
                require(name, "依赖名称")?;
                let command = optional(&self.fields[7].value);
                let args = words(&self.fields[8].value);
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
        Ok(())
    }
}

impl FormTask {
    /// 返回新建 Task 使用的默认值。
    fn default_value() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            healthcheck: None,
            success_exit_codes: vec![0],
            depends_on: BTreeMap::new(),
            restart: "never".to_owned(),
            restart_delay_ms: 500,
            shutdown_timeout_ms: 5_000,
        }
    }
}

impl FormDependency {
    /// 返回新建管理依赖使用的默认值。
    fn default_value() -> Self {
        Self {
            source: String::new(),
            version: String::new(),
            checksum: None,
            unpack: "auto".to_owned(),
            path: None,
            kind: "auto".to_owned(),
            verify: None,
        }
    }
}

/// 创建一个弹窗字段。
fn field(label: &'static str, value: &str, choices: &'static [&'static str]) -> DialogField {
    DialogField {
        label,
        value: value.to_owned(),
        choices,
    }
}

/// 将环境变量映射转换为弹窗文本。
fn pairs_text(values: &BTreeMap<String, String>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// 将依赖映射转换为弹窗文本。
fn dependencies_text(values: &BTreeMap<String, FormTaskDependency>) -> String {
    values
        .iter()
        .map(|(name, dependency)| format!("{name}:{}", dependency.condition))
        .collect::<Vec<_>>()
        .join(",")
}

/// 解析逗号分隔的环境变量集合。
fn parse_pairs(value: &str, label: &str) -> Result<BTreeMap<String, String>, String> {
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
fn parse_dependencies(value: &str) -> Result<BTreeMap<String, FormTaskDependency>, String> {
    value
        .split(',')
        .filter(|item| !item.trim().is_empty())
        .map(|item| {
            let (name, condition) = item.split_once(':').unwrap_or((item, "started"));
            let name = name.trim();
            require(name, "依赖 Task")?;
            let condition = condition.trim();
            if !matches!(condition, "started" | "healthy" | "completed_successfully") {
                return Err("依赖条件只能是 started、healthy 或 completed_successfully".to_owned());
            }
            Ok((
                name.to_owned(),
                FormTaskDependency {
                    condition: condition.to_owned(),
                },
            ))
        })
        .collect()
}

/// 替换或新增一个带名称的配置条目，并防止意外覆盖。
fn replace_entry<T>(
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
fn required_value(value: &str, label: &str) -> Result<String, String> {
    require(value, label)?;
    Ok(value.trim().to_owned())
}

/// 把可空文本转为可选字段。
fn optional(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_owned())
}

/// 将空白分隔的参数文本转换为参数数组。
fn words(value: &str) -> Vec<String> {
    value.split_whitespace().map(str::to_owned).collect()
}

/// 解析一个毫秒数值字段。
fn parse_u64(value: &str, label: &str) -> Result<u64, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{label} 必须是非负整数"))
}
