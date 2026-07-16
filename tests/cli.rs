//! Procora 二进制的命令行冒烟测试。

use std::{
    fs,
    path::PathBuf,
    process::Command as ProcessCommand,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::Parser;
use procora::cli::{Cli, Command, ServerCommand};

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-cli-test-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 返回仓库根目录中的基础配置夹具。
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic.yaml")
}

#[test]
fn 帮助命令可以执行() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("本机任务服务管理器"));
    assert!(stdout.contains("server"));
    assert!(stdout.contains("show"));
    assert!(stdout.contains("init"));
    assert!(stdout.contains("edit"));
    assert!(stdout.contains("deps"));
    assert!(stdout.contains("up"));
    assert!(stdout.contains("down"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("enable"));
    assert!(stdout.contains("disable"));
    assert!(stdout.contains("procora [PATH]"));
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
        let filename = format!("procora.{format}");
        let template = fs::read_to_string(directory.join(filename)).unwrap();
        assert!(!template.contains("command: cargo"));
        assert!(!template.contains("\"command\": \"cargo\""));
        assert!(!template.contains("command = \"cargo\""));

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
    assert!(String::from_utf8_lossy(&up.stdout).contains("全局 Procora：运行中"));

    let status = ProcessCommand::new(binary)
        .arg("status")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("全局 Procora：运行中"));

    let down = ProcessCommand::new(binary)
        .arg("down")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(down.status.success());
    assert!(String::from_utf8_lossy(&down.stdout).contains("全局 Procora：已停止"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn 离线服务列表和状态查询都不会启动全局服务() {
    let home = temporary_directory("offline-queries");
    let binary = env!("CARGO_BIN_EXE_procora");
    let list = ProcessCommand::new(binary)
        .args(["server", "list"])
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(list.status.success());
    assert_eq!(
        String::from_utf8_lossy(&list.stdout),
        "全局 Procora：未运行\n"
    );

    let status = ProcessCommand::new(binary)
        .arg("status")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert_eq!(
        String::from_utf8_lossy(&status.stdout),
        "全局 Procora：未运行\n"
    );
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
    assert!(cli.target.is_none());
}

#[test]
fn 顶层路径解析为指定服务tui入口() {
    let cli = Cli::try_parse_from(["procora", "./services/demo"]).unwrap();

    assert!(cli.command.is_none());
    assert_eq!(cli.target, Some(PathBuf::from("./services/demo")));
}

#[test]
fn 唯一命令前缀可以直接推断() {
    let status = Cli::try_parse_from(["procora", "stat"]).unwrap();
    let list = Cli::try_parse_from(["procora", "server", "li"]).unwrap();

    assert!(matches!(status.command, Some(Command::Status)));
    assert!(matches!(
        list.command,
        Some(Command::Server(arguments))
            if matches!(arguments.command, Some(ServerCommand::List))
    ));
}

#[test]
fn 拼写错误会显示相近命令和帮助入口() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .arg("stats")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("procora status"));
    assert!(stderr.contains("procora --help"));
}

#[test]
fn 参数拼写错误会显示相近参数和帮助入口() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .args(["init", "--froce"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--force"));
    assert!(stderr.contains("--help"));
}

#[test]
fn server拼写错误不会被静默当作路径() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .args(["server", "lsst"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("procora server list"));
    assert!(stderr.contains("procora --help"));
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

#[test]
fn edit与deps命令参数保持稳定() {
    let edit = Cli::try_parse_from(["procora", "edit", "./service"]).unwrap();
    let deps = Cli::try_parse_from(["procora", "deps", "./service", "--check"]).unwrap();

    assert!(matches!(
        edit.command,
        Some(Command::Edit { path: Some(path) }) if path == std::path::Path::new("./service")
    ));
    assert!(matches!(
        deps.command,
        Some(Command::Deps { path, check: true }) if path == std::path::Path::new("./service")
    ));
}

#[test]
fn deps命令可以同步并离线验证本地文件() {
    let directory = temporary_directory("deps");
    fs::write(directory.join("asset.bin"), "managed asset").unwrap();
    fs::write(
        directory.join("procora.yaml"),
        "version: 1\nproject: demo\ndependencies:\n  asset:\n    source: asset.bin\n    version: v1\n    unpack: never\n    kind: file\n    path: asset.bin\ntasks: {}\n",
    )
    .unwrap();
    let binary = env!("CARGO_BIN_EXE_procora");

    let sync = ProcessCommand::new(binary)
        .arg("deps")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(sync.status.success());
    assert!(String::from_utf8_lossy(&sync.stdout).contains("已安装 asset v1"));
    let check = ProcessCommand::new(binary)
        .args(["deps", "--check"])
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(check.status.success());
    assert!(String::from_utf8_lossy(&check.stdout).contains("已验证 asset v1"));
    fs::remove_dir_all(directory).unwrap();
}
