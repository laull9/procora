use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::core::HealthCheckSpec;

use super::ConfigDiagnostic;

/// 配置前端反序列化使用的原始健康检查 DTO。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawHealthCheck {
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<PathBuf>,
    #[serde(default)]
    initial_delay_ms: u64,
    #[serde(default = "default_period_ms")]
    period_ms: u64,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_success_threshold")]
    success_threshold: u32,
    #[serde(default = "default_failure_threshold")]
    failure_threshold: u32,
}

impl RawHealthCheck {
    /// 把 include 片段内的相对工作目录改写为声明文件目录路径。
    pub(super) fn rebase(&mut self, base: &Path) {
        self.cwd = self
            .cwd
            .take()
            .map(|path| normalize_path(&path, Some(base)));
    }
}

/// 校验并规范化可选健康检查。
pub(super) fn normalize_healthcheck(
    raw: Option<RawHealthCheck>,
    task_path: &str,
    base_directory: Option<&Path>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<HealthCheckSpec> {
    let raw = raw?;
    let path = format!("{task_path}.healthcheck");
    let command = raw.command.unwrap_or_else(|| {
        diagnostics.push(diagnostic(format!("{path}.command"), "缺少必需字段"));
        String::new()
    });
    if command.trim().is_empty() {
        diagnostics.push(diagnostic(format!("{path}.command"), "命令不能为空"));
    }
    bounded_duration(raw.period_ms, &format!("{path}.period_ms"), diagnostics);
    bounded_duration(raw.timeout_ms, &format!("{path}.timeout_ms"), diagnostics);
    if raw.initial_delay_ms > MAX_DURATION_MS {
        diagnostics.push(diagnostic(
            format!("{path}.initial_delay_ms"),
            format!("不能超过 {MAX_DURATION_MS} 毫秒"),
        ));
    }
    bounded_threshold(
        raw.success_threshold,
        &format!("{path}.success_threshold"),
        diagnostics,
    );
    bounded_threshold(
        raw.failure_threshold,
        &format!("{path}.failure_threshold"),
        diagnostics,
    );
    Some(HealthCheckSpec {
        command,
        args: raw.args,
        cwd: raw.cwd.map(|path| normalize_path(&path, base_directory)),
        initial_delay_ms: raw.initial_delay_ms,
        period_ms: raw.period_ms,
        timeout_ms: raw.timeout_ms,
        success_threshold: raw.success_threshold,
        failure_threshold: raw.failure_threshold,
    })
}

/// 校验非零且有上限的探针时长。
fn bounded_duration(value: u64, path: &str, diagnostics: &mut Vec<ConfigDiagnostic>) {
    if value == 0 {
        diagnostics.push(diagnostic(path, "必须大于零"));
    } else if value > MAX_DURATION_MS {
        diagnostics.push(diagnostic(path, format!("不能超过 {MAX_DURATION_MS} 毫秒")));
    }
}

/// 校验连续结果阈值，防止无界等待。
fn bounded_threshold(value: u32, path: &str, diagnostics: &mut Vec<ConfigDiagnostic>) {
    if value == 0 {
        diagnostics.push(diagnostic(path, "必须大于零"));
    } else if value > MAX_THRESHOLD {
        diagnostics.push(diagnostic(path, format!("不能超过 {MAX_THRESHOLD}")));
    }
}

/// 按配置目录规范化健康检查工作目录。
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

/// 创建健康检查字段诊断。
fn diagnostic(path: impl Into<String>, message: impl Into<String>) -> ConfigDiagnostic {
    ConfigDiagnostic {
        path: path.into(),
        message: message.into(),
    }
}

/// 默认检查周期。
const fn default_period_ms() -> u64 {
    10_000
}

/// 默认检查超时。
const fn default_timeout_ms() -> u64 {
    1_000
}

/// 默认连续成功阈值。
const fn default_success_threshold() -> u32 {
    1
}

/// 默认连续失败阈值。
const fn default_failure_threshold() -> u32 {
    3
}

/// 单次时长配置上限。
const MAX_DURATION_MS: u64 = 300_000;

/// 连续结果阈值上限。
const MAX_THRESHOLD: u32 = 100;
