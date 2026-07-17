use std::{collections::BTreeMap, path::Path};

use serde::Serialize;

use crate::{
    config::{TaskDefaultsSpec, ValueOrigin},
    core::RestartPolicy,
};

use super::{
    config_form::{FormConfig, FormTask},
    config_form_defaults::form_path,
    config_form_dialog::{
        DialogField, choice_field, field, map_text, optional, parse_i32_list, parse_map, parse_u32,
        parse_u64, required_value,
    },
};

/// 结构化编辑器中的项目级 Task 默认声明。
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct FormTaskDefaults {
    /// 默认工作目录。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    /// 默认 Task 环境。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) env: BTreeMap<String, String>,
    /// 默认成功退出码。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) success_exit_codes: Option<Vec<i32>>,
    /// 默认重启策略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) restart: Option<String>,
    /// 默认重启等待毫秒。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) restart_delay_ms: Option<u64>,
    /// 默认最大连续重启次数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_restarts: Option<u32>,
    /// 默认连续重启计数重置窗口。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) restart_reset_after_ms: Option<u64>,
    /// 默认优雅停止等待时间。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) shutdown_timeout_ms: Option<u64>,
}

impl FormTaskDefaults {
    /// 从规范化声明构造可移植的表单值。
    pub(super) fn from_spec(spec: TaskDefaultsSpec, base_directory: Option<&Path>) -> Self {
        Self {
            cwd: spec.cwd.map(|path| form_path(&path, base_directory)),
            env: spec.env,
            success_exit_codes: spec
                .success_exit_codes
                .map(|values| values.into_iter().collect()),
            restart: spec.restart.map(restart_text).map(str::to_owned),
            restart_delay_ms: spec.restart_delay_ms,
            max_restarts: spec.max_restarts,
            restart_reset_after_ms: spec.restart_reset_after_ms,
            shutdown_timeout_ms: spec.shutdown_timeout_ms,
        }
    }

    /// 判断项目是否实际声明了任一默认字段。
    pub(crate) fn is_empty(&self) -> bool {
        self.cwd.is_none()
            && self.env.is_empty()
            && self.success_exit_codes.is_none()
            && self.restart.is_none()
            && self.restart_delay_ms.is_none()
            && self.max_restarts.is_none()
            && self.restart_reset_after_ms.is_none()
            && self.shutdown_timeout_ms.is_none()
    }

    /// 返回项目详情区使用的紧凑声明摘要。
    pub(crate) fn summary(&self) -> String {
        let mut fields = Vec::new();
        if self.cwd.is_some() {
            fields.push("cwd".to_owned());
        }
        if !self.env.is_empty() {
            fields.push(format!("env({})", self.env.len()));
        }
        if self.success_exit_codes.is_some() {
            fields.push("成功退出码".to_owned());
        }
        if self.restart.is_some() {
            fields.push("重启策略".to_owned());
        }
        if self.restart_delay_ms.is_some() {
            fields.push("重启等待".to_owned());
        }
        if self.max_restarts.is_some() {
            fields.push("重启上限".to_owned());
        }
        if self.restart_reset_after_ms.is_some() {
            fields.push("计数重置".to_owned());
        }
        if self.shutdown_timeout_ms.is_some() {
            fields.push("停止超时".to_owned());
        }
        if fields.is_empty() {
            "未配置".to_owned()
        } else {
            fields.join("、")
        }
    }

