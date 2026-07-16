//! Procora 命令行参数与中心服务器客户端入口。

mod center_runtime;
mod project;
mod runtime;
/// TUI 使用的全局与临时实时会话。
pub mod session;
mod source;
mod suggestion;
mod template;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use tracing_subscriber::EnvFilter;

/// Procora 顶层命令行参数。
#[derive(Debug, Parser)]
#[command(
    name = "procora",
    version,
    about = "本机任务服务管理器",
    infer_subcommands = true,
    subcommand_precedence_over_arg = true,
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// 要在 TUI 中打开的服务目录或配置文件；省略时使用当前目录。
    #[arg(value_name = "PATH")]
    pub target: Option<PathBuf>,
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
        /// 创建后不自动打开配置编辑页，适合脚本环境。
        #[arg(long)]
        no_edit: bool,
    },
    /// 打开配置引导与编辑页面。
    Edit {
        /// 配置文件或服务目录；省略时使用当前目录。
        path: Option<PathBuf>,
    },
    /// 同步或离线验证项目管理依赖。
    Deps {
        /// 配置文件或服务目录。
        #[arg(default_value = ".")]
        path: PathBuf,
        /// 只验证本地安装，不下载缺失依赖。
        #[arg(long)]
        check: bool,
    },
    /// 清空服务目录中的 `.procora` 运行时文件、日志和管理依赖缓存。
    Clean {
        /// 服务目录或配置文件；省略时使用当前目录。
        path: Option<PathBuf>,
    },
    /// 启动当前用户的全局 Procora 服务器。
    Up,
    /// 正常关闭当前用户的全局 Procora 服务器。
    Down,
    /// 显示全局 Procora 服务器的运行状态。
    Status,
    /// 注册并立即启动当前用户的开机自启动托管。
    Enable,
    /// 停止并移除当前用户的开机自启动托管。
    Disable,
    /// 注册、列出或管理本机托管服务。
    Server(ServerArgs),
    /// 获取并确认不会自动应用的外部任务定义候选。
    Source(SourceArgs),
    /// 打开指定名称或路径服务的 TUI。
    Show {
        /// 配置中的服务名称、服务目录或显式配置文件。
        target: String,
    },
    /// 解析配置并检查任务依赖图。
    Validate {
        /// 声明式配置、显式 `procora.py` 或可自动发现配置的目录。
        path: PathBuf,
    },
    /// 输出任务的启动拓扑顺序。
    Graph {
        /// 声明式配置、显式 `procora.py` 或可自动发现配置的目录。
        path: PathBuf,
    },
    /// 输出包含默认值与规范化路径的有效配置 JSON。
    Config {
        /// 声明式配置、显式 `procora.py` 或可自动发现配置的目录。
        path: PathBuf,
    },
    /// 检查当前平台基础能力。
    Doctor,
    /// 输出指定 shell 的命令补全脚本。
    Completions {
        /// 目标 shell。
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// 运行内部全局服务器进程。
    #[command(name = "__daemon", hide = true)]
    Daemon {
        /// 本地 IPC 端点名称。
        #[arg(long)]
        endpoint: String,
        /// 全局服务器 `SQLite` 状态数据库路径。
        #[arg(long)]
        database: PathBuf,
    },
}

/// `procora source` 的任务定义来源命令。
#[derive(Debug, Args)]
pub struct SourceArgs {
    /// 要使用的来源类型。
    #[command(subcommand)]
    pub command: SourceCommand,
}

/// 当前支持的外部任务定义来源。
#[derive(Debug, Subcommand)]
pub enum SourceCommand {
    /// 从 Git 仓库获取固定提交候选。
    Git {
        /// Git 候选操作。
        #[command(subcommand)]
        command: GitSourceCommand,
    },
}

/// Git 来源只读预览与重新确认命令。
#[derive(Debug, Subcommand)]
pub enum GitSourceCommand {
    /// 获取引用并输出不可变提交候选，不注册或启动 Task。
    Preview(GitDefinitionArgs),
    /// 重新获取并确认修订仍未变化，不注册或启动 Task。
    Confirm(GitConfirmArgs),
}

/// Git 来源仓库、引用、配置入口和缓存参数。
#[derive(Debug, Args)]
pub struct GitDefinitionArgs {
    /// HTTPS/SSH/SCP 仓库，配合 `--local` 时为本地仓库路径。
    pub repository: String,
    /// 分支、标签或完整提交引用。
    #[arg(long, default_value = "HEAD")]
    pub reference: String,
    /// 仓库内的相对声明式配置入口。
    #[arg(long, default_value = "procora.yaml")]
    pub config: PathBuf,
    /// 把 repository 显式视为可信本地仓库路径。
    #[arg(long)]
    pub local: bool,
    /// checkout 缓存目录；省略时使用当前用户 Procora 数据目录。
    #[arg(long)]
    pub cache: Option<PathBuf>,
}

/// Git 候选重新确认参数。
#[derive(Debug, Args)]
pub struct GitConfirmArgs {
    /// 与 preview 相同的来源参数。
    #[command(flatten)]
    pub definition: GitDefinitionArgs,
    /// `preview` 输出的完整组合修订。
    pub revision: String,
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

/// 全局 Procora 服务器支持的服务管理命令。
#[derive(Debug, Subcommand)]
pub enum ServerCommand {
    /// 列出全局 Procora 服务器中的全部服务。
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
    /// 预览候选修订及 Task 影响，不产生运行副作用。
    Preview {
        /// 配置中的服务名称、服务目录或显式配置文件。
        target: String,
    },
    /// 应用经过 preview 确认且内容没有变化的候选修订。
    Apply {
        /// 配置中的服务名称、服务目录或显式配置文件。
        target: String,
        /// `preview` 输出的完整 SHA-256 修订值。
        revision: String,
    },
    /// 停止指定名称或路径的服务。
    Stop {
        /// 配置中的服务名称、服务目录或显式配置文件。
        target: String,
    },
    /// 停止并从中心服务器注册表删除指定服务，不删除服务目录。
    Remove {
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
    if cli.command.is_none()
        && let Some(target) = &cli.target
        && let Some(suggestion) = suggestion::for_missing_path(target, suggestion::TOP_LEVEL)
    {
        anyhow::bail!(
            "未知命令 `{}`；是否要运行 `procora {suggestion}`？",
            target.display()
        );
    }
    runtime::dispatch(cli.command, cli.target.as_deref())
}

/// 初始化遵循 `RUST_LOG` 的结构化诊断输出。
fn initialize_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
