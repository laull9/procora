//! Procora 命令行二进制入口。

use std::process::ExitCode;

/// 启动 Procora 命令行入口并为运行期错误提供帮助入口。
fn main() -> ExitCode {
    match procora::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("错误：{error:#}\n\n运行 `procora --help` 查看用法。");
            ExitCode::FAILURE
        }
    }
}
