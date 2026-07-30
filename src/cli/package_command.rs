//! `.pcpkg` Service 包的命令行入口。

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use clap::{Args, Subcommand};

use super::{api, package_build_command::PackageBuildArgs};
use crate::{config::DeployPlatform, package};

/// `procora package` 的嵌套参数。
#[derive(Debug, Args)]
pub struct PackageArgs {
    /// 要执行的包操作。
    #[command(subcommand)]
    pub command: PackageCommand,
}

/// Procora Service 包支持的独立操作。
#[derive(Debug, Subcommand)]
pub enum PackageCommand {
    /// 从 Service 目录构建确定性 `.pcpkg`。
    Build(PackageBuildArgs),
    /// 读取清单摘要，不展开或完整读取全部 Blob。
    Inspect {
        /// 要检查的 `.pcpkg` 文件。
        package: PathBuf,
        /// 输出包含完整清单的稳定 JSON。
        #[arg(long)]
        json: bool,
    },
    /// 流式校验包内每个 Blob 的大小与 SHA-256。
    Verify {
        /// 要验证的 `.pcpkg` 文件。
        package: PathBuf,
    },
    /// 验证并为一个平台物化完整 Service 目录。
    Extract {
        /// 要解包的 `.pcpkg` 文件。
        package: PathBuf,
        /// 必须尚不存在的输出目录。
        #[arg(short, long)]
        output: PathBuf,
        /// `current` 或具体的 `os-arch[-environment]`。
        #[arg(long, default_value = "current")]
        platform: String,
    },
    /// 安装为本机不可变 release，并在验活失败时自动回滚。
    Install {
        /// 要安装的 `.pcpkg` 文件。
        package: PathBuf,
        /// 等待全部 Task 可用的最长时间。
        #[arg(
            long,
            value_name = "DURATION",
            default_value = "30s",
            value_parser = crate::config::parse_duration
        )]
        timeout: u64,
        /// 持续可用达到该窗口后才确认 release。
        #[arg(
            long,
            value_name = "DURATION",
            default_value = "2s",
            value_parser = crate::config::parse_duration
        )]
        stable_for: u64,
        /// 为该 Service 保留的最近 release 数量。
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=32))]
        keep: u32,
    },
    /// 从包启动与当前 TUI 同生命周期的临时 Service。
    Run {
        /// 要临时运行的 `.pcpkg` 文件。
        package: PathBuf,
    },
    /// 列出本机已安装包、活动 release 和恢复状态。
    List {
        /// 输出稳定 JSON，适合脚本和诊断。
        #[arg(long)]
        json: bool,
    },
    /// 查看一个已安装 Service 的包和 release 详情。
    Status {
        /// 配置中的稳定 Service 名称。
        service: String,
        /// 输出稳定 JSON，适合脚本和诊断。
        #[arg(long)]
        json: bool,
    },
    /// 把已安装 Service 切换到历史 release，并在验活失败时恢复原版本。
    Rollback {
        /// 配置中的稳定 Service 名称。
        service: String,
        /// 指定 release ID；省略时选择最近一个非活动 release。
        release: Option<String>,
        /// 等待全部 Task 可用的最长时间。
        #[arg(
            long,
            value_name = "DURATION",
            default_value = "30s",
            value_parser = crate::config::parse_duration
        )]
        timeout: u64,
        /// 持续可用达到该窗口后才确认回滚。
        #[arg(
            long,
            value_name = "DURATION",
            default_value = "2s",
            value_parser = crate::config::parse_duration
        )]
        stable_for: u64,
    },
    /// 恢复一次停留在 pending 阶段的中断安装。
    Recover {
        /// 配置中的稳定 Service 名称。
        service: String,
        /// 恢复与验活的最长时间。
        #[arg(
            long,
            value_name = "DURATION",
            default_value = "30s",
            value_parser = crate::config::parse_duration
        )]
        timeout: u64,
        /// 恢复后持续可用达到该窗口才算成功。
        #[arg(
            long,
            value_name = "DURATION",
            default_value = "2s",
            value_parser = crate::config::parse_duration
        )]
        stable_for: u64,
    },
    /// 从 Center 解除包安装；默认保留 release 和原始包供恢复。
    Uninstall {
        /// 配置中的稳定 Service 名称。
        service: String,
        /// 同时永久删除该 Service 的 release、状态和原始包。
        #[arg(long)]
        purge: bool,
    },
}

