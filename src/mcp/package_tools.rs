//! MCP 的 Procora 包参数、构建与 release 管理实现。

use std::path::PathBuf;

use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

use crate::cli::api;

/// 单个 Procora 包路径参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct PackagePathParams {
    /// 本机 `.pcpkg` 文件。
    pub(super) package: PathBuf,
}

/// 包构建参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct PackageBuildParams {
    /// Service 目录或显式配置入口。
    #[serde(default = "current_directory")]
    source: PathBuf,
    /// 输出 `.pcpkg`；省略时使用当前目录的 `<service>.pcpkg`。
    #[serde(default)]
    output: Option<PathBuf>,
    /// `all`、`current` 或具体平台键。
    #[serde(default = "all_platforms")]
    platform: String,
    /// 安全替换内容不同的已有普通包。
    #[serde(default)]
    force: bool,
    /// 显式允许执行可信的 Python 配置。
    #[serde(default)]
    allow_python: bool,
}

/// 包解压参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct PackageExtractParams {
    /// 本机 `.pcpkg` 文件。
    package: PathBuf,
    /// 必须尚不存在的目标目录。
    output: PathBuf,
    /// `current` 或具体平台键。
    #[serde(default = "current_platform_name")]
    platform: String,
}

/// 包安装参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct PackageInstallParams {
    /// 本机 `.pcpkg` 文件。
    package: PathBuf,
    /// 验活最长毫秒数。
    #[serde(default = "default_deploy_timeout")]
    timeout_ms: u64,
    /// 持续可用确认窗口毫秒数。
    #[serde(default = "default_stable_window")]
    stable_for_ms: u64,
    /// 最近 release 保留数量。
    #[serde(default = "default_release_keep")]
    keep: u32,
    /// 显式允许执行包内可信的 Python 配置。
    #[serde(default)]
    allow_python: bool,
}

/// 已安装包 Service 的回滚或恢复参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct InstalledPackageParams {
    /// 稳定 Service 名称。
    service: String,
    /// 回滚目标 release；省略时选择最近一个非活动 release。
    #[serde(default)]
    release: Option<String>,
    /// 验活最长毫秒数。
    #[serde(default = "default_deploy_timeout")]
    timeout_ms: u64,
    /// 持续可用确认窗口毫秒数。
    #[serde(default = "default_stable_window")]
    stable_for_ms: u64,
}

/// 已安装包解除参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct UninstallPackageParams {
    /// 稳定 Service 名称。
    service: String,
    /// 同时永久清理包与 release 数据。
    #[serde(default)]
    purge: bool,
}

/// 构建 MCP 请求的确定性包。
pub(super) fn build_package(
    params: PackageBuildParams,
) -> anyhow::Result<crate::package::PackageBuildResult> {
    let source = super::trusted_path(params.source, params.allow_python)?;
    let discovered = crate::config::discover_path(&source)?;
    let output = if let Some(output) = params.output {
        super::absolute_path(output)?
    } else {
        crate::platform::current_dir()?.join(format!("{}.pcpkg", discovered.compiled.spec.project))
    };
    let platform = package_platform(&params.platform)?;
    if params.force {
        crate::package::build_replacing(&source, &output, platform)
    } else {
        crate::package::build(&source, &output, platform)
    }
}

/// 验证并物化 MCP 指定的包平台。
pub(super) fn extract_package(
    params: PackageExtractParams,
) -> anyhow::Result<crate::package::PackageExtractResult> {
    let package = super::absolute_path(params.package)?;
    let output = super::absolute_path(params.output)?;
    crate::package::extract(&package, &output, target_platform(&params.platform)?)
}

/// 安装包并把阶段事件随结果一起返回。
pub(super) fn install_package(params: PackageInstallParams) -> anyhow::Result<serde_json::Value> {
    let package = super::trusted_path(params.package, params.allow_python)?;
    let mut events = Vec::new();
    let mut reporter = |event: &crate::transfer::DeployEvent| events.push(event.clone());
    let outcome = crate::transfer::install_package(
        &package,
        params.timeout_ms,
        params.stable_for_ms,
        params.keep,
        &mut reporter,
    )?;
    Ok(serde_json::json!({ "outcome": outcome, "events": events }))
}

