//! CLI 与 MCP 共享的全托管裸机部署入口。

use std::path::PathBuf;

use anyhow::{Context, bail};

use crate::transfer::{DeployEvent, DeployOutcome, DeployPreview};

/// 一次全托管裸机部署的完整输入。
pub(crate) struct DeploySettings {
    pub(crate) source: PathBuf,
    pub(crate) ssh_target: Option<String>,
    pub(crate) remote_bin: Option<String>,
    pub(crate) expected_service: Option<String>,
    pub(crate) timeout_ms: u64,
    pub(crate) stable_for_ms: u64,
    pub(crate) keep: u32,
    pub(crate) batch: bool,
}

/// 已发现并校验、可直接交给传输层的Service。
struct PreparedService {
    root: PathBuf,
    project: String,
    config_path: PathBuf,
    binaries: crate::config::DeployBinaries,
}

/// 无副作用校验本地包并探测远端平台、二进制选择和归档修订。
pub(crate) fn preview(settings: &DeploySettings) -> anyhow::Result<DeployPreview> {
    let ssh_target = settings
        .ssh_target
        .as_deref()
        .context("MCP 部署预检必须显式提供 ssh")?;
    let prepared = prepare(settings)?;
    crate::transfer::preview_deploy(
        &prepared.root,
        &prepared.project,
        &prepared.config_path,
        &prepared.binaries,
        ssh_target,
        settings.remote_bin.as_deref(),
        settings.timeout_ms,
        settings.stable_for_ms,
        settings.keep,
    )
}

/// 重新校验预检修订并执行全托管部署。
pub(crate) fn execute(
    settings: &DeploySettings,
    expected_revision: Option<&str>,
    reporter: &mut dyn FnMut(&DeployEvent),
) -> anyhow::Result<DeployOutcome> {
    let prepared = prepare(settings)?;
    crate::transfer::deploy(
        &prepared.root,
        &prepared.project,
        &prepared.config_path,
        &prepared.binaries,
        settings.ssh_target.as_deref(),
        settings.remote_bin.as_deref(),
        settings.timeout_ms,
        settings.stable_for_ms,
        settings.keep,
        settings.batch,
        expected_revision,
        reporter,
    )
}

/// 固定路径、配置、服务名、target冲突和验收参数。
fn prepare(settings: &DeploySettings) -> anyhow::Result<PreparedService> {
    validate_policy(settings)?;
    let source = super::absolute_user_path(&settings.source)?;
    let discovered = crate::config::discover_path(&source)
        .with_context(|| format!("无法发现待部署 Service：{}", source.display()))?;
    if let Some(expected) = settings.expected_service.as_deref()
        && discovered.compiled.spec.project != expected
    {
        bail!(
            "配置中的 project `{}` 与期望 Service `{expected}` 不一致",
            discovered.compiled.spec.project
        );
    }
    let config_path = discovered
        .config_path
        .strip_prefix(&discovered.root)
        .context("配置入口不在 Service 根目录内")?
        .to_path_buf();
    validate_binary_boundaries(&discovered, &config_path)?;
    Ok(PreparedService {
        root: discovered.root,
        project: discovered.compiled.spec.project,
        config_path,
        binaries: discovered.compiled.deploy_binaries,
    })
}

/// 拒绝没有确定上界或不符合保留策略的部署参数。
fn validate_policy(settings: &DeploySettings) -> anyhow::Result<()> {
    if settings.timeout_ms == 0 {
        bail!("部署验收超时必须大于零");
    }
    if settings.timeout_ms > 10 * 60 * 1_000 {
        bail!("部署验收超时不能超过10分钟");
    }
    if settings.stable_for_ms > settings.timeout_ms {
        bail!("部署稳定窗口不能超过验收超时");
    }
    if !(1..=32).contains(&settings.keep) {
        bail!("release 保留数量必须在 1–32 之间");
    }
    Ok(())
}

/// 防止本地产物或release target覆盖配置入口。
fn validate_binary_boundaries(
    discovered: &crate::config::DiscoveredProject,
    config_path: &std::path::Path,
) -> anyhow::Result<()> {
    let config_source = crate::platform::simplify_path(&discovered.config_path);
    for (name, binary) in &discovered.compiled.deploy_binaries {
        if std::iter::once(&binary.target)
            .chain(
                binary
                    .variants
                    .iter()
                    .filter_map(|variant| variant.target.as_ref()),
            )
            .any(|target| target == config_path)
        {
            bail!("二进制 `{name}` 的 target 不能覆盖 Service 配置入口");
        }
        if binary
            .variants
            .iter()
            .any(|variant| crate::platform::simplify_path(&variant.source) == config_source)
        {
            bail!("二进制 `{name}` 的 source 不能使用 Service 配置入口");
        }
    }
    Ok(())
}
