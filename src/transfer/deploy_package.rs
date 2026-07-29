//! 本机 `.pcpkg` 到不可变托管 release 的安装、验活和回滚。

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};

use super::{
    archive,
    deploy_health::{currently_accepted, wait_until_accepted},
    deploy_protocol::{DeployBinaryMetadata, DeployResult},
    deploy_report::DeployEvent,
    deploy_state::{DeploymentOutcome, DeploymentRecord, ManagedState, now_millis, release_path},
    deploy_wire::acquire_lock,
};

/// 验证并安装本机包，成功前不会替换已确认的活动 release。
#[allow(clippy::too_many_lines)]
pub(crate) fn install(
    package: &Path,
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
    reporter: &mut dyn FnMut(&DeployEvent),
) -> anyhow::Result<DeployResult> {
    validate_policy(timeout_ms, stable_for_ms, keep)?;
    let info = crate::package::verify(package)?;
    let platform = crate::config::DeployPlatform::current()
        .normalized()
        .map_err(anyhow::Error::msg)?;
    let managed_root = crate::cli::api::managed_services_root()?;
    fs::create_dir_all(&managed_root)?;
    let managed_root = crate::platform::canonicalize(&managed_root)?;
    let service_root = managed_root.join(&info.manifest.project);
    let releases_root = service_root.join("releases");
    fs::create_dir_all(&releases_root)?;
    let releases_root = crate::platform::canonicalize(&releases_root)?;
    let _lock = acquire_lock(&service_root)?;
    store_package(&service_root, package, &info.package_digest)?;
    let state_path = service_root.join("state.json");
    let mut state = ManagedState::load(&state_path, &info.manifest.project)?;
    recover_interrupted(
        &info.manifest.project,
        &releases_root,
        &mut state,
        &state_path,
        timeout_ms,
        stable_for_ms,
        reporter,
    )?;
    let active_running =
        ensure_registration_is_managed(&info.manifest.project, &releases_root, &state)?;

    report(reporter, "extract", "正在验证并物化当前平台的包内容");
    let staging = releases_root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    let extracted = crate::package::extract(package, &staging, platform.clone())?;
    let release_sha = extracted
        .release_digest
        .strip_prefix("sha256:")
        .context("包 release 摘要格式无效")?
        .to_owned();
    let release = release_sha[..16].to_owned();
    let config_path = PathBuf::from(&info.manifest.config.source);
    let prepared = (|| {
        let discovered = crate::config::discover_path(staging.join(&config_path))
            .context("包内配置在当前平台物化后无效")?;
        if discovered.compiled.spec.project != info.manifest.project {
            bail!(
                "包清单声明 Service `{}`，物化配置声明 `{}`",
                info.manifest.project,
                discovered.compiled.spec.project
            );
        }
        let binaries = binary_metadata(&discovered, &platform)?;
        state.register_release(
            &release,
            &release_sha,
            &config_path,
            Some(&platform),
            &binaries,
        )?;
        Ok(binaries)
    })();
    if prepared.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let binaries = prepared?;
    let destination = release_path(&releases_root, &release);
    if destination.exists() {
        let verified = verify_existing_release(
            &destination,
            &config_path,
            &info.manifest.project,
            &binaries,
        );
        let _ = fs::remove_dir_all(&staging);
        verified?;
    } else if let Err(error) = fs::rename(&staging, &destination) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error.into());
    }

    let previous = state.active_release.clone();
    let previous_config = previous
        .as_deref()
        .and_then(|id| previous_config(&state, &releases_root, id));
    if previous.as_deref() == Some(&release)
        && active_running
        && currently_accepted(&info.manifest.project).unwrap_or(false)
    {
        state.save(&state_path)?;
        report(reporter, "verify", "相同 release 已经处于可用状态");
        return Ok(DeployResult {
            project: info.manifest.project,
            release,
            previous_release: previous,
            changed: false,
            content_bytes: extracted.content_bytes,
            sha256: release_sha,
        });
    }

    state.pending_release = Some(release.clone());
    state.save(&state_path)?;
    report(reporter, "activate", "正在原子切换到新 release");
    let activation = switch_release(&info.manifest.project, &destination.join(&config_path))
        .and_then(|()| {
            report(reporter, "verify", "正在等待全部 Task 通过部署门控");
            wait_until_accepted(&info.manifest.project, timeout_ms, stable_for_ms)
        });
    match activation {
        Ok(()) => {
            state.active_release = Some(release.clone());
            state.pending_release = None;
            state.record(DeploymentRecord {
                release: release.clone(),
                previous_release: previous.clone(),
                outcome: DeploymentOutcome::Succeeded,
                message: None,
                recorded_at_ms: now_millis(),
            });
            let obsolete_releases = state.prune(keep as usize);
            state.save(&state_path)?;
            for id in obsolete_releases {
                let _ = fs::remove_dir_all(release_path(&releases_root, &id));
            }
            Ok(DeployResult {
                project: info.manifest.project,
                release,
                previous_release: previous,
                changed: true,
                content_bytes: extracted.content_bytes,
                sha256: release_sha,
            })
        }
        Err(failure) => finish_failed(
            &info.manifest.project,
            &state_path,
            &mut state,
            release,
            previous,
            previous_config,
            timeout_ms,
            stable_for_ms,
            reporter,
            &failure,
        ),
    }
}

