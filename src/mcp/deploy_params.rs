//! MCP 两阶段部署的参数与非交互式设置转换。

use std::path::PathBuf;

use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

/// 裸机部署预检参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct DeployPreviewParams {
    /// 本机 Service 目录或显式声明式配置文件。
    #[serde(default = "current_directory")]
    pub(super) source: PathBuf,
    /// SSH config 别名或`[user@]host`，MCP不进行交互询问。
    pub(super) ssh: String,
    /// 远端Procora命令名或不含空格的路径。
    #[serde(default)]
    pub(super) remote_bin: Option<String>,
    /// 可选的project名称断言，防止部署错Service。
    #[serde(default)]
    pub(super) service: Option<String>,
    /// 等待新release可用的最长毫秒数。
    #[serde(default = "default_deploy_timeout")]
    pub(super) timeout_ms: u64,
    /// release持续可用后才确认成功的毫秒数。
    #[serde(default = "default_stable_window")]
    pub(super) stable_for_ms: u64,
    /// 远端保留的最近release数量，范围1到32。
    #[serde(default = "default_release_keep")]
    pub(super) keep: u32,
    /// 显式允许执行可信的 Python 配置。
    #[serde(default)]
    pub(super) allow_python: bool,
}

/// 经过预检修订确认的裸机部署参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct DeployParams {
    /// 本机 Service 目录或显式声明式配置文件。
    #[serde(default = "current_directory")]
    pub(super) source: PathBuf,
    /// SSH config 别名或`[user@]host`，只允许非交互式登录。
    pub(super) ssh: String,
    /// `preview_deploy`返回的完整revision。
    pub(super) revision: String,
    /// 远端Procora命令名或不含空格的路径。
    #[serde(default)]
    pub(super) remote_bin: Option<String>,
    /// 可选的project名称断言，防止部署错Service。
    #[serde(default)]
    pub(super) service: Option<String>,
    /// 必须与预检相同的验收超时毫秒数。
    #[serde(default = "default_deploy_timeout")]
    pub(super) timeout_ms: u64,
    /// 必须与预检相同的稳定窗口毫秒数。
    #[serde(default = "default_stable_window")]
    pub(super) stable_for_ms: u64,
    /// 必须与预检相同的release保留数量。
    #[serde(default = "default_release_keep")]
    pub(super) keep: u32,
    /// 显式允许执行可信的 Python 配置。
    #[serde(default)]
    pub(super) allow_python: bool,
}

/// 构造 MCP 固定为非交互式认证的共享部署输入。
#[allow(clippy::too_many_arguments)]
pub(super) fn settings(
    source: PathBuf,
    ssh: String,
    remote_bin: Option<String>,
    service: Option<String>,
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
) -> crate::cli::api::deploy::DeploySettings {
    crate::cli::api::deploy::DeploySettings {
        source,
        ssh_target: Some(ssh),
        remote_bin,
        expected_service: service,
        timeout_ms,
        stable_for_ms,
        keep,
        batch: true,
    }
}

/// MCP 省略 source 时与 CLI 一致使用当前目录。
fn current_directory() -> PathBuf {
    PathBuf::from(".")
}

/// MCP 默认部署验收超时。
fn default_deploy_timeout() -> u64 {
    30_000
}

/// MCP 默认部署稳定窗口。
fn default_stable_window() -> u64 {
    2_000
}

/// MCP 默认 release 保留数量。
fn default_release_keep() -> u32 {
    3
}
