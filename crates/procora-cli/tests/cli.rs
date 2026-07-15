//! Procora 二进制的命令行冒烟测试。

use std::{
    fs,
    path::PathBuf,
    process::Command as ProcessCommand,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::Parser;
use procora_cli::{Cli, Command, ServerCommand};

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-cli-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 返回仓库根目录中的基础配置夹具。
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/basic.yaml")
}

#[test]
fn 帮助命令可以执行() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("以中心服务器托管本机任务服务"));
    assert!(stdout.contains("server"));
    assert!(stdout.contains("show"));
    assert!(stdout.contains("init"));
    assert!(stdout.contains("up"));
    assert!(stdout.contains("down"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("enable"));
    assert!(stdout.contains("disable"));
}

#[test]
fn init可以创建三种可校验模板() {
    for format in ["yaml", "json", "toml"] {
        let directory = temporary_directory(format);
        let initialized = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
            .args(["init", "--config", format])
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(initialized.status.success(), "{format} 模板创建失败");

        let validated = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
            .args(["validate", "."])
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(validated.status.success(), "{format} 模板校验失败");
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn init默认不覆盖已有配置() {
    let directory = temporary_directory("no-overwrite");
    fs::write(directory.join("procora.yaml"), "用户内容").unwrap();
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .arg("init")
        .current_dir(&directory)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(directory.join("procora.yaml")).unwrap(),
        "用户内容"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn up_status_down形成中心进程闭环() {
    let home = temporary_directory("center-lifecycle");
    let binary = env!("CARGO_BIN_EXE_procora");
    let up = ProcessCommand::new(binary)
        .arg("up")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(up.status.success());
    assert!(String::from_utf8_lossy(&up.stdout).contains("中心服务器：运行中"));

    let status = ProcessCommand::new(binary)
        .arg("status")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("控制：允许"));

    let down = ProcessCommand::new(binary)
        .arg("down")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(down.status.success());
    assert!(String::from_utf8_lossy(&down.stdout).contains("中心服务器已停止"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn 可以校验基础配置() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .arg("validate")
        .arg(fixture())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("共 2 个任务"));
}

#[test]
fn 可以输出确定性任务图() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .arg("graph")
        .arg(fixture())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "1. database\n2. api\n"
    );
}

#[test]
fn server帮助展示高频生命周期命令() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .args(["server", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("list"));
    assert!(stdout.contains("history"));
    assert!(stdout.contains("start"));
    assert!(stdout.contains("restart"));
    assert!(stdout.contains("stop"));
}

#[test]
fn 无子命令解析为当前目录tui入口() {
    let cli = Cli::try_parse_from(["procora"]).unwrap();
    assert!(cli.command.is_none());
}

#[test]
fn 服务生命周期命令保持稳定层级() {
    let cli = Cli::try_parse_from(["procora", "server", "restart", "demo"]).unwrap();
    let Some(Command::Server(arguments)) = cli.command else {
        panic!("应解析为 server 命令");
    };
    assert!(matches!(
        arguments.command,
        Some(ServerCommand::Restart { target }) if target == "demo"
    ));
}

#[test]
fn 自启动命令保持顶层入口() {
    let enable = Cli::try_parse_from(["procora", "enable"]).unwrap();
    let disable = Cli::try_parse_from(["procora", "disable"]).unwrap();

    assert!(matches!(enable.command, Some(Command::Enable)));
    assert!(matches!(disable.command, Some(Command::Disable)));
}