/// 执行一个独立包操作。
pub fn run(arguments: PackageArgs) -> anyhow::Result<()> {
    match arguments.command {
        PackageCommand::Build(arguments) => super::package_build_command::run(&arguments),
        PackageCommand::Inspect {
            package: path,
            json,
        } => inspect(&path, json),
        PackageCommand::Verify { package: path } => verify(&path),
        PackageCommand::Extract {
            package: path,
            output,
            platform,
        } => extract(&path, &output, &platform),
        PackageCommand::Install {
            package,
            timeout,
            stable_for,
            keep,
        } => install_path(&package, timeout, stable_for, keep),
        PackageCommand::Run { package } => super::runtime::run_package_temporary(&package),
        PackageCommand::List { json } => super::package_installed_command::list(json),
        PackageCommand::Status { service, json } => {
            super::package_installed_command::status(&service, json)
        }
        PackageCommand::Rollback {
            service,
            release,
            timeout,
            stable_for,
        } => super::package_installed_command::rollback(
            &service,
            release.as_deref(),
            timeout,
            stable_for,
        ),
        PackageCommand::Recover {
            service,
            timeout,
            stable_for,
        } => super::package_installed_command::recover(&service, timeout, stable_for),
        PackageCommand::Uninstall { service, purge } => {
            super::package_installed_command::uninstall(&service, purge).map(drop)
        }
    }
}

/// 安装包并输出与裸机 deploy 一致的 release 结果。
pub(super) fn install_path(
    path: &Path,
    timeout_ms: u64,
    stable_for_ms: u64,
    keep: u32,
) -> anyhow::Result<()> {
    let path = api::absolute_user_path(path)?;
    let mut reporter = |event: &crate::transfer::DeployEvent| {
        eprintln!("[{}] {}", event.phase, event.message);
    };
    let outcome =
        crate::transfer::install_package(&path, timeout_ms, stable_for_ms, keep, &mut reporter)?;
    if outcome.changed {
        println!("安装完成：{}，release {}", outcome.project, outcome.release);
        if let Some(previous) = outcome.previous_release {
            println!("上一版本：{previous}");
        }
    } else {
        println!(
            "无需更新：{} 已运行 release {}",
            outcome.project, outcome.release
        );
    }
    Ok(())
}

/// 输出包清单或紧凑的人类可读摘要。
fn inspect(path: &Path, json: bool) -> anyhow::Result<()> {
    let path = api::absolute_user_path(path)?;
    let info = package::inspect(&path)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "package_digest": info.package_digest,
                "package_bytes": info.package_bytes,
                "manifest": info.manifest,
            }))?
        );
        return Ok(());
    }
    let platforms = info
        .manifest
        .binaries
        .values()
        .flat_map(|binary| binary.variants.keys().cloned())
        .collect::<BTreeSet<_>>();
    let variants = info
        .manifest
        .binaries
        .values()
        .map(|binary| binary.variants.len())
        .sum::<usize>();
    println!("Service: {}", info.manifest.project);
    println!("Format: {}", info.manifest.format);
    println!("Package: {}", info.package_digest);
    println!("包大小: {} 字节", info.package_bytes);
    println!("配置: {}", info.manifest.config.source);
    println!(
        "内容: {} 个普通文件，{} 个逻辑二进制 / {} 个变体",
        info.manifest.files.len(),
        info.manifest.binaries.len(),
        variants
    );
    if !platforms.is_empty() {
        println!(
            "平台: {}",
            platforms.into_iter().collect::<Vec<_>>().join("、")
        );
    }
    if !info.manifest.exports.is_empty() {
        println!(
            "导出项: {}",
            info.manifest
                .exports
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("、")
        );
    }
    Ok(())
}

/// 完整验证包内容。
fn verify(path: &Path) -> anyhow::Result<()> {
    let path = api::absolute_user_path(path)?;
    let info = package::verify(&path)?;
    println!(
        "验证通过：{} ({}, {} 字节)",
        info.manifest.project, info.package_digest, info.package_bytes
    );
    Ok(())
}

/// 为一个具体平台物化包。
fn extract(path: &Path, output: &Path, platform: &str) -> anyhow::Result<()> {
    let path = api::absolute_user_path(path)?;
    let output = api::absolute_user_path(output)?;
    let platform = parse_target_platform(platform)?;
    let result = package::extract(&path, &output, platform)?;
    println!("已解包到 {}", output.display());
    println!("Platform: {}", result.platform.key());
    println!("Release: {}", result.release_digest);
    println!(
        "内容: {} 个文件，{} 字节",
        result.files, result.content_bytes
    );
    Ok(())
}

/// 解析解包使用的唯一目标平台。
fn parse_target_platform(value: &str) -> anyhow::Result<DeployPlatform> {
    match value {
        "current" => current_platform(),
        "all" => bail!("解包必须选择一个具体平台，不能使用 `all`"),
        value => parse_platform(value),
    }
}

/// 返回规范化后的当前 Procora 编译平台。
fn current_platform() -> anyhow::Result<DeployPlatform> {
    DeployPlatform::current()
        .normalized()
        .map_err(anyhow::Error::msg)
}

/// 解析稳定平台键并保留清晰的参数上下文。
fn parse_platform(value: &str) -> anyhow::Result<DeployPlatform> {
    DeployPlatform::parse_key(value)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("无效平台 `{value}`"))
}
