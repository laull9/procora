use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

use super::TaskId;

/// Task 的进程外就绪检查配置。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthCheckSpec {
    /// 不经过 shell 解释的检查程序。
    pub command: String,
    /// 直接传递给检查程序的参数。
    #[serde(default)]
    pub args: Vec<String>,
    /// 检查程序独立的工作目录；缺省时继承 Task 工作目录。
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// Task 创建后首次检查前的等待毫秒数。
    #[serde(default)]
    pub initial_delay_ms: u64,
    /// 两次检查完成之间的等待毫秒数。
    #[serde(default = "default_health_period_ms")]
    pub period_ms: u64,
    /// 单次检查超过该时间后回收整个检查进程树。
    #[serde(default = "default_health_timeout_ms")]
    pub timeout_ms: u64,
    /// 连续成功多少次后标记为健康。
    #[serde(default = "default_health_success_threshold")]
    pub success_threshold: u32,
    /// 连续失败多少次后标记为不健康。
    #[serde(default = "default_health_failure_threshold")]
    pub failure_threshold: u32,
}

/// Task 退出后的自动重启策略。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    /// 任何退出都不自动重启。
    #[default]
    Never,
    /// 仅启动失败或非零退出时重启。
    OnFailure,
    /// 只要服务期望运行就始终重启。
    Always,
}

/// 依赖边需要上游达到的条件。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyCondition {
    /// 上游进程已经成功启动。
    #[default]
    Started,
    /// 上游健康检查已经通过。
    Healthy,
    /// 上游一次性任务已经成功结束。
    CompletedSuccessfully,
}

/// 单条任务依赖规范。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DependencySpec {
    /// 依赖需要满足的条件。
    #[serde(default)]
    pub condition: DependencyCondition,
}

/// 单个可执行任务的规范化配置。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    /// 要执行的程序名或路径。
    pub command: String,
    /// 不经过 shell 解释的参数数组。
    #[serde(default)]
    pub args: Vec<String>,
    /// 任务独立的工作目录。
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// 传递给任务的环境变量覆盖项。
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// 可选的进程外健康检查。
    #[serde(default)]
    pub healthcheck: Option<HealthCheckSpec>,
    /// 视为成功的退出码集合；规范化后始终包含 0。
    #[serde(default = "default_success_exit_codes")]
    pub success_exit_codes: BTreeSet<i32>,
    /// 以任务标识为键的依赖集合。
    #[serde(default)]
    pub depends_on: BTreeMap<TaskId, DependencySpec>,
    /// Task 退出后的自动重启策略。
    #[serde(default)]
    pub restart: RestartPolicy,
    /// 自动重启前的基础退避毫秒数。
    #[serde(default = "default_restart_delay_ms")]
    pub restart_delay_ms: u64,
    /// 优雅停止后等待强制回收的毫秒数。
    #[serde(default = "default_shutdown_timeout_ms")]
    pub shutdown_timeout_ms: u64,
}

/// 一个项目的规范化任务集合。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSpec {
    /// 配置模式主版本。
    pub version: u32,
    /// 项目稳定标识。
    pub project: String,
    /// 以稳定标识为键的任务集合。
    pub tasks: BTreeMap<TaskId, TaskSpec>,
}

/// 默认自动重启基础退避。
const fn default_restart_delay_ms() -> u64 {
    500
}

/// 默认优雅停止宽限期。
const fn default_shutdown_timeout_ms() -> u64 {
    5_000
}

/// 默认只把退出码 0 视为成功。
fn default_success_exit_codes() -> BTreeSet<i32> {
    BTreeSet::from([0])
}

/// 默认健康检查周期。
const fn default_health_period_ms() -> u64 {
    10_000
}

/// 默认健康检查超时。
const fn default_health_timeout_ms() -> u64 {
    1_000
}

/// 默认健康检查成功阈值。
const fn default_health_success_threshold() -> u32 {
    1
}

/// 默认健康检查失败阈值。
const fn default_health_failure_threshold() -> u32 {
    3
}
