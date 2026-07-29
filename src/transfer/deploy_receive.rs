//! 远端完整 Service release 的接收、验活与自动回滚。

use std::{
    fs,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};

use crate::config::UploadKind;

use super::{
    archive,
    deploy_health::wait_until_accepted,
    deploy_protocol::{DeployInit, DeployPhase, DeployResponse, DeployResult},
    deploy_state::{DeploymentOutcome, DeploymentRecord, ManagedState, now_millis, release_path},
    deploy_wire::{
        MAX_DEPLOY_BYTES, acquire_lock, read_json_line, send_progress, send_response,
        validate_init, write_archive,
    },
};

/// 接收完整 Service，并在确定性验收失败时恢复旧 release。
pub(crate) fn run() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let init: DeployInit = read_json_line(&mut input, "部署请求")?;
    validate_init(&init)?;

    let managed_root = crate::cli::api::managed_services_root()?;
    fs::create_dir_all(&managed_root)?;
    let managed_root = crate::platform::canonicalize(&managed_root)?;
    let service_root = managed_root.join(&init.project);
    let releases_root = service_root.join("releases");
    fs::create_dir_all(&releases_root)?;
    let releases_root = crate::platform::canonicalize(&releases_root)?;
    let _lock = acquire_lock(&service_root)?;

    send_response(&DeployResponse::Ready {
        project: init.project.clone(),
    })?;
    let archive_path = service_root.join(format!(".incoming-{}.tar.gz", uuid::Uuid::new_v4()));
    let result = receive_and_deploy(&mut input, &init, &releases_root, &archive_path);
    let _ = fs::remove_file(&archive_path);
    let result = result?;
    send_response(&DeployResponse::Complete { result })
}

/// 完整接收归档、构造不可变 release 并执行切换。
fn receive_and_deploy(
    input: &mut impl Read,
    init: &DeployInit,
    releases_root: &Path,
    archive_path: &Path,
) -> anyhow::Result<DeployResult> {
    let actual = write_archive(input, archive_path, init.archive_bytes)?;
    if !actual.eq_ignore_ascii_case(&init.sha256) {
        bail!(
            "部署归档 SHA-256 不匹配：期望 {}，实际 {actual}",
            init.sha256
        );
    }
    send_progress(
        DeployPhase::Validating,
        "归档完整性通过，正在验证 release 配置",
    );
    let release = init.sha256[..16].to_owned();
    let config_path = PathBuf::from(&init.config_path);
    let state_path = releases_root
        .parent()
        .context("release 根目录没有父目录")?
        .join("state.json");
    let mut state = ManagedState::load(&state_path, &init.project)?;
    recover_interrupted_deployment(
        &init.project,
        releases_root,
        &mut state,
        &state_path,
        init.timeout_ms,
        init.stable_for_ms,
    )?;
    ensure_registration_is_managed(&init.project, releases_root, &state)?;
    state.register_release(
        &release,
        &init.sha256,
        &config_path,
        init.target_platform.as_ref(),
        &init.binaries,
    )?;
    let release_directory =
        prepare_release(init, &config_path, releases_root, archive_path, &release)?;
    let previous = state.active_release.clone();
    let previous_config = match previous.as_deref() {
        Some(id) => Some(
            previous_config(&state, releases_root, id)
                .with_context(|| format!("活动旧 release `{id}` 缺少配置入口"))?,
        ),
        None => None,
    };
    state.pending_release = Some(release.clone());
    state.save(&state_path)?;

    send_progress(DeployPhase::Activating, "正在原子切换到新 release");
    let activation =
        switch_release(&init.project, &release_directory.join(&config_path)).and_then(|()| {
            send_progress(DeployPhase::Verifying, "正在等待全部 Task 通过部署门控");
            wait_until_accepted(&init.project, init.timeout_ms, init.stable_for_ms)
        });
    match activation {
        Ok(()) => commit_success(
            init,
            releases_root,
            &state_path,
            &mut state,
            release,
            previous,
        ),
        Err(failure) => finish_failed_activation(
            init,
            &state_path,
            &mut state,
            release,
            previous,
            previous_config,
            &failure,
        ),
    }
}

