use std::{collections::BTreeMap, path::Path};

use serde::Serialize;

use crate::{
    config::{CompiledProject, ConfigFormat, DependencyKind, TaskConfigOrigins, UnpackMode},
    core::DependencyCondition,
};

use super::{
    config_form_defaults::{form_path, restart_text},
    config_health_dialog,
    config_task_defaults::FormTaskDefaults,
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
#[derive(Clone, Debug)]
pub(crate) struct FormConfig {
    /// 配置格式版本。
    pub(crate) version: u32,
    /// 项目名称。
    pub(crate) project: String,
    /// 当前持久选择的运行 profile。
    pub(crate) active_profile: Option<String>,
    /// 命名 profile 原始声明，保存时不展开有效值。
    pub(crate) profiles: BTreeMap<String, serde_json::Value>,
    /// 合并到各 Task 前的项目级默认环境。
    pub(crate) env: BTreeMap<String, String>,
    /// 应用到未显式声明对应字段的所有 Task。
    pub(crate) task_defaults: FormTaskDefaults,
    /// 可由 Task 显式引用且不会在保存时展开的命名模板声明。
    pub(crate) task_templates: BTreeMap<String, serde_json::Value>,
    /// 管理依赖集合。
    pub(crate) dependencies: BTreeMap<String, FormDependency>,
    /// Task 集合。
    pub(crate) tasks: BTreeMap<String, FormTask>,
    /// 当前 profile 未准入但保存时必须原样保留的 Task。
    pub(crate) inactive_tasks: BTreeMap<String, serde_json::Value>,
}

/// 表单中的 Task 值对象。
#[derive(Clone, Debug)]
pub(crate) struct FormTask {
    /// 显式引用的命名模板。
    pub(crate) extends: Option<String>,
    /// 程序路径或名称。
    pub(crate) command: String,
    /// 程序参数。
    pub(crate) args: Vec<String>,
    /// 工作目录。
    pub(crate) cwd: Option<String>,
    /// 显式环境文件路径。
    pub(crate) env_file: Option<String>,
    /// 环境变量。
    pub(crate) env: BTreeMap<String, String>,
    /// 可选健康检查，可由结构化表单直接编辑。
    pub(crate) healthcheck: Option<FormHealthCheck>,
    /// 视为成功的退出码。
    pub(crate) success_exit_codes: Vec<i32>,
    /// 上游 Task 依赖。
    pub(crate) depends_on: BTreeMap<String, FormTaskDependency>,
    /// 重启策略。
    pub(crate) restart: String,
    /// 重启前等待时间。
    pub(crate) restart_delay_ms: u64,
    /// 连续自动重启次数上限；0 表示无限。
    pub(crate) max_restarts: u32,
    /// 连续重启计数的稳定运行重置窗口。
    pub(crate) restart_reset_after_ms: u64,
    /// 优雅停止等待时间。
    pub(crate) shutdown_timeout_ms: u64,
    /// 有效值来源，用于解释配置并避免把内建默认展开写回。
    pub(crate) origins: TaskConfigOrigins,
}

/// 表单序列化时保留的健康检查配置。
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FormHealthCheck {
    /// exec 检查程序；HTTP 探针时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<String>,
    /// 检查参数。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) args: Vec<String>,
    /// 可选工作目录。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    /// 可选 HTTP GET 探针。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) http_get: Option<FormHttpHealthCheck>,
    /// 首次检查等待时间。
    pub(crate) initial_delay_ms: u64,
    /// 检查周期。
    pub(crate) period_ms: u64,
    /// 单次检查超时。
    pub(crate) timeout_ms: u64,
    /// 连续成功阈值。
    pub(crate) success_threshold: u32,
    /// 连续失败阈值。
    pub(crate) failure_threshold: u32,
}

/// 表单序列化时保留的 HTTP GET 探针。
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FormHttpHealthCheck {
    /// HTTP 或 HTTPS。
    pub(crate) scheme: String,
    /// 目标主机。
    pub(crate) host: String,
    /// 可选目标端口。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
    /// 请求路径。
    pub(crate) path: String,
    /// 有界请求头。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) headers: BTreeMap<String, String>,
    /// 预期状态码。
    pub(crate) status_code: u16,
}

/// 表单中的单条 Task 依赖。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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
    pub(crate) fn from_compiled(compiled: CompiledProject, base_directory: Option<&Path>) -> Self {
        let project_env = compiled.declared_project_env;
        let task_defaults =
            FormTaskDefaults::from_spec(compiled.declared_task_defaults, base_directory);
        let active_profile = compiled.active_profile;
        let profiles = compiled
            .profiles
            .into_iter()
            .map(|(name, profile)| {
                let mut value = serde_json::to_value(profile).expect("Raw profile 可序列化");
                relativize_profile_paths(&mut value, base_directory);
                (name, value)
            })
            .collect();
        let task_templates = compiled
            .task_templates
            .into_iter()
            .map(|(name, template)| {
                let mut value = serde_json::to_value(template).expect("Raw Task 模板可序列化");
                relativize_template_paths(&mut value, base_directory);
                (name, value)
            })
            .collect();
        let mut task_extends = compiled.task_extends;
        let mut task_env_files = compiled.task_env_files;
        let mut task_inline_env = compiled.task_inline_env;
        let mut task_origins = compiled.task_origins;
        let inactive_tasks = compiled
            .inactive_tasks
            .into_iter()
            .map(|(name, task)| {
                let mut value = serde_json::to_value(task).expect("Raw Task 可序列化");
                relativize_task_paths(&mut value, base_directory);
                (name, value)
            })
            .collect();
        let tasks = compiled
            .spec
            .tasks
            .into_iter()
            .map(|(id, task)| {
                let healthcheck = task.healthcheck.map(config_health_dialog::from_spec);
                let success_exit_codes = task.success_exit_codes.into_iter().collect();
                let env = task_inline_env.remove(&id).unwrap_or_default();
                let env_file = task_env_files
                    .remove(&id)
                    .map(|path| form_path(&path, base_directory));
                let origins = task_origins.remove(&id).unwrap_or_default();
                let depends_on = task
                    .depends_on
                    .into_iter()
                    .filter(|(id, _)| {
                        origins.depends_on.get(&id.to_string())
                            == Some(&crate::config::ValueOrigin::Task)
                    })
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
                        extends: task_extends.remove(&id),
                        command: task.command,
                        args: task.args,
                        cwd: task.cwd.map(|path| form_path(&path, base_directory)),
                        env_file,
                        env,
                        healthcheck,
                        success_exit_codes,
                        depends_on,
                        restart: restart_text(task.restart).to_owned(),
                        restart_delay_ms: task.restart_delay_ms,
                        max_restarts: task.max_restarts,
                        restart_reset_after_ms: task.restart_reset_after_ms,
                        shutdown_timeout_ms: task.shutdown_timeout_ms,
                        origins,
                    },
                )
            })
            .collect();
        let dependencies = form_dependencies(compiled.dependencies);
        Self {
            version: compiled.spec.version,
            project: compiled.spec.project,
            active_profile,
            profiles,
            env: project_env,
            task_defaults,
            task_templates,
            dependencies,
            tasks,
            inactive_tasks,
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

    /// 返回当前持久选择的 profile。
    pub(crate) fn active_profile(&self) -> Option<&str> {
        self.active_profile.as_deref()
    }

    /// 返回 profile 名称迭代器。
    pub(crate) fn profile_names(&self) -> impl Iterator<Item = &String> {
        self.profiles.keys()
    }

    /// 判断命名 profile 是否存在。
    pub(crate) fn has_profile(&self, name: &str) -> bool {
        self.profiles.contains_key(name)
    }

    /// 返回命名 profile 数量。
    pub(crate) fn profile_count(&self) -> usize {
        self.profiles.len()
    }

    /// 返回 Task 迭代器。
    pub(crate) fn tasks(&self) -> impl Iterator<Item = (&String, &FormTask)> {
        self.tasks.iter()
    }

    /// 判断命名模板是否存在。
    pub(crate) fn has_template(&self, name: &str) -> bool {
        self.task_templates.contains_key(name)
    }

    /// 返回命名模板数量。
    pub(crate) fn template_count(&self) -> usize {
        self.task_templates.len()
    }

    /// 返回管理依赖迭代器。
    pub(crate) fn dependencies(&self) -> impl Iterator<Item = (&String, &FormDependency)> {
        self.dependencies.iter()
    }
}