/// 校验本机安装的有界验活和保留参数。
fn validate_policy(timeout_ms: u64, stable_for_ms: u64, keep: u32) -> anyhow::Result<()> {
    if timeout_ms == 0 || timeout_ms > 10 * 60 * 1_000 {
        bail!("包安装验收超时必须在 1 毫秒到 10 分钟之间");
    }
    if stable_for_ms > timeout_ms {
        bail!("包安装稳定窗口不能超过验收超时");
    }
    if !(1..=32).contains(&keep) {
        bail!("包安装 release 保留数量必须在 1–32 之间");
    }
    Ok(())
}

/// 从物化后的配置与目标文件生成可复核二进制元数据。
fn binary_metadata(
    discovered: &crate::config::DiscoveredProject,
    platform: &crate::config::DeployPlatform,
) -> anyhow::Result<Vec<DeployBinaryMetadata>> {
    crate::config::select_deploy_binaries(&discovered.compiled.deploy_binaries, platform)
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .map(|binary| {
            let path = discovered.root.join(&binary.target);
            let metadata = fs::symlink_metadata(&path).with_context(|| {
                format!(
                    "物化 release 缺少二进制 `{}`：{}",
                    binary.name,
                    path.display()
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
                bail!("物化 release 的二进制 `{}` 不是非空普通文件", binary.name);
            }
            Ok(DeployBinaryMetadata {
                name: binary.name,
                selector: binary.selector,
                target: portable_path(&binary.target)?,
                bytes: metadata.len(),
                sha256: archive::hash_file(&path)?,
            })
        })
        .collect()
}

/// 复核同 ID 的已有不可变 release 没有被外部修改。
fn verify_existing_release(
    root: &Path,
    config_path: &Path,
    project: &str,
    expected: &[DeployBinaryMetadata],
) -> anyhow::Result<()> {
    let discovered = crate::config::discover_path(root.join(config_path))?;
    if discovered.compiled.spec.project != project {
        bail!("已有 release 的 Service 身份与包不一致");
    }
    let platform = crate::config::DeployPlatform::current()
        .normalized()
        .map_err(anyhow::Error::msg)?;
    let actual = binary_metadata(&discovered, &platform)?;
    if actual != expected {
        bail!("已有不可变 release 的二进制内容发生变化");
    }
    Ok(())
}

/// 内容寻址保存原始逻辑包，便于审计和后续导出。
fn store_package(service_root: &Path, source: &Path, digest: &str) -> anyhow::Result<()> {
    let digest = digest.strip_prefix("sha256:").context("包摘要格式无效")?;
    let directory = service_root.join("packages");
    fs::create_dir_all(&directory)?;
    let destination = directory.join(format!("{digest}.pcpkg"));
    if destination.exists() {
        let existing = crate::package::verify(&destination)?;
        if existing.package_digest != format!("sha256:{digest}") {
            bail!("已保存包的逻辑摘要与文件名不一致");
        }
        return Ok(());
    }
    let temporary = directory.join(format!(".incoming-{}.pcpkg", uuid::Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        fs::copy(source, &temporary)?;
        let copied = crate::package::verify(&temporary)?;
        if copied.package_digest != format!("sha256:{digest}") {
            bail!("复制包时逻辑摘要发生变化");
        }
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// 恢复上次在 pending 阶段中断的本机包安装。
#[allow(clippy::too_many_arguments)]
fn recover_interrupted(
    project: &str,
    releases_root: &Path,
    state: &mut ManagedState,
    state_path: &Path,
    timeout_ms: u64,
    stable_for_ms: u64,
    reporter: &mut dyn FnMut(&DeployEvent),
) -> anyhow::Result<()> {
    let Some(pending) = state.pending_release.clone() else {
        return Ok(());
    };
    report(reporter, "rollback", "检测到未完成安装，正在恢复稳定版本");
    let existing = managed_service(project)?;
    let pending_root = release_path(releases_root, &pending);
    let active_root = state
        .active_release
        .as_deref()
        .map(|id| release_path(releases_root, id));
    match existing {
        Some(existing) if Some(existing.root.as_path()) == active_root.as_deref() => {}
        Some(existing) if existing.root == pending_root => rollback(
            project,
            active_config(state, releases_root),
            timeout_ms,
            stable_for_ms,
        )?,
        Some(existing) if existing.root.parent() != Some(releases_root) => {
            bail!(
                "恢复包安装时发现同名非托管 Service `{project}`：{}",
                existing.root.display()
            );
        }
        Some(existing) => bail!("恢复包安装时发现未知活动目录：{}", existing.root.display()),
        None => {
            if let Some(config) = active_config(state, releases_root) {
                crate::cli::api::add_service(config)?;
                wait_until_accepted(project, timeout_ms, stable_for_ms)?;
            }
        }
    }
    state.record(DeploymentRecord {
        release: pending,
        previous_release: state.active_release.clone(),
        outcome: DeploymentOutcome::FailedRolledBack,
        message: Some("检测到上次包安装中断，已恢复稳定 release".to_owned()),
        recorded_at_ms: now_millis(),
    });
    state.pending_release = None;
    state.save(state_path)?;
    report(reporter, "restored", "上一稳定 release 已恢复");
    Ok(())
}

/// 完成失败安装的回滚与持久记录。
#[allow(clippy::too_many_arguments)]
fn finish_failed(
    project: &str,
    state_path: &Path,
    state: &mut ManagedState,
    release: String,
    previous: Option<String>,
    previous_config: Option<PathBuf>,
    timeout_ms: u64,
    stable_for_ms: u64,
    reporter: &mut dyn FnMut(&DeployEvent),
    failure: &anyhow::Error,
) -> anyhow::Result<DeployResult> {
    report(reporter, "rollback", "新 release 验收失败，正在回滚");
    let restored = rollback(project, previous_config, timeout_ms, stable_for_ms);
    let (outcome, message) = match restored {
        Ok(()) => (
            DeploymentOutcome::FailedRolledBack,
            format!("新版本验收失败：{failure:#}；旧版本已恢复"),
        ),
        Err(rollback) => (
            DeploymentOutcome::FailedRollbackFailed,
            format!("新版本验收失败：{failure:#}；自动回滚失败：{rollback:#}"),
        ),
    };
    state.active_release.clone_from(&previous);
    if matches!(outcome, DeploymentOutcome::FailedRolledBack) {
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

/// 切换或首次注册包物化出的 Service。
fn switch_release(project: &str, config_path: &Path) -> anyhow::Result<()> {
    if let Some(existing) = managed_service(project)? {
        crate::cli::api::relocate_managed_service(project, &existing.root, config_path)?;
    } else {
        crate::cli::api::add_service(config_path.to_path_buf())?;
    }
    Ok(())
}

/// 恢复旧配置，或移除首次安装失败的 Service。
fn rollback(
    project: &str,
    previous_config: Option<PathBuf>,
    timeout_ms: u64,
    stable_for_ms: u64,
) -> anyhow::Result<()> {
    match (managed_service(project)?, previous_config) {
        (Some(existing), Some(previous)) => {
            crate::cli::api::relocate_managed_service(project, &existing.root, &previous)?;
        }
        (Some(_), None) => {
            crate::cli::api::remove_service(project)?;
            return Ok(());
        }
        (None, Some(previous)) => {
            crate::cli::api::add_service(previous)?;
        }
        (None, None) => return Ok(()),
    }
    wait_until_accepted(project, timeout_ms, stable_for_ms)
}

/// 确认同名注册项属于统一的托管 release 根目录。
fn ensure_registration_is_managed(
    project: &str,
    releases_root: &Path,
    state: &ManagedState,
) -> anyhow::Result<bool> {
    let Some(existing) = managed_service(project)? else {
        return Ok(false);
    };
    let expected = state
        .active_release
        .as_deref()
        .map(|id| release_path(releases_root, id));
    if existing.root.parent() != Some(releases_root) {
        bail!(
            "本机已有同名非托管 Service `{project}`：{}；拒绝由包安装接管",
            existing.root.display()
        );
    }
    if expected.as_deref() != Some(existing.root.as_path()) {
        bail!("托管 Service `{project}` 的活动目录与 state.json 不一致");
    }
    Ok(existing.status == crate::protocol::ServiceStatusDto::Running)
}

/// 返回 Center 中的同名 Service。
fn managed_service(project: &str) -> anyhow::Result<Option<crate::protocol::ServiceViewDto>> {
    Ok(crate::cli::api::managed_deploy_services()?
        .into_iter()
        .find(|service| service.name == project))
}

/// 返回状态记录中的活动配置入口。
fn active_config(state: &ManagedState, releases_root: &Path) -> Option<PathBuf> {
    state
        .active_release
        .as_deref()
        .and_then(|id| previous_config(state, releases_root, id))
}

/// 返回一个 release 中确实存在的配置文件。
fn previous_config(state: &ManagedState, releases_root: &Path, release: &str) -> Option<PathBuf> {
    let path = release_path(releases_root, release).join(state.config_path(release)?);
    path.is_file().then_some(path)
}

/// 把相对 target 转换为稳定 `/` 分隔文本。
fn portable_path(path: &Path) -> anyhow::Result<String> {
    let text = path
        .components()
        .map(|component| match component {
            std::path::Component::Normal(value) => {
                value.to_str().context("二进制 target 必须使用 UTF-8")
            }
            _ => bail!("二进制 target 必须是普通相对路径"),
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .join("/");
    Ok(text)
}

/// 向 CLI 报告本机包安装阶段。
fn report(reporter: &mut dyn FnMut(&DeployEvent), phase: &str, message: &str) {
    reporter(&DeployEvent::new(phase, message));
}
