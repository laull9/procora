//! 已安装 Procora 包与 release 的查询和管理命令。

use anyhow::Context;

use super::api;
use crate::{config::DeployPlatform, package};

/// 执行显式历史 release 回滚并输出最终活动版本。
pub(super) fn rollback(
    service: &str,
    release: Option<&str>,
    timeout_ms: u64,
    stable_for_ms: u64,
) -> anyhow::Result<()> {
    let mut reporter = |event: &crate::transfer::DeployEvent| {
        eprintln!("[{}] {}", event.phase, event.message);
    };
    let outcome = crate::transfer::rollback_installed_package(
        service,
        release,
        timeout_ms,
        stable_for_ms,
        &mut reporter,
    )?;
    println!(
        "回滚完成：{}，active {}（原 {}）",
        outcome.project,
        outcome.release,
        outcome.previous_release.as_deref().unwrap_or("-")
    );
    Ok(())
}

/// 恢复中断安装；无 pending 时保持幂等。
pub(super) fn recover(service: &str, timeout_ms: u64, stable_for_ms: u64) -> anyhow::Result<()> {
    let mut reporter = |event: &crate::transfer::DeployEvent| {
        eprintln!("[{}] {}", event.phase, event.message);
    };
    if crate::transfer::recover_installed_package(
        service,
        timeout_ms,
        stable_for_ms,
        &mut reporter,
    )? {
        println!("恢复完成：{service} 已回到上一个稳定 release");
    } else {
        println!("无需恢复：{service} 没有 pending 安装");
    }
    Ok(())
}

/// 解除包安装；只有 `purge` 明确为真时删除本地历史数据。
pub(super) fn uninstall(service: &str, purge: bool) -> anyhow::Result<()> {
    crate::transfer::uninstall_installed_package(service, purge)?;
    if purge {
        println!("已卸载并清理 `{service}` 的 release、状态和原始包");
    } else {
        println!(
            "已从 Center 解除 `{service}`；安装数据仍保留，可重新安装同一包或使用 `--purge` 清理"
        );
    }
    Ok(())
}

/// 列出当前用户全部已安装包。
pub(super) fn list(json: bool) -> anyhow::Result<()> {
    let catalog = package::installed_catalog(&api::managed_services_root()?)?;
    if json {
        println!("{}", installed_catalog_json(&catalog)?);
        return Ok(());
    }
    if catalog.services.is_empty() {
        println!("尚未安装 Procora 包；可运行 `procora package install <包>`。");
        return Ok(());
    }
    for service in catalog.services {
        let active = service.active_release.as_deref().unwrap_or("-");
        let pending = service
            .pending_release
            .as_deref()
            .map_or(String::new(), |release| format!("，pending {release}"));
        let state = service
            .error
            .as_deref()
            .map_or(String::new(), |error| format!("，状态损坏：{error}"));
        println!(
            "{}：active {}{}，{} 个 release，{} 个原始包{}",
            service.project,
            active,
            pending,
            service.releases.len(),
            service.packages.len(),
            state
        );
    }
    Ok(())
}

/// 输出一个已安装 Service 的 release 与包详情。
pub(super) fn status(service: &str, json: bool) -> anyhow::Result<()> {
    let _: crate::core::ServiceName = service.parse()?;
    let catalog = package::installed_catalog(&api::managed_services_root()?)?;
    let installed = catalog
        .services
        .into_iter()
        .find(|installed| installed.project == service)
        .with_context(|| format!("本机没有已安装包 Service `{service}`"))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&installed_service_json(&installed))?
        );
        return Ok(());
    }
    println!("Service: {}", installed.project);
    println!("目录: {}", installed.root.display());
    println!(
        "Active: {}",
        installed.active_release.as_deref().unwrap_or("-")
    );
    println!(
        "Pending: {}",
        installed.pending_release.as_deref().unwrap_or("-")
    );
    if let Some(error) = &installed.error {
        println!("状态错误: {error}");
    }
    println!("Releases:");
    for release in installed.releases {
        let marker = if release.active {
            "active"
        } else if release.pending {
            "pending"
        } else {
            "inactive"
        };
        let platform = release
            .target_platform
            .as_ref()
            .map_or_else(|| "-".to_owned(), DeployPlatform::key);
        println!("  {}  {:8}  {}", release.id, marker, platform);
    }
    println!("Packages:");
    for stored in installed.packages {
        let identity = stored
            .package_digest
            .as_deref()
            .unwrap_or("invalid package");
        println!("  {}  {}", identity, stored.path.display());
    }
    Ok(())
}

/// 把安装目录转换成稳定脚本 JSON。
fn installed_catalog_json(catalog: &package::InstalledCatalog) -> anyhow::Result<String> {
    let services = catalog
        .services
        .iter()
        .map(installed_service_json)
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({ "services": services })).map_err(Into::into)
}

/// 把单个安装项转换成稳定脚本 JSON。
fn installed_service_json(service: &package::InstalledService) -> serde_json::Value {
    serde_json::json!({
        "project": service.project,
        "root": service.root,
        "active_release": service.active_release,
        "pending_release": service.pending_release,
        "error": service.error,
        "releases": service.releases.iter().map(|release| serde_json::json!({
            "id": release.id,
            "sha256": release.sha256,
            "config_path": release.config_path,
            "target_platform": release.target_platform,
            "deployed_at_ms": release.deployed_at_ms,
            "active": release.active,
            "pending": release.pending,
        })).collect::<Vec<_>>(),
        "packages": service.packages.iter().map(|stored| serde_json::json!({
            "path": stored.path,
            "package_digest": stored.package_digest,
            "package_bytes": stored.package_bytes,
            "project": stored.project,
            "error": stored.error,
        })).collect::<Vec<_>>(),
    })
}