/// 把已规范化的项目管理依赖转换为可编辑表单值。
fn form_dependencies(
    dependencies: crate::config::ManagedDependencies,
) -> BTreeMap<String, FormDependency> {
    dependencies
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
        .collect()
}

/// 把模板声明中的绝对运行路径还原为相对当前入口的可移植写法。
fn relativize_template_paths(value: &mut serde_json::Value, base_directory: Option<&Path>) {
    relativize_task_paths(value, base_directory);
}

/// 把 profile 默认工作目录还原为相对入口的可移植写法。
fn relativize_profile_paths(value: &mut serde_json::Value, base_directory: Option<&Path>) {
    relativize_pointer(value, "/task_defaults/cwd", base_directory);
}

/// 把 Task 声明中的运行路径还原为相对入口的可移植写法。
fn relativize_task_paths(value: &mut serde_json::Value, base_directory: Option<&Path>) {
    for pointer in ["/cwd", "/env_file", "/healthcheck/cwd"] {
        relativize_pointer(value, pointer, base_directory);
    }
}

/// 还原一个存在的 JSON 路径字段。
fn relativize_pointer(value: &mut serde_json::Value, pointer: &str, base_directory: Option<&Path>) {
    let Some(path) = value.pointer(pointer).and_then(serde_json::Value::as_str) else {
        return;
    };
    let relative = form_path(Path::new(path), base_directory);
    *value.pointer_mut(pointer).expect("路径字段仍存在") = serde_json::Value::String(relative);
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
