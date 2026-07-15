use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 用户和配置共同决定的任务期望状态。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredState {
    /// 引擎应维持任务运行。
    Running,
    /// 引擎应确保任务停止。
    Stopped,
}

/// 任务进程的观测状态。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedState {
    /// 任务等待调度。
    Pending,
    /// 任务被未满足的条件阻断。
    Blocked,
    /// 任务正在启动。
    Starting,
    /// 任务进程已经运行。
    Running,
    /// 任务正在停止。
    Stopping,
    /// 任务已经退出。
    Exited,
    /// 任务已经失败。
    Failed,
    /// 任务正在等待重启退避结束。
    Backoff,
    /// 任务可能存在遗留进程但所有权无法确认。
    Orphaned,
}

/// 与进程状态正交的健康状态。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    /// 尚未得到健康信息。
    Unknown,
    /// 健康检查正在达到成功阈值。
    Starting,
    /// 健康检查已经通过。
    Healthy,
    /// 健康检查已经达到失败阈值。
    Unhealthy,
    /// 任务没有配置健康检查。
    NotConfigured,
}

/// 引擎保存的单任务运行时状态。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskRuntimeState {
    /// 当前期望状态。
    pub desired: DesiredState,
    /// 当前观测状态。
    pub observed: ObservedState,
    /// 当前健康状态。
    pub health: HealthState,
    /// 当前配置运行代次。
    pub generation: u64,
    /// 当前进程运行身份；没有活动进程时为空。
    pub run_id: Option<Uuid>,
    /// 最近一次退出码；被信号终止或未退出时为空。
    pub exit_code: Option<i32>,
    /// 当前 generation 内连续自动重启次数。
    pub restart_attempt: u32,
}

impl Default for TaskRuntimeState {
    fn default() -> Self {
        Self {
            desired: DesiredState::Running,
            observed: ObservedState::Pending,
            health: HealthState::NotConfigured,
            generation: 1,
            run_id: None,
            exit_code: None,
            restart_attempt: 0,
        }
    }
}
