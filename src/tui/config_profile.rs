use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    config_form::FormConfig,
    config_form_dialog::{
        DialogField, choice_field, field, map_text, optional, parse_args, parse_duration,
        parse_i32_list, parse_map, parse_u32, required_value,
    },
    config_task_defaults::FormTaskDefaults,
};

/// 结构化编辑器保留的命名 profile 本地声明。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct FormProfile {
    /// 可选基础 profile。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) extends: Option<String>,
    /// 显式 Task 准入白名单；`None` 表示继承或准入全部。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tasks: Option<Vec<String>>,
    /// 覆盖项目环境的键。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) env: BTreeMap<String, String>,
    /// 覆盖项目 Task 默认层的字段。
    #[serde(default, skip_serializing_if = "FormTaskDefaults::is_empty")]
    pub(crate) task_defaults: FormTaskDefaults,
}

impl FormConfig {
    /// 返回 profile 名称迭代器。
    pub(crate) fn profile_names(&self) -> impl Iterator<Item = &String> {
        self.profiles.keys()
    }

    /// 返回 profile 声明迭代器。
    pub(crate) fn profiles(&self) -> impl Iterator<Item = (&String, &FormProfile)> {
        self.profiles.iter()
    }

    /// 判断命名 profile 是否存在。
    pub(crate) fn has_profile(&self, name: &str) -> bool {
        self.profiles.contains_key(name)
    }

    /// 返回命名 profile 数量。
    pub(crate) fn profile_count(&self) -> usize {
        self.profiles.len()
    }

    /// 替换 profile 声明，并在重命名时同步显式引用。
    pub(crate) fn replace_profile(
        &mut self,
        original: Option<&str>,
        name: String,
        profile: FormProfile,
    ) {
        if let Some(original) = original {
            self.profiles.remove(original);
            for candidate in self.profiles.values_mut() {
                if candidate.extends.as_deref() == Some(original) {
                    candidate.extends = Some(name.clone());
                }
            }
            if self.active_profile.as_deref() == Some(original) {
                self.active_profile = Some(name.clone());
            }
        }
        self.profiles.insert(name, profile);
    }

    /// 返回直接继承指定 profile 的声明名称。
    pub(crate) fn profile_dependents(&self, name: &str) -> Vec<String> {
        self.profiles
            .iter()
            .filter(|(_, profile)| profile.extends.as_deref() == Some(name))
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// 删除没有继承者的 profile，并让活动选择回到基础配置。
    pub(crate) fn remove_profile(&mut self, name: &str) {
        self.profiles.remove(name);
        if self.active_profile.as_deref() == Some(name) {
            self.active_profile = None;
        }
    }

    /// 判断活动或未准入声明中是否存在指定 Task。
    pub(crate) fn has_task_declaration(&self, name: &str) -> bool {
        self.tasks.contains_key(name) || self.inactive_tasks.contains_key(name)
    }
}

impl FormProfile {
    /// 返回 profile 列表使用的紧凑说明。
    pub(crate) fn summary(&self) -> String {
        let base = self.extends.as_deref().unwrap_or("基础配置");
        let tasks = self.tasks.as_ref().map_or_else(
            || "全部/继承".to_owned(),
            |tasks| format!("{} 个 Task", tasks.len()),
        );
        format!("继承 {base} · {tasks}")
    }

