use std::collections::BTreeMap;

use serde::Serialize;

use crate::{
    config::{CompiledProject, ConfigFormat, DependencyKind, UnpackMode},
    core::{DependencyCondition, RestartPolicy},
};

/// 结构化编辑页当前聚焦的配置区域。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FormPane {
    /// 项目基础信息。
    Project,
    /// Task 列表。
    Tasks,
    /// 管理依赖列表。
    Dependencies,
}

/// 表单可编辑的完整配置文档。
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FormConfig {
    /// 配置格式版本。
    version: u32,
    /// 项目名称。
    pub(crate) project: String,
    /// 管理依赖集合。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) dependencies: BTreeMap<String, FormDependency>,
    /// Task 集合。
    pub(crate) tasks: BTreeMap<String, FormTask>,
}

/// 表单中的 Task 值对象。
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FormTask {
    /// 程序路径或名称。
    pub(crate) command: String,
    /// 程序参数。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) args: Vec<String>,
    /// 工作目录。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    /// 环境变量。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) env: BTreeMap<String, String>,
    /// 上游 Task 依赖。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) depends_on: BTreeMap<String, FormTaskDependency>,
    /// 重启策略。
    pub(crate) restart: String,
    /// 重启前等待时间。
    pub(crate) restart_delay_ms: u64,
    /// 优雅停止等待时间。
    pub(crate) shutdown_timeout_ms: u64,
}

/// 表单中的单条 Task 依赖。
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FormTaskDependency {
    /// 依赖就绪条件。
    pub(crate) condition: String,
}

/// 表单中的管理依赖值对象。
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FormDependency {
    /// 下载或本地来源。
    pub(crate) source: String,
    /// 固定版本。
    pub(crate) version: String,
    /// 可选 SHA-256。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) checksum: Option<String>,
    /// 解包策略。
    pub(crate) unpack: String,
    /// 归档内相对路径。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    /// 最终内容类型。
    pub(crate) kind: String,
    /// 可选验证规则。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) verify: Option<FormVerify>,
}

/// 表单中的依赖版本验证规则。
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FormVerify {
    /// 验证程序。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<String>,
    /// 验证参数。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) args: Vec<String>,
    /// 预期输出片段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) contains: Option<String>,
}

impl FormConfig {
    /// 从已校验的配置构造不会丢失已支持字段的表单模型。
    pub(crate) fn from_compiled(compiled: CompiledProject) -> Self {
        let tasks = compiled
            .spec
            .tasks
            .into_iter()
            .map(|(id, task)| {
                let depends_on = task
                    .depends_on
                    .into_iter()
                    .map(|(id, dependency)| {
                        (
                            id.to_string(),
                            FormTaskDependency {
                                condition: condition_text(dependency.condition).to_owned(),
                            },
                        )
                    })
                    .collect();
                (
                    id.to_string(),
                    FormTask {
                        command: task.command,
                        args: task.args,
                        cwd: task.cwd.map(|path| path.display().to_string()),
                        env: task.env,
                        depends_on,
                        restart: restart_text(task.restart).to_owned(),
                        restart_delay_ms: task.restart_delay_ms,
                        shutdown_timeout_ms: task.shutdown_timeout_ms,
                    },
                )
            })
            .collect();
        let dependencies = compiled
            .dependencies
            .into_iter()
            .map(|(id, dependency)| {
                let verify = dependency.verify.map(|verify| FormVerify {
                    command: verify.command.map(|path| path.display().to_string()),
                    args: verify.args,
                    contains: verify.contains,
                });
                (
                    id,
                    FormDependency {
                        source: dependency.source,
                        version: dependency.version,
                        checksum: dependency.checksum,
                        unpack: unpack_text(dependency.unpack).to_owned(),
                        path: dependency.path.map(|path| path.display().to_string()),
                        kind: kind_text(dependency.kind).to_owned(),
                        verify,
                    },
                )
            })
            .collect();
        Self {
            version: compiled.spec.version,
            project: compiled.spec.project,
            dependencies,
            tasks,
        }
    }

    /// 按当前目标格式输出规范化配置文本。
    pub(crate) fn text(&self, format: ConfigFormat) -> Result<String, String> {
        match format {
            ConfigFormat::Json => serde_json::to_string_pretty(self)
                .map(|text| format!("{text}\n"))
                .map_err(|error| error.to_string()),
            ConfigFormat::Toml => toml::to_string_pretty(self)
                .map(|text| format!("{text}\n"))
                .map_err(|error| error.to_string()),
            ConfigFormat::Yaml => Ok(self.yaml()),
        }
    }

    /// 返回项目名称。
    pub(crate) fn project(&self) -> &str {
        &self.project
    }