    /// 把当前默认层应用到没有 Task 显式声明的表单有效值。
    pub(super) fn apply_to(&self, task: &mut FormTask) {
        inherited(task, "cwd", self.cwd.clone(), None, |task, value| {
            task.cwd = value;
        });
        inherited(
            task,
            "success_exit_codes",
            self.success_exit_codes.clone(),
            Some(vec![0]),
            |task, value| task.success_exit_codes = value.expect("退出码有内建默认"),
        );
        inherited(
            task,
            "restart",
            self.restart.clone(),
            Some("never".to_owned()),
            |task, value| task.restart = value.expect("重启策略有内建默认"),
        );
        inherited(
            task,
            "restart_delay_ms",
            self.restart_delay_ms,
            Some(500),
            |task, value| task.restart_delay_ms = value.expect("重启等待有内建默认"),
        );
        inherited(
            task,
            "max_restarts",
            self.max_restarts,
            Some(0),
            |task, value| task.max_restarts = value.expect("重启上限有内建默认"),
        );
        inherited(
            task,
            "restart_reset_after_ms",
            self.restart_reset_after_ms,
            Some(60_000),
            |task, value| task.restart_reset_after_ms = value.expect("重置窗口有内建默认"),
        );
        inherited(
            task,
            "shutdown_timeout_ms",
            self.shutdown_timeout_ms,
            Some(5_000),
            |task, value| task.shutdown_timeout_ms = value.expect("停止超时有内建默认"),
        );
    }

    /// 把默认声明追加到手写 YAML 输出。
    #[allow(clippy::format_push_string)]
    pub(super) fn append_yaml(&self, text: &mut String) {
        if self.is_empty() {
            return;
        }
        text.push_str("task_defaults:\n");
        optional_yaml(text, "cwd", self.cwd.as_deref());
        if !self.env.is_empty() {
            text.push_str("  env:\n");
            for (key, value) in &self.env {
                text.push_str(&format!("    {}: {}\n", quoted(key), quoted(value)));
            }
        }
        if let Some(values) = &self.success_exit_codes {
            text.push_str("  success_exit_codes:\n");
            for value in values {
                text.push_str(&format!("    - {value}\n"));
            }
        }
        optional_scalar(text, "restart", self.restart.as_deref());
        optional_number(text, "restart_delay_ms", self.restart_delay_ms);
        optional_number(text, "max_restarts", self.max_restarts);
        optional_number(text, "restart_reset_after_ms", self.restart_reset_after_ms);
        optional_number(text, "shutdown_timeout_ms", self.shutdown_timeout_ms);
    }
}

impl FormConfig {
    /// 替换项目默认层并刷新所有未由 Task 显式声明的有效值。
    pub(super) fn replace_task_defaults(&mut self, defaults: FormTaskDefaults) {
        self.task_defaults = defaults;
        for task in self.tasks.values_mut() {
            self.task_defaults.apply_to(task);
        }
    }

    /// 创建已经应用当前项目默认层的新 Task 草稿。
    pub(super) fn new_task_value(&self) -> FormTask {
        let mut task = FormTask::default_value();
        self.task_defaults.apply_to(&mut task);
        task
    }
}

/// 构造项目弹窗中基础信息与 Task 默认值字段。
pub(super) fn project_fields(config: &FormConfig) -> Vec<DialogField> {
    let mut fields = vec![
        field("项目名称", &config.project, &[]),
        field(
            "默认环境变量（JSON 对象或 KEY=VALUE）",
            &map_text(&config.env),
            &[],
        ),
        field(
            "Task 默认工作目录（可空）",
            config.task_defaults.cwd.as_deref().unwrap_or(""),
            &[],
        ),
        field(
            "Task 默认环境（JSON 对象或 KEY=VALUE）",
            &map_text(&config.task_defaults.env),
            &[],
        ),
        field(
            "Task 默认成功退出码（可空）",
            &config
                .task_defaults
                .success_exit_codes
                .as_ref()
                .map(|values| serde_json::to_string(values).expect("整数数组序列化不会失败"))
                .unwrap_or_default(),
            &[],
        ),
        field(
            "Task 默认重启策略",
            config.task_defaults.restart.as_deref().unwrap_or("default"),
            &["default", "never", "on-failure", "always"],
        ),
        field(
            "Task 默认重启等待毫秒（可空）",
            &option_text(config.task_defaults.restart_delay_ms),
            &[],
        ),
        field(
            "Task 默认最大重启次数（可空）",
            &option_text(config.task_defaults.max_restarts),
            &[],
        ),
        field(
            "Task 默认计数重置毫秒（可空）",
            &option_text(config.task_defaults.restart_reset_after_ms),
            &[],
        ),
        field(
            "Task 默认停止超时毫秒（可空）",
            &option_text(config.task_defaults.shutdown_timeout_ms),
            &[],
        ),
    ];
    let mut profiles = vec!["none".to_owned()];
    profiles.extend(config.profile_names().cloned());
    fields.push(choice_field(
        "活动 profile",
        config.active_profile().unwrap_or("none"),
        profiles,
    ));
    fields
}

