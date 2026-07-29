//! 已安装 `.pcpkg` release 的显式回滚、恢复与清理。

use std::fs;

use anyhow::{Context, bail};

use super::{
    deploy_health::wait_until_accepted,
    deploy_package::{
        ensure_registration_is_managed, finish_failed, managed_service, previous_config,
        recover_interrupted, report, switch_release, validate_policy,
    },
    deploy_protocol::DeployResult,
    deploy_report::DeployEvent,
    deploy_state::{DeploymentOutcome, DeploymentRecord, ManagedState, now_millis},
    deploy_wire::acquire_lock,
};

/// 把已安装 Service 切换到指定或最近一个非活动 release。
pub(crate) fn rollback_installed(
    project: &str,
    requested_release: Option<&str>,
    timeout_ms: u64,
    stable_for_ms: u64,
    reporter: &mut dyn FnMut(&DeployEvent),
) -> anyhow::Result<DeployResult> {
    validate_policy(timeout_ms, stable_for_ms, 1)?;
    let _: crate::core::ServiceName = project.parse()?;
    let managed_root = crate::platform::canonicalize(&crate::cli::api::managed_services_root()?)?;
    let service_root = crate::platform::canonicalize(managed_root.join(project))?;
    let releases_root = crate::platform::canonicalize(service_root.join("releases"))?;
    let _lock = acquire_lock(&service_root)?;
    let state_path = service_root.join("state.json");
    let mut state = ManagedState::load(&state_path, project)?;
    recover_interrupted(
        project,
        &releases_root,
        &mut state,
        &state_path,
        timeout_ms,
        stable_for_ms,
        reporter,
    )?;
    ensure_registration_is_managed(project, &releases_root, &state)?;
    let previous = state
        .active_release
        .clone()
        .context("已安装 Service 没有活动 release")?;
    let target = requested_release.map_or_else(
        || {
            state
                .releases
                .iter()
                .filter(|release| release.id != previous)
                .max_by_key(|release| release.deployed_at_ms)
                .map(|release| release.id.clone())
                .context("没有可回滚的历史 release")
        },
        |release| {
            state
                .releases
                .iter()
                .find(|candidate| candidate.id == release)
                .map(|candidate| candidate.id.clone())
                .with_context(|| format!("不存在 release `{release}`"))
        },
    )?;
    if target == previous {
        bail!("release `{target}` 已经处于活动状态");
    }
    let config = previous_config(&state, &releases_root, &target)
        .with_context(|| format!("release `{target}` 的配置入口缺失"))?;
    state.pending_release = Some(target.clone());
    state.save(&state_path)?;
    report(reporter, "activate", "正在切换到历史 release");
    let activation = switch_release(project, &config).and_then(|()| {
        report(reporter, "verify", "正在等待回滚 release 通过部署门控");
        wait_until_accepted(project, timeout_ms, stable_for_ms)
    });
    if let Err(failure) = activation {
        let previous_config = previous_config(&state, &releases_root, &previous);
        return finish_failed(
            project,
            &state_path,
            &mut state,
            target,
            Some(previous.clone()),
            previous_config,
            timeout_ms,
            stable_for_ms,
            reporter,
            &failure,
        );
    }
    state.active_release = Some(target.clone());
    state.pending_release = None;
    state.record(DeploymentRecord {
        release: target.clone(),
        previous_release: Some(previous.clone()),
        outcome: DeploymentOutcome::Succeeded,
        message: Some("用户主动回滚到历史 release".to_owned()),
        recorded_at_ms: now_millis(),
    });
    state.save(&state_path)?;
    report(reporter, "complete", "历史 release 已恢复并通过验活");
    Ok(DeployResult {
        project: project.to_owned(),
        release: target,
        previous_release: Some(previous),
        changed: true,
        content_bytes: 0,
        sha256: String::new(),
    })
}

/// 恢复一个停留在 pending 状态的中断安装。
pub(crate) fn recover_installed(
    project: &str,
    timeout_ms: u64,
    stable_for_ms: u64,
    reporter: &mut dyn FnMut(&DeployEvent),
) -> anyhow::Result<bool> {
    validate_policy(timeout_ms, stable_for_ms, 1)?;
    let _: crate::core::ServiceName = project.parse()?;
    let managed_root = crate::platform::canonicalize(&crate::cli::api::managed_services_root()?)?;
    let service_root = crate::platform::canonicalize(managed_root.join(project))?;
    let releases_root = crate::platform::canonicalize(service_root.join("releases"))?;
    let _lock = acquire_lock(&service_root)?;
    let state_path = service_root.join("state.json");
    let mut state = ManagedState::load(&state_path, project)?;
    let interrupted = state.pending_release.is_some();
    recover_interrupted(
        project,
        &releases_root,
        &mut state,
        &state_path,
        timeout_ms,
        stable_for_ms,
        reporter,
    )?;
    Ok(interrupted)
}

/// 从 Center 解除已安装 Service，并可选择清理其包与 release 数据。
pub(crate) fn uninstall_installed(project: &str, purge: bool) -> anyhow::Result<bool> {
    let _: crate::core::ServiceName = project.parse()?;
    let managed_root = crate::platform::canonicalize(&crate::cli::api::managed_services_root()?)?;
    let service_root = managed_root.join(project);
    if !service_root.is_dir() {
        bail!("本机没有已安装包 Service `{project}`");
    }
    let service_root = crate::platform::canonicalize(&service_root)?;
    let releases_root = crate::platform::canonicalize(service_root.join("releases"))?;
    {
        let _lock = acquire_lock(&service_root)?;
        if let Some(existing) = managed_service(project)? {
            if existing.root.parent() != Some(releases_root.as_path()) {
                bail!(
                    "同名 Service `{project}` 不属于包托管目录：{}；拒绝解除",
                    existing.root.display()
                );
            }
            crate::cli::api::remove_service(project)?;
        }
    }
    if purge {
        fs::remove_dir_all(&service_root)
            .with_context(|| format!("无法清理安装目录 `{}`", service_root.display()))?;
    }
    Ok(purge)
}
