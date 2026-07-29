//! `.pcpkg` 通过现有裸机部署协议的远端平台物化与发送。

use std::path::Path;

use anyhow::bail;

use super::{
    deploy_prepare::{build_preview, prepare_package_deployment},
    deploy_remote::{DeployOptions, deploy_outcome, report_prepared, transfer_with_fallback},
    deploy_report::{DeployEvent, DeployOutcome, DeployPreview},
    remote,
    remote_auth::SshAuth,
};

/// 无副作用探测远端平台并生成包部署预检。
#[allow(clippy::too_many_arguments)]
pub(crate) fn preview(
    package: &Path,
    project: &str,
    configured_target: &str,
    configured_remote_bin: Option<&str>,
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
) -> anyhow::Result<DeployPreview> {
    let mut remote_bin = configured_remote_bin.unwrap_or("procora").to_owned();
    remote::validate_remote_bin(&remote_bin)?;
    let ssh_target = remote::resolve_ssh_target(Some(configured_target), true)?;
    let mut auth = SshAuth::automatic();
    let (prepared, config_path) = prepare_package_deployment(
        package,
        project,
        &ssh_target,
        configured_remote_bin,
        &mut remote_bin,
        &mut auth,
        true,
    )?;
    build_preview(
        package,
        project,
        &config_path,
        &ssh_target,
        &remote_bin,
        timeout_ms,
        stable_for_ms,
        keep,
        &prepared,
    )
}

/// 按远端实际平台物化包，并复用现有 release、验活和回滚协议。
#[allow(clippy::too_many_arguments)]
pub(crate) fn deploy(
    package: &Path,
    project: &str,
    configured_target: Option<&str>,
    configured_remote_bin: Option<&str>,
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
    batch: bool,
    expected_revision: Option<&str>,
    reporter: &mut dyn FnMut(&DeployEvent),
) -> anyhow::Result<DeployOutcome> {
    let mut remote_bin = configured_remote_bin.unwrap_or("procora").to_owned();
    remote::validate_remote_bin(&remote_bin)?;
    let ssh_target = remote::resolve_ssh_target(configured_target, batch)?;
    let mut auth = SshAuth::automatic();
    let (prepared, config_path) = prepare_package_deployment(
        package,
        project,
        &ssh_target,
        configured_remote_bin,
        &mut remote_bin,
        &mut auth,
        batch,
    )?;
    let preview = build_preview(
        package,
        project,
        &config_path,
        &ssh_target,
        &remote_bin,
        timeout_ms,
        stable_for_ms,
        keep,
        &prepared,
    )?;
    if let Some(expected) = expected_revision
        && expected != preview.revision
    {
        bail!(
            "部署预检修订已经变化：期望 `{expected}`，当前 `{}`；请重新预检",
            preview.revision
        );
    }
    let mut events = Vec::new();
    report_prepared(&preview, &prepared.archive, &mut events, reporter);
    let options = DeployOptions {
        timeout_ms,
        stable_for_ms,
        keep,
        target_platform: Some(&prepared.target_platform),
        binaries: &prepared.binaries,
    };
    let result = transfer_with_fallback(
        &ssh_target,
        &mut remote_bin,
        configured_remote_bin,
        batch,
        &mut auth,
        project,
        &config_path,
        &prepared.archive,
        options,
        &mut events,
        reporter,
    )?;
    Ok(deploy_outcome(result, preview, events))
}