/// 原子提交成功状态后再清理不再引用的旧release。
fn commit_success(
    init: &DeployInit,
    releases_root: &Path,
    state_path: &Path,
    state: &mut ManagedState,
    release: String,
    previous: Option<String>,
) -> anyhow::Result<DeployResult> {
    state.active_release = Some(release.clone());
    state.pending_release = None;
    state.record(DeploymentRecord {
        release: release.clone(),
        previous_release: previous.clone(),
        outcome: DeploymentOutcome::Succeeded,
        message: None,
        recorded_at_ms: now_millis(),
    });
    let stale_releases = state.prune(init.keep as usize);
    state.save(state_path)?;
    for stale in stale_releases {
        let _ = fs::remove_dir_all(release_path(releases_root, &stale));
    }
    Ok(DeployResult {
        project: init.project.clone(),
        release,
        previous_release: previous,
        content_bytes: init.content_bytes,
        sha256: init.sha256.clone(),
    })
}

/// 处理验活失败，记录回滚是否完成并保留可继续恢复的pending状态。
fn finish_failed_activation(
    init: &DeployInit,
    state_path: &Path,
    state: &mut ManagedState,
    release: String,
    previous: Option<String>,
    previous_config: Option<PathBuf>,
    failure: &anyhow::Error,
) -> anyhow::Result<DeployResult> {
    send_progress(
        DeployPhase::RollingBack,
        "新 release 未通过验收，正在恢复上一稳定版本",
    );
    let rollback = rollback(
        &init.project,
        previous_config,
        init.timeout_ms,
        init.stable_for_ms,
    );
    let (outcome, message, rollback_completed) = match rollback {
        Ok(()) if previous.is_some() => {
            send_progress(DeployPhase::Restored, "上一稳定 release 已恢复并通过验活");
            (
                DeploymentOutcome::FailedRolledBack,
                format!("新版本验收失败：{failure:#}；旧版本已恢复"),
                true,
            )
        }
        Ok(()) => {
            send_progress(DeployPhase::Restored, "首次部署的失败 Service 已移除");
            (
                DeploymentOutcome::FailedRolledBack,
                format!("首次部署验收失败：{failure:#}；失败服务已停止"),
                true,
            )
        }
        Err(rollback) => (
            DeploymentOutcome::FailedRollbackFailed,
            format!("新版本验收失败：{failure:#}；自动回滚失败：{rollback:#}"),
            false,
        ),
    };
    state.active_release.clone_from(&previous);
    if rollback_completed {
        state.pending_release = None;
    }
    state.record(DeploymentRecord {
        release,
        previous_release: previous,
        outcome,
        message: Some(message.clone()),
        recorded_at_ms: now_millis(),
    });
    state.save(state_path)?;
    bail!(message)
}

/// 发现两阶段状态未提交时，以最近一次已确认release为准完成恢复。
fn recover_interrupted_deployment(
    project: &str,
    releases_root: &Path,
    state: &mut ManagedState,
    state_path: &Path,
    timeout_ms: u64,
    stable_for_ms: u64,
) -> anyhow::Result<()> {
    let Some(pending) = state.pending_release.clone() else {
        return Ok(());
    };
    send_progress(
        DeployPhase::RollingBack,
        format!("检测到未完成部署 `{pending}`，正在恢复上一稳定版本"),
    );
    let existing = crate::cli::api::managed_deploy_services()?
        .into_iter()
        .find(|service| service.name == project);
    let pending_root = release_path(releases_root, &pending);
    let active_root = state
        .active_release
        .as_deref()
        .map(|release| release_path(releases_root, release));
    match existing {
        Some(existing) if Some(existing.root.as_path()) == active_root.as_deref() => {}
        Some(existing) if existing.root == pending_root => {
            rollback(
                project,
                active_config(state, releases_root)?,
                timeout_ms,
                stable_for_ms,
            )
            .context("恢复上次中断的部署失败")?;
        }
        Some(existing) if existing.root.parent() != Some(releases_root) => {
            bail!(
                "恢复中断部署时发现同名非托管 Service `{project}`：{}",
                existing.root.display()
            );
        }
        Some(existing) => {
            bail!(
                "恢复中断部署时发现未知活动目录：{}",
                existing.root.display()
            );
        }
        None => {
            if let Some(config) = active_config(state, releases_root)? {
                crate::cli::api::add_service(config).context("恢复中断部署的旧 release 失败")?;
                wait_until_accepted(project, timeout_ms, stable_for_ms)
                    .context("中断部署后的旧 release 未恢复可用")?;
            }
        }
    }
    state.record(DeploymentRecord {
        release: pending,
        previous_release: state.active_release.clone(),
        outcome: DeploymentOutcome::FailedRolledBack,
        message: Some("检测到上次部署中断，已恢复上一稳定 release".to_owned()),
        recorded_at_ms: now_millis(),
    });
    state.pending_release = None;
    state.save(state_path)?;
    send_progress(
        DeployPhase::Restored,
        "上次中断的部署已恢复到上一稳定 release",
    );
    Ok(())
}

