use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use crate::core::{DependencySpec, ServiceName, TaskId, TaskSpec};
use serde::{Deserialize, Serialize};

use super::ConfigDiagnostic;
use super::health::normalize_healthcheck;
use super::{DependencyVerifySpec, ManagedDependencies, ManagedDependencySpec};
pub(crate) use profile::RawProfile;
use restart::{
    default_restart_delay_ms, default_restart_reset_after_ms, default_shutdown_timeout_ms,
};
use task::RawDependencyCondition;
pub(crate) use task::RawTask;

mod command;
mod conversions;
mod declarations;
mod env_file;
mod merge;
mod profile;
mod project_normalize;
mod restart;
mod task;
mod task_defaults;
mod task_templates;
mod variables;

/// 为结构化编辑器复用与配置编译完全一致的命令文本切分。
pub(crate) fn split_command_text(value: &str) -> Result<(String, Vec<String>), String> {
    command::split_text(value)
}

/// 配置前端反序列化使用的原始项目 DTO。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProject {
    #[serde(default)]
    include: Vec<PathBuf>,
    version: Option<u32>,
    project: Option<String>,
    /// 当前持久选择的命名运行场景。
    profile: Option<String>,
    #[serde(default)]
    profiles: BTreeMap<String, profile::RawProfile>,
    /// 可在显式支持字段中通过 `${vars.NAME}` 引用的项目变量。
    #[serde(default)]
    vars: BTreeMap<String, String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    task_defaults: task_defaults::RawTaskDefaults,
    #[serde(default)]
    task_templates: BTreeMap<String, RawTask>,
    #[serde(default)]
    dependencies: BTreeMap<String, RawManagedDependency>,
    #[serde(default)]
    tasks: BTreeMap<String, RawTask>,
    #[serde(skip)]
    task_declarations: BTreeMap<String, RawTask>,
    #[serde(skip)]
    task_template_sources: BTreeMap<String, task_templates::TemplateSources>,
    #[serde(skip)]
    declared_env: BTreeMap<String, String>,
    #[serde(skip)]
    declared_task_defaults: task_defaults::RawTaskDefaults,
    #[serde(skip)]
    profile_sources: profile::ProfileSources,
    #[serde(skip)]
    admitted_tasks: Option<BTreeSet<String>>,
    #[serde(skip)]
    resolved_vars: BTreeMap<String, String>,
    #[serde(skip)]
    variable_references: BTreeMap<String, BTreeSet<String>>,
    #[serde(skip)]
    declared_profiles: BTreeMap<String, profile::RawProfile>,
    #[serde(skip)]
    declared_task_templates: BTreeMap<String, RawTask>,
    #[serde(skip)]
    declared_tasks: BTreeMap<String, RawTask>,
}

/// 配置前端反序列化使用的项目依赖 DTO。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManagedDependency {
    source: Option<String>,
    version: Option<String>,
    checksum: Option<String>,
    #[serde(default)]
    unpack: RawUnpackMode,
    path: Option<PathBuf>,
    #[serde(default)]
    kind: RawDependencyKind,
    verify: Option<RawDependencyVerify>,
}

/// 配置前端反序列化使用的版本验证 DTO。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDependencyVerify {
    command: Option<PathBuf>,
    #[serde(default)]
    args: Vec<String>,
    contains: Option<String>,
}

/// 原始依赖内容类型。
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawDependencyKind {
    #[default]
    Auto,
    Binary,
    File,
    Directory,
}

/// 原始依赖解包模式。
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawUnpackMode {
    #[default]
    Auto,
    Never,
}

/// 原始配置支持的重启策略拼写。
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum RawRestartPolicy {
    #[default]
    Never,
    OnFailure,
    Always,
}