/// 校验并提交项目基础信息与 Task 默认层。
pub(super) fn commit_project(
    fields: &[DialogField],
    config: &mut FormConfig,
) -> Result<(), String> {
    config.project = required_value(&fields[0].value, "项目名称")?;
    let profile = (fields[10].value != "none").then(|| fields[10].value.clone());
    if let Some(name) = profile.as_deref()
        && !config.has_profile(name)
    {
        return Err(format!("profile `{name}` 不存在"));
    }
    config.active_profile = profile;
    config.env = parse_map(&fields[1].value, "默认环境变量")?;
    let defaults = FormTaskDefaults {
        cwd: optional(&fields[2].value),
        env: parse_map(&fields[3].value, "Task 默认环境")?,
        success_exit_codes: optional(&fields[4].value)
            .map(|value| parse_i32_list(&value, "Task 默认成功退出码"))
            .transpose()?,
        restart: (fields[5].value != "default").then(|| fields[5].value.clone()),
        restart_delay_ms: parse_optional_u64(&fields[6].value, "Task 默认重启等待毫秒")?,
        max_restarts: parse_optional_u32(&fields[7].value, "Task 默认最大重启次数")?,
        restart_reset_after_ms: parse_optional_u64(&fields[8].value, "Task 默认计数重置毫秒")?,
        shutdown_timeout_ms: parse_optional_u64(&fields[9].value, "Task 默认停止超时毫秒")?,
    };
    config.replace_task_defaults(defaults);
    Ok(())
}

/// 未被 Task 显式声明的字段在项目默认与内建默认之间切换。
fn inherited<T>(
    task: &mut FormTask,
    field: &str,
    configured: Option<T>,
    built_in: Option<T>,
    assign: impl FnOnce(&mut FormTask, Option<T>),
) {
    if matches!(
        task.origins.field(field),
        ValueOrigin::Task | ValueOrigin::TaskTemplate | ValueOrigin::Profile
    ) {
        return;
    }
    let origin = if configured.is_some() {
        ValueOrigin::TaskDefaults
    } else {
        ValueOrigin::BuiltIn
    };
    assign(task, configured.or(built_in));
    task.origins.fields.insert(field.to_owned(), origin);
}

/// 解析可空的 64 位无符号整数。
fn parse_optional_u64(value: &str, label: &str) -> Result<Option<u64>, String> {
    optional(value)
        .map(|value| parse_u64(&value, label))
        .transpose()
}

/// 解析可空的 32 位无符号整数。
fn parse_optional_u32(value: &str, label: &str) -> Result<Option<u32>, String> {
    optional(value)
        .map(|value| parse_u32(&value, label))
        .transpose()
}

/// 把可选数字转换为弹窗文本。
fn option_text(value: Option<impl ToString>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

/// 把重启策略转为配置拼写。
const fn restart_text(value: RestartPolicy) -> &'static str {
    match value {
        RestartPolicy::Never => "never",
        RestartPolicy::OnFailure => "on-failure",
        RestartPolicy::Always => "always",
    }
}

/// 输出安全双引号字符串。
fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("字符串序列化不会失败")
}

/// 输出可选 YAML 字符串。
#[allow(clippy::format_push_string)]
fn optional_yaml(text: &mut String, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        text.push_str(&format!("  {key}: {}\n", quoted(value)));
    }
}

/// 输出无需引号的受控枚举值。
#[allow(clippy::format_push_string)]
fn optional_scalar(text: &mut String, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        text.push_str(&format!("  {key}: {value}\n"));
    }
}

/// 输出可选 YAML 数字。
#[allow(clippy::format_push_string)]
fn optional_number(text: &mut String, key: &str, value: Option<impl std::fmt::Display>) {
    if let Some(value) = value {
        text.push_str(&format!("  {key}: {value}\n"));
    }
}
