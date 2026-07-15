use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use procora_core::{
    DependencyCondition, DependencySpec, ProjectSpec, RestartPolicy, ServiceName, TaskId, TaskSpec,
};
use serde::Deserialize;

use crate::ConfigDiagnostic;

/// 配置前端反序列化使用的原始项目 DTO。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProject {
    version: Option<u32>,
    project: Option<String>,
    #[serde(default)]
    tasks: BTreeMap<String, RawTask>,
}

/// 配置前端反序列化使用的原始 Task DTO。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTask {
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<PathBuf>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    depends_on: BTreeMap<String, RawDependency>,
    #[serde(default)]
    restart: RawRestartPolicy,
    #[serde(default = "default_restart_delay_ms")]
    restart_delay_ms: u64,
    #[serde(default = "default_shutdown_timeout_ms")]
    shutdown_timeout_ms: u64,
}

/// 原始配置中的依赖边 DTO。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDependency {
    #[serde(default)]
    condition: RawDependencyCondition,
}

/// 原始配置支持的依赖条件拼写。
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawDependencyCondition {
    #[default]
    Started,
    Healthy,
    CompletedSuccessfully,
}

/// 原始配置支持的重启策略拼写。
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RawRestartPolicy {
    #[default]
    Never,
    OnFailure,
    Always,
}

impl RawProject {
    /// 校验并规范化原始 DTO，独立错误尽量一次返回。
    pub(crate) fn normalize(
        self,
        base_directory: Option<&Path>,
    ) -> Result<ProjectSpec, Vec<ConfigDiagnostic>> {
        let mut diagnostics = Vec::new();
        let version = self.version.unwrap_or_else(|| {
            diagnostics.push(diagnostic("version", "缺少必需字段"));
            0
        });
        if version != 0 && version != 1 {
            diagnostics.push(diagnostic(
                "version",
                format!("不支持版本 {version}，当前只支持版本 1"),
            ));
        }
        let project = normalize_project(self.project, &mut diagnostics);
        let valid_ids = self
            .tasks
            .keys()
            .filter_map(|value| value.parse::<TaskId>().ok())
            .collect::<BTreeSet<_>>();
        let mut tasks = BTreeMap::new();
        for (raw_id, raw_task) in self.tasks {
            let path = format!("tasks.{raw_id}");
            let Ok(task_id) = raw_id.parse::<TaskId>() else {
                diagnostics.push(diagnostic(&path, "Task ID 包含非法字符"));
                continue;
            };
            let task = normalize_task(
                raw_task,
                &path,
                base_directory,
                &valid_ids,
                &mut diagnostics,
            );
            tasks.insert(task_id, task);
        }
        if diagnostics.is_empty() {
            Ok(ProjectSpec {
                version,
                project,
                tasks,
            })
        } else {
            Err(diagnostics)
        }
    }
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
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> TaskSpec {
    let command = raw.command.unwrap_or_else(|| {
        diagnostics.push(diagnostic(format!("{path}.command"), "缺少必需字段"));
        String::new()
    });
    if command.trim().is_empty() {
        diagnostics.push(diagnostic(format!("{path}.command"), "命令不能为空"));
    }
    if raw.shutdown_timeout_ms == 0 {
        diagnostics.push(diagnostic(
            format!("{path}.shutdown_timeout_ms"),
            "必须大于零",
        ));
    } else if raw.shutdown_timeout_ms > MAX_SHUTDOWN_TIMEOUT_MS {
        diagnostics.push(diagnostic(
            format!("{path}.shutdown_timeout_ms"),
            format!("不能超过 {MAX_SHUTDOWN_TIMEOUT_MS} 毫秒"),
        ));
    }
    if raw.restart_delay_ms == 0 {
        diagnostics.push(diagnostic(format!("{path}.restart_delay_ms"), "必须大于零"));
    } else if raw.restart_delay_ms > MAX_RESTART_DELAY_MS {
        diagnostics.push(diagnostic(
            format!("{path}.restart_delay_ms"),
            format!("不能超过 {MAX_RESTART_DELAY_MS} 毫秒"),
        ));
    }
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
    TaskSpec {
        command,
        args: raw.args,
        cwd: raw.cwd.map(|path| normalize_path(&path, base_directory)),
        env: raw.env,
        depends_on,
        restart: raw.restart.into(),
        restart_delay_ms: raw.restart_delay_ms,
        shutdown_timeout_ms: raw.shutdown_timeout_ms,
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

/// 默认重启退避毫秒数。
const fn default_restart_delay_ms() -> u64 {
    500
}

/// 默认停止宽限毫秒数。
const fn default_shutdown_timeout_ms() -> u64 {
    5_000
}

/// 配置允许的最大自动重启基础退避时间。
const MAX_RESTART_DELAY_MS: u64 = 30_000;

/// 配置允许的最大单 Task 优雅停止时间。
const MAX_SHUTDOWN_TIMEOUT_MS: u64 = 300_000;

impl From<RawDependencyCondition> for DependencyCondition {
    fn from(value: RawDependencyCondition) -> Self {
        match value {
            RawDependencyCondition::Started => Self::Started,
            RawDependencyCondition::Healthy => Self::Healthy,
            RawDependencyCondition::CompletedSuccessfully => Self::CompletedSuccessfully,
        }
    }
}

impl From<RawRestartPolicy> for RestartPolicy {
    fn from(value: RawRestartPolicy) -> Self {
        match value {
            RawRestartPolicy::Never => Self::Never,
            RawRestartPolicy::OnFailure => Self::OnFailure,
            RawRestartPolicy::Always => Self::Always,
        }
    }
}
