//! 顶层服务命令、兼容入口和拼写建议测试。

use std::{path::PathBuf, process::Command as ProcessCommand};

use clap::Parser;
use procora::cli::{Cli, Command, ServerCommand};

#[test]
// 无子命令解析为当前目录tui入口。
fn no_subcommand_defaults_to_current_directory_tui() {
    let cli = Cli::try_parse_from(["procora"]).unwrap();
    assert!(cli.command.is_none());
    assert!(cli.target.is_none());
}

#[test]
// 顶层路径解析为指定服务tui入口。
fn top_level_path_opens_service_tui() {
    let cli = Cli::try_parse_from(["procora", "./services/demo"]).unwrap();

    assert!(cli.command.is_none());
    assert_eq!(cli.target, Some(PathBuf::from("./services/demo")));
}

#[test]
// show省略目标时默认使用当前目录。
fn show_without_target_uses_current_directory() {
    let cli = Cli::try_parse_from(["procora", "show"]).unwrap();

    assert!(matches!(
        cli.command,
        Some(Command::Show { target }) if target == "."
    ));
}

#[test]
// 唯一命令前缀可以直接推断。
fn unique_command_prefixes_are_inferred() {
    let status = Cli::try_parse_from(["procora", "stat"]).unwrap();
    let list = Cli::try_parse_from(["procora", "li"]).unwrap();

    assert!(matches!(status.command, Some(Command::Status)));
    assert!(matches!(list.command, Some(Command::List)));
}

#[test]
// 拼写错误会显示相近顶层命令和帮助入口。
fn typos_suggest_similar_top_level_commands() {
    for (typo, expected) in [("stats", "status"), ("lsst", "list")] {
        let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
            .arg(typo)
            .output()
            .unwrap();

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(&format!("procora {expected}")));
        assert!(stderr.contains("procora --help"));
    }
}

#[test]
// 服务管理命令全部位于顶层。
fn service_management_commands_are_top_level() {
    let add = Cli::try_parse_from(["procora", "add", "./demo"]).unwrap();
    let list = Cli::try_parse_from(["procora", "list"]).unwrap();
    let history = Cli::try_parse_from(["procora", "history", "demo"]).unwrap();
    let start = Cli::try_parse_from(["procora", "start", "demo"]).unwrap();
    let restart = Cli::try_parse_from(["procora", "restart", "demo"]).unwrap();
    let preview = Cli::try_parse_from(["procora", "preview", "demo"]).unwrap();
    let apply = Cli::try_parse_from(["procora", "apply", "demo", "abc123"]).unwrap();
    let stop = Cli::try_parse_from(["procora", "stop", "demo"]).unwrap();
    let remove = Cli::try_parse_from(["procora", "remove", "demo"]).unwrap();

    assert!(matches!(add.command, Some(Command::Add { .. })));
    assert!(matches!(list.command, Some(Command::List)));
    assert!(matches!(history.command, Some(Command::History { .. })));
    assert!(matches!(start.command, Some(Command::Start { .. })));
    assert!(matches!(restart.command, Some(Command::Restart { .. })));
    assert!(matches!(preview.command, Some(Command::Preview { .. })));
    assert!(matches!(apply.command, Some(Command::Apply { .. })));
    assert!(matches!(stop.command, Some(Command::Stop { .. })));
    assert!(matches!(remove.command, Some(Command::Remove { .. })));
}

#[test]
// mcp命令解析为本地标准输入输出服务入口。
fn mcp_command_is_top_level() {
    let cli = Cli::try_parse_from(["procora", "mcp"]).unwrap();

    assert!(matches!(cli.command, Some(Command::Mcp)));
}

#[test]
// 旧server层级保持隐藏兼容。
fn legacy_server_hierarchy_remains_compatible() {
    let cli = Cli::try_parse_from(["procora", "server", "remove", "demo"]).unwrap();
    let Some(Command::Server(arguments)) = cli.command else {
        panic!("应解析为 server 命令");
    };
    assert!(matches!(
        arguments.command,
        Some(ServerCommand::Remove { target }) if target == "demo"
    ));
}

#[test]
// 旧server拼写错误仍会得到兼容建议。
fn legacy_server_typos_keep_compatibility_suggestions() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .args(["server", "lsst"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("procora server list"));
}
