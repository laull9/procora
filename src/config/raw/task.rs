use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::{RawRestartPolicy, command::RawCommand};
use crate::config::health::RawHealthCheck;

/// 配置前端反序列化使用的原始 Task DTO。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawTask {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) extends: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) command: Option<RawCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) env: BTreeMap<String, String>,
    #[serde(skip, default)]
    pub(super) inline_env_before_file: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) env_file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) healthcheck: Option<RawHealthCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) success_exit_codes: Option<Vec<i32>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) depends_on: BTreeMap<String, RawDependency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) restart: Option<RawRestartPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) restart_delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_restarts: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) restart_reset_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) shutdown_timeout_ms: Option<u64>,
}

/// 原始配置中的依赖边 DTO。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawDependency {
    #[serde(default)]
    pub(super) condition: RawDependencyCondition,
}

/// 原始配置支持的依赖条件拼写。
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RawDependencyCondition {
    /// 上游进程已创建。
    #[default]
    Started,
    /// 上游达到健康阈值。
    Healthy,
    /// 上游以成功退出码结束。
    CompletedSuccessfully,
}