/// 校验并规范化全部项目级管理依赖。
fn normalize_dependencies(
    raw_dependencies: BTreeMap<String, RawManagedDependency>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> ManagedDependencies {
    let mut dependencies = BTreeMap::new();
    for (id, raw) in raw_dependencies {
        let field = format!("dependencies.{id}");
        if !valid_dependency_id(&id) {
            diagnostics.push(diagnostic(
                &field,
                "依赖名称只能包含 ASCII 字母、数字、点、短横线和下划线",
            ));
            continue;
        }
        let source = required_text(raw.source, &format!("{field}.source"), diagnostics);
        if !source.is_empty() && !valid_source(&source) {
            diagnostics.push(diagnostic(
                format!("{field}.source"),
                "只支持 http://、https://、ssh://、SCP 地址、file:// 或本地路径",
            ));
        }
        let version = required_text(raw.version, &format!("{field}.version"), diagnostics);
        if matches!(version.as_str(), "." | "..")
            || version.contains(['/', '\\'])
            || version.chars().any(char::is_control)
        {
            diagnostics.push(diagnostic(
                format!("{field}.version"),
                "不能包含路径分隔符、控制字符或父目录",
            ));
        }
        if let Some(checksum) = raw.checksum.as_deref()
            && !valid_checksum(checksum)
        {
            diagnostics.push(diagnostic(
                format!("{field}.checksum"),
                "必须是 64 位十六进制 SHA-256，可带 sha256: 前缀",
            ));
        }
        if raw
            .path
            .as_ref()
            .is_some_and(|path| !valid_relative_path(path))
        {
            diagnostics.push(diagnostic(
                format!("{field}.path"),
                "必须是归档内不含父目录的相对路径",
            ));
        }
        let verify = raw.verify.map(|verify| DependencyVerifySpec {
            command: verify.command,
            args: verify.args,
            contains: verify.contains,
        });
        if verify
            .as_ref()
            .and_then(|verify| verify.command.as_ref())
            .is_some_and(|path| !valid_relative_path(path))
        {
            diagnostics.push(diagnostic(
                format!("{field}.verify.command"),
                "必须是安装根目录内不含父目录的相对路径",
            ));
        }
        dependencies.insert(
            id,
            ManagedDependencySpec {
                source,
                version,
                checksum: raw.checksum,
                unpack: raw.unpack.into(),
                path: raw.path,
                kind: raw.kind.into(),
                verify,
            },
        );
    }
    dependencies
}

/// 读取必需的非空文本字段。
fn required_text(
    value: Option<String>,
    path: &str,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> String {
    let Some(value) = value else {
        diagnostics.push(diagnostic(path, "缺少必需字段"));
        return String::new();
    };
    if value.trim().is_empty() {
        diagnostics.push(diagnostic(path, "不能为空"));
    }
    value
}

/// 判断依赖名称能否稳定用于占位符和目录。
fn valid_dependency_id(value: &str) -> bool {
    !matches!(value, "" | "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// 判断来源是否属于支持的网络、SSH 或本地形式。
fn valid_source(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("ssh://")
        || value.starts_with("file://")
        || (!value.contains("://")
            && value
                .split_once(':')
                .is_some_and(|(host, path)| !host.contains('/') && path.starts_with('/')))
        || !value.contains("://")
}

/// 判断 SHA-256 字符串格式是否合法。
fn valid_checksum(value: &str) -> bool {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// 判断配置路径是否为不含点或父目录分量的相对路径。
fn valid_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

/// 校验并返回项目稳定名称文本。
fn normalize_project(project: Option<String>, diagnostics: &mut Vec<ConfigDiagnostic>) -> String {
    let Some(project) = project else {
        diagnostics.push(diagnostic("project", "缺少必需字段"));
        return String::new();
    };
    if project.trim().is_empty() {
        diagnostics.push(diagnostic("project", "项目标识不能为空"));
    } else if let Err(error) = project.parse::<ServiceName>() {
        diagnostics.push(diagnostic("project", error.to_string()));
    }
    project
}

/// 把单个原始 Task 转为领域规范。
fn normalize_task(
    raw: RawTask,
    path: &str,
    base_directory: Option<&Path>,
    valid_ids: &BTreeSet<TaskId>,
    project_env: &BTreeMap<String, String>,
    validate_runtime_limits: bool,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> TaskSpec {
    if validate_runtime_limits {
        raw.validate_runtime_limits(path, diagnostics);
    }
    let (command, args) = command::normalize(raw.command, raw.args, path, diagnostics);
    let mut depends_on = BTreeMap::new();
    for (raw_dependency, dependency) in raw.depends_on {
        let dependency_path = format!("{path}.depends_on.{raw_dependency}");
        match raw_dependency.parse::<TaskId>() {
            Ok(task_id) if valid_ids.contains(&task_id) => {
                depends_on.insert(
                    task_id,
                    DependencySpec {
                        condition: dependency.condition.into(),
                    },
                );
            }
            Ok(_) => diagnostics.push(diagnostic(dependency_path, "依赖的 Task 不存在")),
            Err(error) => diagnostics.push(diagnostic(dependency_path, error.to_string())),
        }
    }
    let mut success_exit_codes = raw
        .success_exit_codes
        .unwrap_or_default()
        .into_iter()
        .filter(|code| {
            if *code < 0 {
                diagnostics.push(diagnostic(
                    format!("{path}.success_exit_codes"),
                    "退出码不能为负数",
                ));
                false
            } else {
                true
            }
        })
        .collect::<BTreeSet<_>>();
    success_exit_codes.insert(0);
    let mut env = project_env.clone();
    env.extend(raw.env);
    TaskSpec {
        command,
        args,
        cwd: raw.cwd.map(|path| normalize_path(&path, base_directory)),
        env,
        healthcheck: normalize_healthcheck(raw.healthcheck, path, base_directory, diagnostics),
        success_exit_codes,
        depends_on,
        restart: raw.restart.unwrap_or_default().into(),
        restart_delay_ms: raw
            .restart_delay_ms
            .unwrap_or_else(default_restart_delay_ms),
        max_restarts: raw.max_restarts.unwrap_or_default(),
        restart_reset_after_ms: raw
            .restart_reset_after_ms
            .unwrap_or_else(default_restart_reset_after_ms),
        shutdown_timeout_ms: raw
            .shutdown_timeout_ms
            .unwrap_or_else(default_shutdown_timeout_ms),
    }
}

/// 按配置文件目录解析相对路径，并消除点与父目录分量。
fn normalize_path(path: &Path, base_directory: Option<&Path>) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_directory.map_or_else(|| path.to_path_buf(), |base| base.join(path))
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// 创建一条字段级配置诊断。
fn diagnostic(path: impl Into<String>, message: impl Into<String>) -> ConfigDiagnostic {
    ConfigDiagnostic {
        path: path.into(),
        message: message.into(),
    }
}
