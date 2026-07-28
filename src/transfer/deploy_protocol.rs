//! 全托管 Service 部署使用的 SSH 行协议。

use serde::{Deserialize, Serialize};

/// SSH 全托管部署协议的当前版本。
pub(crate) const DEPLOY_PROTOCOL_VERSION: u32 = 1;

/// 客户端在部署会话开始时发送的元数据。
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct DeployInit {
    pub(crate) protocol: u32,
    pub(crate) project: String,
    /// 始终使用 `/` 分隔的跨平台 Service 相对路径。
    pub(crate) config_path: String,
    pub(crate) archive_bytes: u64,
    pub(crate) content_bytes: u64,
    pub(crate) sha256: String,
    pub(crate) timeout_ms: u64,
    pub(crate) stable_for_ms: u64,
    pub(crate) keep: u32,
}

/// 远端部署接收器可报告的确定性阶段。
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeployPhase {
    Validating,
    Activating,
    Verifying,
    RollingBack,
    Restored,
}

impl DeployPhase {
    /// 返回适合终端展示的简短阶段名。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Validating => "校验",
            Self::Activating => "切换",
            Self::Verifying => "验活",
            Self::RollingBack => "回滚",
            Self::Restored => "恢复",
        }
    }
}

/// 远端部署接收器发送的协商、进度和完成消息。
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum DeployResponse {
    Ready { project: String },
    Progress { phase: DeployPhase, message: String },
    Complete { result: DeployResult },
}

/// 一次成功部署的确定性结果。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct DeployResult {
    pub(crate) project: String,
    pub(crate) release: String,
    pub(crate) previous_release: Option<String>,
    pub(crate) content_bytes: u64,
    pub(crate) sha256: String,
}
