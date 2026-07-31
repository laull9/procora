//! Procora 包的准备、构建与可选一键部署入口。

use std::{
    io,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
};

use anyhow::{Context, bail};
use clap::Args;

use super::api;
use crate::package::{self, PackagePlatform};

/// `procora package build` 的构建、准备和可选部署参数。
#[derive(Debug, Args)]
pub struct PackageBuildArgs {
    /// 配置文件或可自动发现配置的 Service 目录。
    #[arg(default_value = ".")]
    source: PathBuf,
    /// 输出文件；省略时使用当前目录的 `<service>.pcpkg`。
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// `all`、`current` 或具体的 `os-arch[-environment]`。
    #[arg(long, default_value = "all")]
    platform: String,
    /// 打包前执行的命令文本；可重复使用，按声明顺序且不经过 shell。
    #[arg(long, value_name = "COMMAND")]
    prepare: Vec<String>,
    /// 构建成功后立即把包部署到指定 SSH 目标。
    #[arg(long, value_name = "SSH_TARGET")]
    deploy: Option<String>,
    /// 部署时使用的远端 Procora 命令名或路径。
    #[arg(long, value_name = "PATH", requires = "deploy")]
    remote_bin: Option<String>,
    /// 部署后等待全部 Task 可用的最长时间。
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "30s",
        value_parser = crate::config::parse_duration
    )]
    timeout: u64,
    /// 部署后持续可用达到该窗口才确认 release。
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "2s",
        value_parser = crate::config::parse_duration
    )]
    stable_for: u64,
    /// 部署目标为该 Service 保留的最近 release 数量。
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=32))]
    keep: u32,
    /// 部署时禁止主机确认与密码登录回退，适合 CI。
    #[arg(long, requires = "deploy")]
    batch: bool,
    /// 安全备份并替换内容不同的已有普通包；失败时恢复原文件。
    #[arg(long)]
    force: bool,
    /// 只输出稳定 JSON 构建结果，适合 Python 和其他脚本调用。
    #[arg(long, conflicts_with = "deploy")]
    json: bool,
}

/// 执行包准备、确定性构建和可选部署。
pub(super) fn run(arguments: &PackageBuildArgs) -> anyhow::Result<()> {
    let source = api::absolute_user_path(&arguments.source)?;
    super::project::warn_python_execution(&source);
    let discovered = crate::config::discover_path(&source)
        .with_context(|| format!("无法发现待打包 Service：{}", source.display()))?;
    let output = arguments.output.as_deref().map_or_else(
        || {
            crate::platform::current_dir()
                .context("无法读取当前目录")
                .map(|current| current.join(format!("{}.pcpkg", discovered.compiled.spec.project)))
        },
        api::absolute_user_path,
    )?;
    let platform = parse_build_platform(&arguments.platform)?;
    run_prepare_commands(
        &arguments.prepare,
        &discovered,
        &output,
        &platform,
        arguments.json,
    )?;
    let result = if arguments.force {
        package::build_replacing(&source, &output, platform)?
    } else {
        package::build(&source, &output, platform)?
    };
    if arguments.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print_build_result(&result);
    }
    if let Some(ssh_target) = arguments.deploy.as_deref() {
        deploy_built_package(&result, &discovered.root, ssh_target, arguments)?;
    }
    Ok(())
}

/// 输出可供人工和脚本稳定读取的构建摘要。
fn print_build_result(result: &package::PackageBuildResult) {
    if result.changed {
        println!("已构建 {}", result.path.display());
    } else {
        println!("包未变化 {}", result.path.display());
    }
    println!("Service: {}", result.project);
    println!("Package: {}", result.package_digest);
    println!(
        "内容: {} 个普通文件，{} 个二进制变体，{} 字节",
        result.files, result.binary_variants, result.package_bytes
    );
}

