//! Procora 命令行参数与中心服务器客户端入口。

mod runtime;
mod session;
mod template;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use tracing_subscriber::EnvFilter;

/// Procora 顶层命令行参数。
#[derive(Debug, Parser)]
#[command(
    name = "procora",
    version,
    about = "以中心服务器托管本机任务服务的 TUI 管理器"
)]
pub struct Cli {
    /// 要执行的子命令；省略时在当前目录打开 TUI。
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Procora 的顶层命令集合。
#[derive(Debug, Subcommand)]
pub enum Command {
    /// 在当前目录创建一个可直接校验的示例服务项目。
    Init {
        /// 模板配置格式。
        #[arg(long, value_enum, default_value_t = TemplateFormat::Yaml)]
        config: TemplateFormat,
        /// 允许覆盖同名配置文件。
        #[arg(long)]
        force: bool,
    },
    /// 启动当前用户唯一的中心后台服务器。
    Up,
    /// 正常关闭当前用户的中心后台服务器。
    Down,
    /// 显示中心后台服务器的运行状态与身份。
    Status,
    /// 注册并立即启动当前用户的开机自启动托管。
    Enable,
    /// 停止并移除当前用户的开机自启动托管。
    Disable,
    /// 注册、列出或管理本机托管服务。
    Server(ServerArgs),
    /// 打开指定名称或路径服务的 TUI。
    Show {
        /// 配置中的服务名称、服务目录或显式配置文件。
        target: String,
    },
    /// 解析配置并检查任务依赖图。
    Validate {
        /// YAML、TOML、JSON 配置文件或可自动发现配置的目录。
        path: PathBuf,
    },
    /// 输出任务的启动拓扑顺序。
    Graph {
        /// YAML、TOML、JSON 配置文件或可自动发现配置的目录。
        path: PathBuf,
    },
    /// 检查当前平台基础能力。
    Doctor,
    /// 运行内部中心服务器进程。
    #[command(name = "__daemon", hide = true)]
    Daemon {
        /// 本地 IPC 端点名称。
        #[arg(long)]
        endpoint: String,
        /// 中心服务器 `SQLite` 状态数据库路径。
        #[arg(long)]
        database: PathBuf,
    },
}

/// `procora server` 的路径参数与嵌套管理命令。
#[derive(Debug, Args)]
pub struct ServerArgs {
    /// 不带管理子命令时要注册并启动的目录或配置文件。
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,
    /// 对注册服务执行的管理命令。
    #[command(subcommand)]
    pub command: Option<ServerCommand>,
}

/// 中心服务器支持的服务管理命令。
#[derive(Debug, Subcommand)]
pub enum ServerCommand {
    /// 列出本机中心服务器中的全部服务。
    List,
    /// 列出指定服务的持久化状态历史。
    History {
        /// 配置中的服务名称、服务目录或显式配置文件。
        target: String,
    },
    /// 启动指定名称或路径的服务。
    Start {
        /// 配置中的服务名称、服务目录或显式配置文件。
        target: String,
    },
    /// 重新加载配置并重启指定服务。
    Restart {
        /// 配置中的服务名称、服务目录或显式配置文件。
        target: String,
    },
    /// 停止指定名称或路径的服务。
    Stop {
        /// 配置中的服务名称、服务目录或显式配置文件。
        target: String,
    },
}

/// `procora init` 支持的模板配置格式。
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum TemplateFormat {
    /// 创建 `procora.yaml`。
    Yaml,
    /// 创建 `procora.json`。
    Json,
    /// 创建 `procora.toml`。
    Toml,
}

/// 解析进程参数并执行对应命令。
///
/// # Errors
///
/// 当配置加载、中心服务器连接或 TUI 终端操作失败时返回错误。
pub fn run() -> anyhow::Result<()> {
    run_with(Cli::parse())
}

/// 执行已完成解析的命令，便于测试和嵌入。
///
/// # Errors
///
/// 当配置加载、中心服务器连接或 TUI 终端操作失败时返回错误。
pub fn run_with(cli: Cli) -> anyhow::Result<()> {
    initialize_tracing();
    runtime::dispatch(cli.command)
}

/// 初始化遵循 `RUST_LOG` 的结构化诊断输出。
fn initialize_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