/// 返回当前已确认release的配置入口。
fn active_config(state: &ManagedState, releases_root: &Path) -> anyhow::Result<Option<PathBuf>> {
    state
        .active_release
        .as_deref()
        .map(|release| {
            previous_config(state, releases_root, release)
                .with_context(|| format!("活动旧 release `{release}` 缺少配置入口"))
        })
        .transpose()
}

/// 展开归档并在远端重新校验配置身份。
fn prepare_release(
    init: &DeployInit,
    config_path: &Path,
    releases_root: &Path,
    archive_path: &Path,
    release: &str,
) -> anyhow::Result<PathBuf> {
    let destination = release_path(releases_root, release);
    if destination.exists() {
        let discovered = crate::config::discover_path(destination.join(config_path))?;
        ensure_project(init, &discovered.compiled.spec.project)?;
        verify_release_binaries(init, &discovered.compiled.deploy_binaries, &destination)?;
        return crate::platform::canonicalize(&destination).map_err(Into::into);
    }
    let staging = releases_root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    let unpacked = archive::unpack(
        archive_path,
        &staging,
        UploadKind::Directory,
        MAX_DEPLOY_BYTES,
    );
    if unpacked.as_ref().is_err_and(|_| staging.exists()) {
        let _ = fs::remove_dir_all(&staging);
    }
    let unpacked = unpacked?;
    if unpacked != init.content_bytes {
        let _ = fs::remove_dir_all(&staging);
        bail!(
            "部署内容大小不匹配：期望 {}，实际 {unpacked}",
            init.content_bytes
        );
    }
    let discovered = crate::config::discover_path(staging.join(config_path));
    let discovered = match discovered {
        Ok(discovered) => discovered,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error).context("远端 release 配置校验失败");
        }
    };
    if let Err(error) = ensure_project(init, &discovered.compiled.spec.project) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) =
        verify_release_binaries(init, &discovered.compiled.deploy_binaries, &staging)
    {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    fs::rename(&staging, &destination)?;
    crate::platform::canonicalize(&destination).map_err(Into::into)
}

/// 按远端自身平台重新选择配置变体并复核release内二进制摘要。
fn verify_release_binaries(
    init: &DeployInit,
    binaries: &crate::config::DeployBinaries,
    release_root: &Path,
) -> anyhow::Result<()> {
    if binaries.is_empty() {
        if !init.binaries.is_empty() {
            bail!("部署请求携带二进制元数据，但远端配置没有声明 binaries");
        }
        if init.target_platform.is_none() {
            return Ok(());
        }
    }
    let current = crate::config::DeployPlatform::current()
        .normalized()
        .map_err(anyhow::Error::msg)?;
    if init.target_platform.as_ref() != Some(&current) {
        bail!(
            "部署目标平台在探测后发生变化：请求 {:?}，当前 {}",
            init.target_platform
                .as_ref()
                .map(crate::config::DeployPlatform::key),
            current.key()
        );
    }
    if binaries.is_empty() {
        return Ok(());
    }
    let selected =
        crate::config::select_deploy_binaries(binaries, &current).map_err(anyhow::Error::msg)?;
    if selected.len() != init.binaries.len() {
        bail!("部署二进制数量与远端配置不一致");
    }
    for binary in selected {
        let metadata = init
            .binaries
            .iter()
            .find(|metadata| metadata.name == binary.name)
            .with_context(|| format!("部署请求缺少二进制 `{}` 的摘要", binary.name))?;
        let target = PathBuf::from(&metadata.target);
        if metadata.selector != binary.selector || target != binary.target {
            bail!("二进制 `{}` 的远端平台选择与请求不一致", binary.name);
        }
        let path = release_root.join(&binary.target);
        let file = fs::symlink_metadata(&path)
            .with_context(|| format!("release 缺少二进制 `{}`：{}", binary.name, path.display()))?;
        if file.file_type().is_symlink() || !file.is_file() || file.len() != metadata.bytes {
            bail!("二进制 `{}` 的文件类型或大小与请求不一致", binary.name);
        }
        let actual = archive::hash_file(&path)?;
        if !actual.eq_ignore_ascii_case(&metadata.sha256) {
            bail!(
                "二进制 `{}` SHA-256 不匹配：期望 {}，实际 {actual}",
                binary.name,
                metadata.sha256
            );
        }
    }
    Ok(())
}

