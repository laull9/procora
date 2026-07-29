//! 完整 Service 的无目标全托管部署入口。

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
    let settings = crate::cli::api::deploy::DeploySettings {
        source: source.clone(),
        ssh_target: ssh.clone(),
        remote_bin: remote_bin.clone(),
        expected_service: expected_service.clone(),
        timeout_ms: *timeout_ms,
        stable_for_ms: *stable_for_ms,
        keep: *keep,
        batch: *batch,
    };
    let mut reporter = |event: &crate::transfer::DeployEvent| {
        if matches!(event.phase.as_str(), "preflight" | "binary" | "archive") {
            println!("{}", event.message);
        } else {
            eprintln!("[{}] {}", event.phase, event.message);
        }
    };
    let outcome = crate::cli::api::deploy::execute(&settings, None, &mut reporter)?;
    println!("部署完成：{}，release {}", outcome.project, outcome.release);
    if let Some(previous) = outcome.previous_release {
        println!("上一版本：{previous}");
    }
    Ok(())
}