/// 按用户显式顺序执行不经过 shell 的构建准备命令。
fn run_prepare_commands(
    commands: &[String],
    discovered: &crate::config::DiscoveredProject,
    output: &Path,
    platform: &PackagePlatform,
    json_output: bool,
) -> anyhow::Result<()> {
    let platform = match platform {
        PackagePlatform::All => "all".to_owned(),
        PackagePlatform::Target(platform) => platform.key(),
    };
    for (index, command_text) in commands.iter().enumerate() {
        let (program, arguments) = crate::config::split_command_text(command_text)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("无法解析第 {} 条构建准备命令", index + 1))?;
        eprintln!("[准备 {}/{}] {}", index + 1, commands.len(), command_text);
        let mut command = ProcessCommand::new(&program);
        command
            .args(&arguments)
            .current_dir(&discovered.root)
            .env("PROCORA_PACKAGE_SOURCE", &discovered.root)
            .env("PROCORA_PACKAGE_OUTPUT", output)
            .env("PROCORA_PACKAGE_PLATFORM", &platform)
            .env("PROCORA_PACKAGE_PROJECT", &discovered.compiled.spec.project);
        let status = if json_output {
            command.stdout(Stdio::piped());
            let mut child = command
                .spawn()
                .with_context(|| format!("无法启动构建准备程序 `{program}`"))?;
            let forwarded = child
                .stdout
                .take()
                .map_or(Ok(0), |mut stdout| io::copy(&mut stdout, &mut io::stderr()));
            let status = child.wait().context("无法等待构建准备程序退出")?;
            forwarded.context("无法转发构建准备程序 stdout")?;
            status
        } else {
            command
                .status()
                .with_context(|| format!("无法启动构建准备程序 `{program}`"))?
        };
        if !status.success() {
            bail!(
                "第 {} 条构建准备命令失败（退出 {}）：{}",
                index + 1,
                status,
                command_text
            );
        }
    }
    Ok(())
}

/// 把刚验证的包直接交给现有裸机部署状态机。
fn deploy_built_package(
    result: &package::PackageBuildResult,
    memory_root: &Path,
    ssh_target: &str,
    arguments: &PackageBuildArgs,
) -> anyhow::Result<()> {
    println!("开始部署 {} → {ssh_target}", result.path.display());
    let settings = api::deploy::DeploySettings {
        source: result.path.clone(),
        ssh_target: Some(ssh_target.to_owned()),
        remote_bin: arguments.remote_bin.clone(),
        expected_service: Some(result.project.clone()),
        timeout_ms: arguments.timeout,
        stable_for_ms: arguments.stable_for,
        keep: arguments.keep,
        batch: arguments.batch,
    };
    let mut reporter = |event: &crate::transfer::DeployEvent| {
        eprintln!("[{}] {}", event.phase, event.message);
    };
    let outcome = api::deploy::execute(&settings, None, &mut reporter)?;
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
    save_deploy_target(memory_root, arguments.batch, outcome);
    Ok(())
}

/// 非批处理成功后保留与普通 deploy 一致的目标记忆。
fn save_deploy_target(memory_root: &Path, batch: bool, outcome: crate::transfer::DeployOutcome) {
    if !batch
        && let Err(error) =
            super::deploy_memory::save_target(super::deploy_memory::DeployTargetMemory {
                root: memory_root.to_path_buf(),
                project: outcome.project,
                ssh_target: outcome.preview.ssh_target,
                remote_bin: outcome.preview.remote_bin,
            })
    {
        eprintln!("警告：部署已完成，但无法保存部署目标记忆：{error:#}");
    }
}

/// 解析构建平台范围。
fn parse_build_platform(value: &str) -> anyhow::Result<PackagePlatform> {
    match value {
        "all" => Ok(PackagePlatform::All),
        "current" => Ok(PackagePlatform::Target(current_platform()?)),
        value => Ok(PackagePlatform::Target(
            crate::config::DeployPlatform::parse_key(value)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("无效平台 `{value}`"))?,
        )),
    }
}

/// 返回规范化后的当前 Procora 编译平台。
fn current_platform() -> anyhow::Result<crate::config::DeployPlatform> {
    crate::config::DeployPlatform::current()
        .normalized()
        .map_err(anyhow::Error::msg)
}
