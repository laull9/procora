//! 面向裸机部署目标的远端观察与生命周期命令。

use std::{
    env,
    io::{self, Write},
};

use clap::{Args, Subcommand};

use crate::core::{ServiceName, TaskId};

use super::deploy_memory::load_target;

/// 远端裸机 Service 的连接参数与操作。
#[derive(Debug, Args)]
pub struct RemoteArgs {
    /// 要连接的服务器：SSH config 别名或 `[user@]host`。
    #[arg(long, global = true, value_name = "SSH_TARGET")]
    ssh: Option<String>,
    /// 远端 Procora 命令名或 Unix/Windows 无空格路径。
    #[arg(long, global = true, value_name = "PATH")]
    remote_bin: Option<String>,
    /// 禁止主机确认与密码登录回退，适合 CI。
    #[arg(long, global = true)]
    batch: bool,
    /// 要在远端执行的只读查询或生命周期操作。
    #[command(subcommand)]
    command: RemoteCommand,
}

/// 裸机远端的常用观察与控制操作。
#[derive(Debug, Subcommand)]
enum RemoteCommand {
    /// 列出远端 Procora 当前托管的全部 Service。
    Ps,
    /// 显示远端全局 Procora 服务器状态。
    Status,
    /// 读取指定 Service 的持久状态历史。
    History {
        /// Service 稳定名称。
        service: ServiceName,
    },
    /// 读取指定 Task 的当前活动日志。
    Logs {
        /// Service 稳定名称。
        service: ServiceName,
        /// Service 内的 Task 标识。
        task: TaskId,
    },
    /// 启动指定 Service。
    Start {
        /// Service 稳定名称。
        service: ServiceName,
    },
    /// 重新加载配置并重启指定 Service。
    Restart {
        /// Service 稳定名称。
        service: ServiceName,
    },
    /// 停止指定 Service。
    Stop {
        /// Service 稳定名称。
        service: ServiceName,
    },
    /// 停止并移除远端 Service 注册，不删除 release。
    #[command(name = "rm")]
    Remove {
        /// Service 稳定名称。
        service: ServiceName,
    },
}

/// 解析当前项目记忆并执行一条远端 Procora 命令。
pub(super) fn run(arguments: RemoteArgs) -> anyhow::Result<()> {
    let remembered = if arguments.ssh.is_none()
        && !arguments.batch
        && env::var("PROCORA_SSH_TARGET")
            .ok()
            .is_none_or(|value| value.trim().is_empty())
    {
        current_project_target()
    } else {
        None
    };
    if let Some(target) = &remembered {
        eprintln!(
            "使用 Service `{}` 上次成功的部署目标：{}",
            target.project, target.ssh_target
        );
    }
    let ssh = arguments
        .ssh
        .or_else(|| remembered.as_ref().map(|target| target.ssh_target.clone()));
    let remote_bin = arguments
        .remote_bin
        .or_else(|| remembered.as_ref().map(|target| target.remote_bin.clone()));
    let command = command_arguments(arguments.command);
    let output = crate::transfer::run_remote_cli(
        ssh.as_deref(),
        remote_bin.as_deref(),
        arguments.batch,
        &command,
    )?;
    io::stdout().write_all(&output.stdout)?;
    io::stdout().flush()?;
    if !output.stderr.is_empty() {
        eprint!(
            "{}",
            crate::platform::decode_external_output(&output.stderr)
        );
    }
    Ok(())
}

/// 把领域已校验的操作转换为安全的远端参数。
fn command_arguments(command: RemoteCommand) -> Vec<String> {
    match command {
        RemoteCommand::Ps => vec!["list".to_owned()],
        RemoteCommand::Status => vec!["status".to_owned()],
        RemoteCommand::History { service } => {
            vec!["history".to_owned(), service.to_string()]
        }
        RemoteCommand::Logs { service, task } => {
            vec!["logs".to_owned(), service.to_string(), task.to_string()]
        }
        RemoteCommand::Start { service } => vec!["start".to_owned(), service.to_string()],
        RemoteCommand::Restart { service } => vec!["restart".to_owned(), service.to_string()],
        RemoteCommand::Stop { service } => vec!["stop".to_owned(), service.to_string()],
        RemoteCommand::Remove { service } => vec!["remove".to_owned(), service.to_string()],
    }
}

/// 从当前目录找到 Service 并读取它自己的部署目标记忆。
fn current_project_target() -> Option<super::deploy_memory::DeployTargetMemory> {
    let current = crate::platform::current_dir().ok()?;
    let discovered = crate::config::discover_path(&current).ok()?;
    load_target(&discovered.root, &discovered.compiled.spec.project)
}