    /// 返回详情区使用的完整声明摘要。
    pub(crate) fn detail(&self, name: &str) -> String {
        let tasks = self.tasks.as_ref().map_or_else(
            || "未声明（继承或全部）".to_owned(),
            |tasks| {
                if tasks.is_empty() {
                    "空白名单".to_owned()
                } else {
                    tasks.join(", ")
                }
            },
        );
        format!(
            "名称：{name}\n继承：{}\nTask 白名单：{tasks}\n环境覆盖：{} 项\nTask 默认覆盖：{}",
            self.extends.as_deref().unwrap_or("未配置"),
            self.env.len(),
            self.task_defaults.summary()
        )
    }
}

/// 构造新建或编辑 profile 的字段集合。
pub(super) fn fields(
    original: Option<&str>,
    profile: &FormProfile,
    config: &FormConfig,
) -> Vec<DialogField> {
    let mut bases = vec!["none".to_owned()];
    bases.extend(
        config
            .profile_names()
            .filter(|name| Some(name.as_str()) != original)
            .cloned(),
    );
    vec![
        field("profile 名称", original.unwrap_or(""), &[]),
        choice_field(
            "继承 profile",
            profile.extends.as_deref().unwrap_or("none"),
            bases,
        ),
        field(
            "Task 白名单（空=继承/全部，JSON []=无 Task）",
            &profile
                .tasks
                .as_ref()
                .map(|tasks| serde_json::to_string(tasks).expect("字符串数组可序列化"))
                .unwrap_or_default(),
            &[],
        ),
        field(
            "环境覆盖（JSON 对象或 KEY=VALUE）",
            &map_text(&profile.env),
            &[],
        ),
        field(
            "Task 默认工作目录（可空）",
            profile.task_defaults.cwd.as_deref().unwrap_or(""),
            &[],
        ),
        field(
            "Task 默认环境（JSON 对象或 KEY=VALUE）",
            &map_text(&profile.task_defaults.env),
            &[],
        ),
        field(
            "Task 默认成功退出码（可空）",
            &profile
                .task_defaults
                .success_exit_codes
                .as_ref()
                .map(|values| serde_json::to_string(values).expect("整数数组可序列化"))
                .unwrap_or_default(),
            &[],
        ),
        choice_field(
            "Task 默认重启策略",
            profile
                .task_defaults
                .restart
                .as_deref()
                .unwrap_or("default"),
            ["default", "never", "on-failure", "always"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        ),
        field(
            "Task 默认重启等待（如 750ms/5s，可空）",
            &option_duration(profile.task_defaults.restart_delay_ms),
            &[],
        ),
        field(
            "Task 默认最大重启次数（可空）",
            &option_text(profile.task_defaults.max_restarts),
            &[],
        ),
        field(
            "Task 默认计数重置（如 1m，可空）",
            &option_duration(profile.task_defaults.restart_reset_after_ms),
            &[],
        ),
        field(
            "Task 默认停止超时（如 5s，可空）",
            &option_duration(profile.task_defaults.shutdown_timeout_ms),
            &[],
        ),
    ]
}

/// 校验 profile 字段并以不展开继承值的方式更新表单声明。
pub(super) fn commit(
    original: Option<&str>,
    fields: &[DialogField],
    config: &mut FormConfig,
) -> Result<(), String> {
    let name = required_value(&fields[0].value, "profile 名称")?;
    if !valid_name(&name) {
        return Err("profile 名称只能包含 ASCII 字母、数字、点、短横线和下划线".to_owned());
    }
    if original != Some(name.as_str()) && config.has_profile(&name) {
        return Err(format!("profile 名称 `{name}` 已存在"));
    }
    let extends = (fields[1].value != "none").then(|| fields[1].value.clone());
    if extends.as_deref() == Some(name.as_str()) {
        return Err("profile 不能继承自身".to_owned());
    }
    validate_inheritance(original, &name, extends.as_deref(), config)?;
    let tasks = optional(&fields[2].value)
        .map(|value| parse_args(&value, "Task 白名单"))
        .transpose()?;
    validate_tasks(tasks.as_deref(), config)?;
    let profile = FormProfile {
        extends,
        tasks,
        env: parse_map(&fields[3].value, "环境覆盖")?,
        task_defaults: FormTaskDefaults {
            cwd: optional(&fields[4].value),
            env: parse_map(&fields[5].value, "Task 默认环境")?,
            success_exit_codes: optional(&fields[6].value)
                .map(|value| parse_i32_list(&value, "Task 默认成功退出码"))
                .transpose()?,
            restart: (fields[7].value != "default").then(|| fields[7].value.clone()),
            restart_delay_ms: parse_optional_duration(&fields[8].value, "Task 默认重启等待")?,
            max_restarts: parse_optional_u32(&fields[9].value, "Task 默认最大重启次数")?,
            restart_reset_after_ms: parse_optional_duration(
                &fields[10].value,
                "Task 默认计数重置",
            )?,
            shutdown_timeout_ms: parse_optional_duration(&fields[11].value, "Task 默认停止超时")?,
        },
    };
    config.replace_profile(original, name, profile);
    Ok(())
}

/// 在修改表单前验证继承关系不会通过其他 profile 回到自身。
fn validate_inheritance(
    original: Option<&str>,
    name: &str,
    base: Option<&str>,
    config: &FormConfig,
) -> Result<(), String> {
    let mut seen = BTreeSet::from([name]);
    let mut current = base;
    while let Some(candidate) = current {
        let normalized = if Some(candidate) == original {
            name
        } else {
            candidate
        };
        if !seen.insert(normalized) {
            return Err(format!(
                "profile 继承形成循环：{}",
                seen.into_iter().collect::<Vec<_>>().join(" -> ")
            ));
        }
        current = config
            .profiles
            .get(candidate)
            .and_then(|profile| profile.extends.as_deref());
    }
    Ok(())
}

/// 校验白名单没有重复或未知 Task。
fn validate_tasks(tasks: Option<&[String]>, config: &FormConfig) -> Result<(), String> {
    let Some(tasks) = tasks else {
        return Ok(());
    };
    let mut seen = BTreeSet::new();
    for task in tasks {
        if !seen.insert(task) {
            return Err(format!("Task `{task}` 在白名单中重复出现"));
        }
        if !config.has_task_declaration(task) {
            return Err(format!("Task 白名单引用了不存在的 Task `{task}`"));
        }
    }
    Ok(())
}

/// 判断名称是否符合配置模型的稳定标识规则。
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// 解析可空带单位时长。
fn parse_optional_duration(value: &str, label: &str) -> Result<Option<u64>, String> {
    optional(value)
        .map(|value| parse_duration(&value, label))
        .transpose()
}

/// 解析可空计数值。
fn parse_optional_u32(value: &str, label: &str) -> Result<Option<u32>, String> {
    optional(value)
        .map(|value| parse_u32(&value, label))
        .transpose()
}

/// 把可空数值写成表单文本。
fn option_text<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

/// 把可空时长写成表单文本。
fn option_duration(value: Option<u64>) -> String {
    value.map_or_else(String::new, crate::config::format_duration)
}