/// 将同名 Service 原子切换到新 release。
fn switch_release(project: &str, config_path: &Path) -> anyhow::Result<()> {
    let existing = crate::cli::api::managed_deploy_services()?
        .into_iter()
        .find(|service| service.name == project);
    if let Some(existing) = existing {
        crate::cli::api::relocate_managed_service(project, &existing.root, config_path)
            .context("新 release 切换失败")?;
    } else {
        crate::cli::api::add_service(config_path.to_path_buf()).context("新 release 启动失败")?;
    }
    Ok(())
}

/// 停止失败 release，并恢复和验收上一 release。
fn rollback(
    project: &str,
    previous_config: Option<PathBuf>,
    timeout_ms: u64,
    stable_for_ms: u64,
) -> anyhow::Result<()> {
    let existing = crate::cli::api::managed_deploy_services()?
        .into_iter()
        .find(|service| service.name == project);
    match (existing, previous_config) {
        (Some(existing), Some(previous)) => {
            crate::cli::api::relocate_managed_service(project, &existing.root, &previous)
                .context("切回旧 release 失败")?;
        }
        (Some(_), None) => {
            crate::cli::api::remove_service(project)
                .context("停止首次部署的失败 release 时出错")?;
            return Ok(());
        }
        (None, Some(previous)) => {
            crate::cli::api::add_service(previous).context("重新注册旧 release 失败")?;
        }
        (None, None) => return Ok(()),
    }
    wait_until_accepted(project, timeout_ms, stable_for_ms).context("旧 release 未恢复可用")
}

/// 确认远端同名服务不是用户自行注册的目录。
fn ensure_registration_is_managed(
    project: &str,
    releases_root: &Path,
    state: &ManagedState,
) -> anyhow::Result<()> {
    if let Some(existing) = crate::cli::api::managed_deploy_services()?
        .into_iter()
        .find(|service| service.name == project)
    {
        let expected = state
            .active_release
            .as_deref()
            .map(|release| release_path(releases_root, release));
        if existing.root.parent() != Some(releases_root) {
            bail!(
                "远端已有同名非托管 Service `{project}`：{}；拒绝由 deploy 接管",
                existing.root.display()
            );
        }
        if expected.as_deref() != Some(existing.root.as_path()) {
            bail!("托管 Service `{project}` 的活动目录与 state.json 不一致，拒绝自动切换");
        }
    }
    Ok(())
}

/// 返回旧 release 的配置入口。
fn previous_config(state: &ManagedState, releases_root: &Path, release: &str) -> Option<PathBuf> {
    let relative = state.config_path(release)?;
    let path = release_path(releases_root, release).join(relative);
    path.is_file().then_some(path)
}

/// 确认远端重新编译得到相同 Service 身份。
fn ensure_project(init: &DeployInit, actual: &str) -> anyhow::Result<()> {
    if init.project != actual {
        bail!(
            "本地声明 Service `{}`，远端 release 配置声明 `{actual}`",
            init.project
        );
    }
    Ok(())
}
