//! 全托管部署供 CLI 与 MCP 共享的结构化结果。

use std::path::PathBuf;

use serde::Serialize;

use crate::config::DeployPlatform;

/// 预检选中的单个平台二进制。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DeployBinaryChoice {
    pub(crate) name: String,
    pub(crate) selector: String,
    pub(crate) source: PathBuf,
    pub(crate) target: String,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
}

/// 不修改远端状态的部署预检结果。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DeployPreview {
    pub(crate) project: String,
    pub(crate) source: PathBuf,
    pub(crate) config_path: String,
    pub(crate) ssh_target: String,
    pub(crate) remote_bin: String,
    pub(crate) target_platform: DeployPlatform,
    pub(crate) binaries: Vec<DeployBinaryChoice>,
    pub(crate) content_bytes: u64,
    pub(crate) archive_bytes: u64,
    pub(crate) archive_sha256: String,
    pub(crate) timeout_ms: u64,
    pub(crate) stable_for_ms: u64,
    pub(crate) keep: u32,
    pub(crate) revision: String,
}

/// 一条可由终端即时渲染、也可由 MCP 结构化返回的部署事件。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DeployEvent {
    pub(crate) phase: String,
    pub(crate) message: String,
}

impl DeployEvent {
    /// 构造稳定阶段名与人类可读消息。
    pub(crate) fn new(phase: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            message: message.into(),
        }
    }
}

/// 成功部署及其完整预检、阶段摘要。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DeployOutcome {
    pub(crate) project: String,
    pub(crate) release: String,
    pub(crate) previous_release: Option<String>,
    pub(crate) preview: DeployPreview,
    pub(crate) events: Vec<DeployEvent>,
}
