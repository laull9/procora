//! 完整 Service 的无目标全托管部署入口。

use std::env;

use anyhow::Context;

use super::deploy_memory::{DeployTargetMemory, load_target, save_target};

/// `procora deploy` 的完整 Service 与确定性验收参数。
#[derive(Debug, clap::Args)]
pub struct DeployArgs {
    /// 本机服务目录或显式声明式配置文件。
    #[arg(default_value = ".")]
    pub source: std::path::PathBuf,
    /// 要连接的服务器：SSH config 别名或 `[user@]host`。
    #[arg(long, value_name = "SSH_TARGET")]
    pub ssh: Option<String>,
    /// 远端 Procora 命令名或 Unix/Windows 无空格路径。
    #[arg(long, value_name = "PATH")]
    pub remote_bin: Option<String>,
    /// 额外校验配置中的 project 必须与该名称一致。
    #[arg(long, value_name = "NAME")]
    pub service: Option<String>,
    /// 等待新版本达到可用状态的最长时间。
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "30s",
        value_parser = crate::config::parse_duration
    )]
    pub timeout: u64,
    /// 新版本保持可用后才确认部署的稳定窗口。
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "2s",
        value_parser = crate::config::parse_duration
    )]
    pub stable_for: u64,
    /// 每个 Service 保留的最近 release 数量。
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=32))]
    pub keep: u32,
    /// 禁止主机确认与密码登录回退，适合 CI。
    #[arg(long)]
    pub batch: bool,
    /// 只探测、选择产物并生成部署计划，不上传或切换远端 Service。
    #[arg(long)]
    pub dry_run: bool,
}

/// 校验本地 Service 并通过 SSH 交给远端 Procora 全托管。
pub(super) fn run(arguments: &DeployArgs) -> anyhow::Result<()> {
    let DeployArgs {
        source,
        ssh,
        remote_bin,
        service: expected_service,
        timeout: timeout_ms,
        stable_for: stable_for_ms,
        keep,
        batch,
        dry_run,
    } = arguments;
    let source = crate::cli::api::absolute_user_path(source)?;
    let discovered = crate::config::discover_path(&source)
        .with_context(|| format!("无法发现待部署 Service：{}", source.display()))?;
    let remembered = if ssh.is_none()
        && !batch
        && env::var("PROCORA_SSH_TARGET")
            .ok()
            .is_none_or(|value| value.trim().is_empty())
    {
        load_target(&discovered.root, &discovered.compiled.spec.project)
    } else {
        None
    };
    if let Some(target) = &remembered {
        eprintln!(
            "使用 Service `{}` 上次成功的部署目标：{}",
            target.project, target.ssh_target
        );
    }
    let ssh_target = ssh
        .clone()
        .or_else(|| remembered.as_ref().map(|target| target.ssh_target.clone()));
    let remote_bin = remote_bin
        .clone()
        .or_else(|| remembered.as_ref().map(|target| target.remote_bin.clone()));
    let mut settings = crate::cli::api::deploy::DeploySettings {
        source,
        ssh_target,
        remote_bin,
        expected_service: expected_service.clone(),
        timeout_ms: *timeout_ms,
        stable_for_ms: *stable_for_ms,
        keep: *keep,
        batch: *batch,
    };
    if *dry_run {
        if settings.ssh_target.is_none() {
            settings.ssh_target = Some(crate::transfer::resolve_ssh_target(None, settings.batch)?);
        }
        let preview = crate::cli::api::deploy::preview(&settings)?;
        print_preview(&preview);
        println!(
            "预检完成：未修改远端；执行相同命令并移除 `--dry-run` 即可部署。\n修订：{}",
            preview.revision
        );
        return Ok(());
    }
    let mut reporter = |event: &crate::transfer::DeployEvent| {
        if matches!(event.phase.as_str(), "preflight" | "binary" | "archive") {
            println!("{}", event.message);
        } else {
            eprintln!("[{}] {}", event.phase, event.message);
        }
    };
    let outcome = crate::cli::api::deploy::execute(&settings, None, &mut reporter)?;
    if outcome.changed {
        println!("部署完成：{}，release {}", outcome.project, outcome.release);
        if let Some(previous) = outcome.previous_release {
            println!("上一版本：{previous}");
        }
    } else {
        println!(
            "无需更新：{} 已运行 release {}",
            outcome.project, outcome.release
        );
    }
    if !batch
        && let Err(error) = save_target(DeployTargetMemory {
            root: discovered.root,
            project: outcome.project,
            ssh_target: outcome.preview.ssh_target,
            remote_bin: outcome.preview.remote_bin,
        })
    {
        eprintln!("警告：部署已完成，但无法保存部署目标记忆：{error:#}");
    }
    Ok(())
}

/// 以适合人工确认的稳定格式打印无副作用部署计划。
fn print_preview(preview: &crate::transfer::DeployPreview) {
    println!("部署计划：{} → {}", preview.project, preview.ssh_target);
    println!("远端平台：{}", preview.target_platform.key());
    println!("配置入口：{}", preview.config_path);
    for binary in &preview.binaries {
        println!(
            "二进制 `{}`：{}，{} → {}",
            binary.name,
            binary.selector,
            binary.source.display(),
            binary.target
        );
    }
    println!(
        "内容：{}（压缩后 {}），release {}",
        crate::transfer::human_bytes(preview.content_bytes),
        crate::transfer::human_bytes(preview.archive_bytes),
        &preview.archive_sha256[..16]
    );
    println!(
        "验收：最长 {} ms，稳定窗口 {} ms；保留 {} 个 release",
        preview.timeout_ms, preview.stable_for_ms, preview.keep
    );
}
