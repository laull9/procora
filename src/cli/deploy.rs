//! 完整 Service 的无目标全托管部署入口。

use anyhow::{Context, bail};

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
    } = arguments;
    if *timeout_ms == 0 {
        bail!("部署验收超时必须大于零");
    }
    if stable_for_ms > timeout_ms {
        bail!("部署稳定窗口不能超过验收超时");
    }
    let source = crate::cli::api::absolute_user_path(source)?;
    let discovered = crate::config::discover_path(&source)
        .with_context(|| format!("无法发现待部署 Service：{}", source.display()))?;
    if let Some(expected) = expected_service
        && discovered.compiled.spec.project != expected.as_str()
    {
        bail!(
            "配置中的 project `{}` 与 --service `{expected}` 不一致",
            discovered.compiled.spec.project
        );
    }
    let config_path = discovered
        .config_path
        .strip_prefix(&discovered.root)
        .context("配置入口不在 Service 根目录内")?
        .to_path_buf();
    let outcome: crate::transfer::DeployOutcome = crate::transfer::deploy(
        &discovered.root,
        &discovered.compiled.spec.project,
        &config_path,
        ssh.as_deref(),
        remote_bin.as_deref(),
        *timeout_ms,
        *stable_for_ms,
        *keep,
        *batch,
    )?;
    println!("部署完成：{}，release {}", outcome.project, outcome.release);
    if let Some(previous) = outcome.previous_release {
        println!("上一版本：{previous}");
    }
    Ok(())
}