    /// 返回 Task 迭代器。
    pub(crate) fn tasks(&self) -> impl Iterator<Item = (&String, &FormTask)> {
        self.tasks.iter()
    }

    /// 返回管理依赖迭代器。
    pub(crate) fn dependencies(&self) -> impl Iterator<Item = (&String, &FormDependency)> {
        self.dependencies.iter()
    }

    /// 使用易读且所有字符串均安全转义的 YAML 输出配置。
    #[allow(clippy::format_push_string)]
    fn yaml(&self) -> String {
        let mut text = format!(
            "version: {}\nproject: {}\n",
            self.version,
            quoted(&self.project)
        );
        if !self.dependencies.is_empty() {
            text.push_str("dependencies:\n");
            for (id, dependency) in &self.dependencies {
                text.push_str(&format!("  {}:\n", quoted(id)));
                text.push_str(&format!("    source: {}\n", quoted(&dependency.source)));
                text.push_str(&format!("    version: {}\n", quoted(&dependency.version)));
                optional_yaml(&mut text, 4, "checksum", dependency.checksum.as_deref());
                text.push_str(&format!("    unpack: {}\n", dependency.unpack));
                optional_yaml(&mut text, 4, "path", dependency.path.as_deref());
                text.push_str(&format!("    kind: {}\n", dependency.kind));
                if let Some(verify) = &dependency.verify {
                    text.push_str("    verify:\n");
                    optional_yaml(&mut text, 6, "command", verify.command.as_deref());
                    yaml_array(&mut text, 6, "args", &verify.args);
                    optional_yaml(&mut text, 6, "contains", verify.contains.as_deref());
                }
            }
        }
        text.push_str("tasks:\n");
        for (id, task) in &self.tasks {
            text.push_str(&format!("  {}:\n", quoted(id)));
            text.push_str(&format!("    command: {}\n", quoted(&task.command)));
            yaml_array(&mut text, 4, "args", &task.args);
            optional_yaml(&mut text, 4, "cwd", task.cwd.as_deref());
            if !task.env.is_empty() {
                text.push_str("    env:\n");
                for (key, value) in &task.env {
                    text.push_str(&format!("      {}: {}\n", quoted(key), quoted(value)));
                }
            }
            if !task.depends_on.is_empty() {
                text.push_str("    depends_on:\n");
                for (name, dependency) in &task.depends_on {
                    text.push_str(&format!(
                        "      {}:\n        condition: {}\n",
                        quoted(name),
                        dependency.condition
                    ));
                }
            }
            text.push_str(&format!("    restart: {}\n", task.restart));
            text.push_str(&format!(
                "    restart_delay_ms: {}\n",
                task.restart_delay_ms
            ));
            text.push_str(&format!(
                "    shutdown_timeout_ms: {}\n",
                task.shutdown_timeout_ms
            ));
        }
        text
    }
}

/// 输出 JSON 风格安全双引号字符串，YAML 同样支持该转义形式。
fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("字符串序列化不会失败")
}

/// 输出可选 YAML 字符串字段。
#[allow(clippy::format_push_string)]
fn optional_yaml(text: &mut String, indent: usize, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        text.push_str(&format!(
            "{}{}: {}\n",
            " ".repeat(indent),
            key,
            quoted(value)
        ));
    }
}

/// 输出 YAML 字符串数组字段。
#[allow(clippy::format_push_string)]
fn yaml_array(text: &mut String, indent: usize, key: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    text.push_str(&format!("{}{}:\n", " ".repeat(indent), key));
    for value in values {
        text.push_str(&format!("{}- {}\n", " ".repeat(indent + 2), quoted(value)));
    }
}

/// 将重启策略转为配置中的拼写。
const fn restart_text(value: RestartPolicy) -> &'static str {
    match value {
        RestartPolicy::Never => "never",
        RestartPolicy::OnFailure => "on-failure",
        RestartPolicy::Always => "always",
    }
}

/// 将依赖条件转为配置中的拼写。
const fn condition_text(value: DependencyCondition) -> &'static str {
    match value {
        DependencyCondition::Started => "started",
        DependencyCondition::Healthy => "healthy",
        DependencyCondition::CompletedSuccessfully => "completed_successfully",
    }
}

/// 将解包策略转为配置中的拼写。
const fn unpack_text(value: UnpackMode) -> &'static str {
    match value {
        UnpackMode::Auto => "auto",
        UnpackMode::Never => "never",
    }
}

/// 将依赖内容类型转为配置中的拼写。
const fn kind_text(value: DependencyKind) -> &'static str {
    match value {
        DependencyKind::Auto => "auto",
        DependencyKind::Binary => "binary",
        DependencyKind::File => "file",
        DependencyKind::Directory => "directory",
    }
}
