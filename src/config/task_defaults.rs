use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use serde::Serialize;

use crate::core::RestartPolicy;

/// 项目级 Task 默认声明；未声明字段继续使用模式内建默认。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct TaskDefaultsSpec {
    /// Task 未声明时采用的工作目录。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// 合并到每个 Task 本地环境之前的共享环境。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Task 未声明时采用的成功退出码集合。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_exit_codes: Option<BTreeSet<i32>>,
    /// Task 未声明时采用的重启策略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart: Option<RestartPolicy>,
    /// Task 未声明时采用的基础重启退避。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_delay_ms: Option<u64>,
    /// Task 未声明时采用的连续自动重启上限。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_restarts: Option<u32>,
    /// Task 未声明时采用的连续重启计数重置窗口。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_reset_after_ms: Option<u64>,
    /// Task 未声明时采用的优雅停止等待时间。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shutdown_timeout_ms: Option<u64>,
}

impl TaskDefaultsSpec {
    /// 判断项目是否实际声明了任一 Task 默认值。
    pub fn is_empty(&self) -> bool {
        self.cwd.is_none()
            && self.env.is_empty()
            && self.success_exit_codes.is_none()
            && self.restart.is_none()
            && self.restart_delay_ms.is_none()
            && self.max_restarts.is_none()
            && self.restart_reset_after_ms.is_none()
            && self.shutdown_timeout_ms.is_none()
    }
}
