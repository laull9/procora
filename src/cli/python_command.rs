//! 嵌入式 Python API 的安装与状态命令。

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// `procora python` 的嵌套参数。
#[derive(Debug, Args)]
pub struct PythonArgs {
    /// 要执行的 Python API 操作。
    #[command(subcommand)]
    command: PythonCommand,
}

/// Python API 支持的安装操作。
#[derive(Debug, Subcommand)]
enum PythonCommand {
    /// 安装内嵌 Python 包并写入当前解释器用户 site-packages。
    Install {
        /// 要关联的 Python 3 解释器。
        #[arg(long, value_name = "PROGRAM")]
        interpreter: Option<PathBuf>,
        /// 成功时不输出路径，供安装脚本使用。
        #[arg(long)]
        quiet: bool,
    },
    /// 输出内嵌 Python 包路径，不修改 site-packages。
    Path,
    /// 移除当前解释器的 Procora `.pth` 文件。
    Uninstall {
        /// 要解除关联的 Python 3 解释器。
        #[arg(long, value_name = "PROGRAM")]
        interpreter: Option<PathBuf>,
        /// 成功时不输出路径，供卸载脚本使用。
        #[arg(long)]
        quiet: bool,
    },
}

/// 执行 Python API 安装操作。
pub(super) fn run(arguments: PythonArgs) -> anyhow::Result<()> {
    match arguments.command {
        PythonCommand::Install { interpreter, quiet } => {
            let report = crate::python::install(interpreter)?;
            if !quiet {
                println!("Procora Python API：{}", report.package_root.display());
                println!("Python 路径文件：{}", report.path_file.display());
            }
        }
        PythonCommand::Path => println!("{}", crate::python::ensure_package()?.display()),
        PythonCommand::Uninstall { interpreter, quiet } => {
            let removed = crate::python::uninstall(interpreter)?;
            if !quiet {
                match removed {
                    Some(path) => println!("已移除 Python 路径文件：{}", path.display()),
                    None => println!("当前 Python 未安装 Procora API 路径文件"),
                }
            }
        }
    }
    Ok(())
}
