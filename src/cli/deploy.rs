//! 完整 Service 的无目标全托管部署入口。

use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::Context;

use super::deploy_memory::{DeployTargetMemory, load_target, save_target};

/// 交互式部署确认后的结果。
pub(super) enum ReviewedDeploy {
    /// 用户在计划确认阶段取消，远端未被修改。
    Cancelled { project: String },
    /// 部署完成或确认远端已经运行相同 release。
    Completed {
        project: String,
        release: String,
        changed: bool,
        ssh_target: String,
    },
}

/// 已解析部署目标记忆的共享请求。
struct DeployRequest {
    memory_root: PathBuf,
    settings: crate::cli::api::deploy::DeploySettings,
}

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
    let mut request = prepare_request(arguments, None)?;
    if arguments.dry_run {
        if request.settings.ssh_target.is_none() {
            request.settings.ssh_target = Some(crate::transfer::resolve_ssh_target(
                None,
                request.settings.batch,
            )?);
        }
        let preview = crate::cli::api::deploy::preview(&request.settings)?;
        print_preview(&preview);
        println!(
            "预检完成：未修改远端；执行相同命令并移除 `--dry-run` 即可部署。\n修订：{}",
            preview.revision
        );
        return Ok(());
    }
    let outcome = execute_and_report(&request.settings, None)?;
    remember_target(&request.memory_root, request.settings.batch, &outcome);
    Ok(())
}

/// 预检并展示完整计划，确认后复核修订并执行部署。
pub(super) fn run_reviewed(
    arguments: &DeployArgs,
    memory_root: Option<&Path>,
) -> anyhow::Result<ReviewedDeploy> {
    let mut request = prepare_request(arguments, memory_root)?;
    if request.settings.ssh_target.is_none() {
        request.settings.ssh_target = Some(crate::transfer::resolve_ssh_target(
            None,
            request.settings.batch,
        )?);
    }
    let preview = crate::cli::api::deploy::preview(&request.settings)?;
    print_preview(&preview);
    let confirmed = crate::tui::select_inline(
        "确认远端裸机部署",
        "预检未修改远端；确认后将上传、切换并验活，失败时自动回滚。",
        vec![
            crate::tui::SelectionItem::new("确认部署（推荐）", "复核当前修订并执行完整部署", true),
            crate::tui::SelectionItem::new("取消", "保留本地包且不修改远端", false),
        ],
    )?
    .unwrap_or(false);
    if !confirmed {
        return Ok(ReviewedDeploy::Cancelled {
            project: preview.project,
        });
    }
    request.settings.remote_bin = Some(preview.remote_bin.clone());
    let outcome = execute_and_report(&request.settings, Some(&preview.revision))?;
    remember_target(&request.memory_root, request.settings.batch, &outcome);
    Ok(ReviewedDeploy::Completed {
        project: outcome.project,
        release: outcome.release,
        changed: outcome.changed,
        ssh_target: outcome.preview.ssh_target,
    })
}

/// 解析部署来源、服务身份和可复用的目标记忆。
fn prepare_request(
    arguments: &DeployArgs,
    memory_root_override: Option<&Path>,
) -> anyhow::Result<DeployRequest> {
    let source = crate::cli::api::absolute_user_path(&arguments.source)?;
    let (memory_root, project) = if crate::package::is_package_path(&source) {
        let info = crate::package::inspect(&source)?;
        (source.clone(), info.manifest.project)
    } else {
        let discovered = crate::config::discover_path(&source)
            .with_context(|| format!("无法发现待部署 Service：{}", source.display()))?;
        (discovered.root, discovered.compiled.spec.project)
    };
    let memory_root = memory_root_override.map_or(memory_root, crate::platform::simplify_path);
    let remembered = if arguments.ssh.is_none()
        && !arguments.batch
        && env::var("PROCORA_SSH_TARGET")
            .ok()
            .is_none_or(|value| value.trim().is_empty())
    {
        load_target(&memory_root, &project)
    } else {
        None
    };
    if let Some(target) = &remembered {
        eprintln!(
            "使用 Service `{}` 上次成功的部署目标：{}",
            target.project, target.ssh_target
        );
    }
    let ssh_target = arguments
        .ssh
        .clone()
        .or_else(|| remembered.as_ref().map(|target| target.ssh_target.clone()));
    let remote_bin = arguments
        .remote_bin
        .clone()
        .or_else(|| remembered.as_ref().map(|target| target.remote_bin.clone()));
    Ok(DeployRequest {
        memory_root,
        settings: crate::cli::api::deploy::DeploySettings {
            source,
            ssh_target,
            remote_bin,
            expected_service: arguments.service.clone(),
            timeout_ms: arguments.timeout,
            stable_for_ms: arguments.stable_for,
            keep: arguments.keep,
            batch: arguments.batch,
        },
    })
}

/// 执行部署并输出稳定的阶段与结果摘要。
fn execute_and_report(
    settings: &crate::cli::api::deploy::DeploySettings,
    expected_revision: Option<&str>,
) -> anyhow::Result<crate::transfer::DeployOutcome> {
    let mut reporter = |event: &crate::transfer::DeployEvent| {
        if matches!(event.phase.as_str(), "preflight" | "binary" | "archive") {
            println!("{}", event.message);
        } else {
            eprintln!("[{}] {}", event.phase, event.message);
        }
    };
    let outcome = crate::cli::api::deploy::execute(settings, expected_revision, &mut reporter)?;
    if outcome.changed {
        println!("部署完成：{}，release {}", outcome.project, outcome.release);
        if let Some(previous) = &outcome.previous_release {
            println!("上一版本：{previous}");
        }
    } else {
        println!(
            "无需更新：{} 已运行 release {}",
            outcome.project, outcome.release
        );
    }
    Ok(outcome)
}

/// 非批处理成功后按稳定 Service 身份保存非敏感目标。
fn remember_target(memory_root: &Path, batch: bool, outcome: &crate::transfer::DeployOutcome) {
    if !batch
        && let Err(error) = save_target(DeployTargetMemory {
            root: memory_root.to_path_buf(),
            project: outcome.project.clone(),
            ssh_target: outcome.preview.ssh_target.clone(),
            remote_bin: outcome.preview.remote_bin.clone(),
        })
    {
        eprintln!("警告：部署已完成，但无法保存部署目标记忆：{error:#}");
    }
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