/// 返回可直接序列化的已安装包目录。
pub(super) fn installed_packages() -> anyhow::Result<serde_json::Value> {
    let catalog = crate::package::installed_catalog(&api::managed_services_root()?)?;
    let services = catalog
        .services
        .into_iter()
        .map(installed_service_json)
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "services": services }))
}

/// 把一个已安装 Service 转换为稳定 JSON。
fn installed_service_json(service: crate::package::InstalledService) -> serde_json::Value {
    serde_json::json!({
        "project": service.project,
        "root": service.root,
        "active_release": service.active_release,
        "pending_release": service.pending_release,
        "error": service.error,
        "releases": service.releases.into_iter().map(|release| serde_json::json!({
            "id": release.id,
            "sha256": release.sha256,
            "config_path": release.config_path,
            "target_platform": release.target_platform,
            "deployed_at_ms": release.deployed_at_ms,
            "active": release.active,
            "pending": release.pending,
        })).collect::<Vec<_>>(),
        "packages": service.packages.into_iter().map(|package| serde_json::json!({
            "path": package.path,
            "package_digest": package.package_digest,
            "package_bytes": package.package_bytes,
            "project": package.project,
            "error": package.error,
        })).collect::<Vec<_>>(),
    })
}

/// 回滚已安装包并返回阶段事件。
pub(super) fn rollback_package(
    params: &InstalledPackageParams,
) -> anyhow::Result<serde_json::Value> {
    let mut events = Vec::new();
    let mut reporter = |event: &crate::transfer::DeployEvent| events.push(event.clone());
    let outcome = crate::transfer::rollback_installed_package(
        &params.service,
        params.release.as_deref(),
        params.timeout_ms,
        params.stable_for_ms,
        &mut reporter,
    )?;
    Ok(serde_json::json!({ "outcome": outcome, "events": events }))
}

/// 恢复中断的包切换并返回阶段事件。
pub(super) fn recover_package(
    params: &InstalledPackageParams,
) -> anyhow::Result<serde_json::Value> {
    let mut events = Vec::new();
    let mut reporter = |event: &crate::transfer::DeployEvent| events.push(event.clone());
    let recovered = crate::transfer::recover_installed_package(
        &params.service,
        params.timeout_ms,
        params.stable_for_ms,
        &mut reporter,
    )?;
    Ok(serde_json::json!({ "recovered": recovered, "events": events }))
}

/// 解除包注册并保留精确清理结果。
pub(super) fn uninstall_package(
    params: &UninstallPackageParams,
) -> anyhow::Result<serde_json::Value> {
    let outcome = crate::transfer::uninstall_installed_package(&params.service, params.purge)?;
    let registration = match outcome.registration {
        crate::transfer::PackageRegistrationDisposition::Removed => {
            serde_json::json!({ "status": "removed" })
        }
        crate::transfer::PackageRegistrationDisposition::Absent => {
            serde_json::json!({ "status": "absent" })
        }
        crate::transfer::PackageRegistrationDisposition::UnrelatedPreserved(path) => {
            serde_json::json!({ "status": "unrelated_preserved", "path": path })
        }
    };
    Ok(serde_json::json!({
        "purged": outcome.purged,
        "registration": registration,
    }))
}

/// 解析包构建平台范围。
fn package_platform(value: &str) -> anyhow::Result<crate::package::PackagePlatform> {
    match value {
        "all" => Ok(crate::package::PackagePlatform::All),
        value => target_platform(value).map(crate::package::PackagePlatform::Target),
    }
}

/// 解析当前或具体部署平台。
fn target_platform(value: &str) -> anyhow::Result<crate::config::DeployPlatform> {
    if value == "current" {
        return crate::config::DeployPlatform::current()
            .normalized()
            .map_err(anyhow::Error::msg);
    }
    crate::config::DeployPlatform::parse_key(value).map_err(anyhow::Error::msg)
}

/// MCP 省略 source 时使用当前目录。
fn current_directory() -> PathBuf {
    PathBuf::from(".")
}

/// MCP 默认包构建包含全部声明平台。
fn all_platforms() -> String {
    "all".to_owned()
}

/// MCP 默认包物化选择当前平台。
fn current_platform_name() -> String {
    "current".to_owned()
}

/// MCP 默认包安装验收超时。
fn default_deploy_timeout() -> u64 {
    30_000
}

/// MCP 默认包安装稳定窗口。
fn default_stable_window() -> u64 {
    2_000
}

/// MCP 默认包 release 保留数量。
fn default_release_keep() -> u32 {
    3
}
