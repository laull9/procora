//! Procora 二进制的命令行冒烟测试。

use std::{
    fs,
    path::PathBuf,
    process::Command as ProcessCommand,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::Parser;
use procora::cli::{Cli, Command};

use crate::command_support::{remove_directory_when_released, run_background_cli};

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
// 帮助命令可以执行。
fn help_command_runs() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("本机任务服务管理器"));
    for command in [
        "add", "list", "history", "start", "restart", "preview", "apply", "stop", "remove",
    ] {
        assert!(stdout.contains(command), "帮助应显示 {command}");
    }
    assert!(
        !stdout
            .lines()
            .any(|line| line.trim_start().starts_with("server "))
    );
    assert!(stdout.contains("show"));
    assert!(stdout.contains("init"));
    assert!(stdout.contains("edit"));
    assert!(stdout.contains("deps"));
    assert!(stdout.contains("clean"));
    assert!(stdout.contains("push"));
    assert!(stdout.contains("up"));
    assert!(stdout.contains("down"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("enable"));
    assert!(stdout.contains("disable"));
    assert!(stdout.contains("completions"));
    assert!(stdout.contains("mcp"));
    let usage = if cfg!(windows) {
        "procora.exe [PATH]"
    } else {
        "procora [PATH]"
    };
    assert!(stdout.contains(usage));
}

#[test]
// init可以创建三种可校验模板。
fn init_creates_three_valid_templates() {
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
// init默认不覆盖已有配置。
fn init_does_not_overwrite_existing_config_by_default() {
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
// up_status_down形成中心进程闭环。
fn up_status_down_form_center_lifecycle() {
    let home = temporary_directory("center-lifecycle");
    let binary = env!("CARGO_BIN_EXE_procora");
    let up = run_background_cli(
        ProcessCommand::new(binary)
            .arg("up")
            .env("PROCORA_HOME", &home),
        &home,
        "up",
    );
    assert!(up.status.success());
    assert!(String::from_utf8_lossy(&up.stdout).contains("全局 Procora：运行中"));
    let installed = if cfg!(windows) {
        home.join("bin/procora.exe")
    } else {
        home.join("bin/procora")
    };
    assert!(installed.is_file());

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
    remove_directory_when_released(&home);
}

#[test]
// 离线服务列表和状态查询都不会启动全局服务。
fn offline_list_and_status_do_not_start_center() {
    let home = temporary_directory("offline-queries");
    let binary = env!("CARGO_BIN_EXE_procora");
    let list = ProcessCommand::new(binary)
        .arg("list")
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
// 可以校验基础配置。
fn validate_accepts_basic_config() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .arg("validate")
        .arg(fixture())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("共 2 个任务"));
}

#[test]
// 可以输出确定性任务图。
fn graph_output_is_deterministic() {
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
// 旧server帮助仍展示兼容命令。
fn legacy_server_help_lists_compatible_commands() {
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
    assert!(stdout.contains("preview"));
    assert!(stdout.contains("apply"));
    assert!(stdout.contains("stop"));
    assert!(stdout.contains("remove"));
}

#[test]
// 参数拼写错误会显示相近参数和帮助入口。
fn argument_typos_suggest_similar_options_and_help() {
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
// add和remove形成顶层服务管理闭环。
fn add_and_remove_form_top_level_service_lifecycle() {
    let home = temporary_directory("server-remove-home");
    let service = temporary_directory("server-remove-service");
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: removable\ntasks: {}\n",
    )
    .unwrap();
    let binary = env!("CARGO_BIN_EXE_procora");

    let opened = run_background_cli(
        ProcessCommand::new(binary)
            .arg("add")
            .arg(&service)
            .env("PROCORA_HOME", &home),
        &home,
        "add",
    );
    assert!(opened.status.success());
    let removed = ProcessCommand::new(binary)
        .args(["remove", "removable"])
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(removed.status.success());
    assert!(String::from_utf8_lossy(&removed.stdout).contains("已删除服务：removable"));
    assert!(service.join("procora.yaml").is_file());

    let listed = ProcessCommand::new(binary)
        .arg("list")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(listed.status.success());
    assert!(!String::from_utf8_lossy(&listed.stdout).contains("removable"));
    let down = ProcessCommand::new(binary)
        .arg("down")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(down.status.success());
    remove_directory_when_released(&home);
    fs::remove_dir_all(service).unwrap();
}

#[test]
// 相对服务路径必须按调用端目录解析，不能依赖后台中心的工作目录。
fn relative_add_uses_client_working_directory() {
    let home = temporary_directory("relative-add-home");
    let service = temporary_directory("relative-add-service");
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: relative-add\ntasks: {}\n",
    )
    .unwrap();
    let binary = env!("CARGO_BIN_EXE_procora");

    let up = run_background_cli(
        ProcessCommand::new(binary)
            .arg("up")
            .current_dir(&home)
            .env("PROCORA_HOME", &home),
        &home,
        "relative-add-up",
    );
    assert!(up.status.success());

    let opened = ProcessCommand::new(binary)
        .args(["add", "."])
        .current_dir(&service)
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(
        opened.status.success(),
        "相对路径注册失败\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&opened.stdout),
        String::from_utf8_lossy(&opened.stderr)
    );
    assert!(String::from_utf8_lossy(&opened.stdout).contains("relative-add"));

    let down = ProcessCommand::new(binary)
        .arg("down")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(down.status.success());
    remove_directory_when_released(&home);
    fs::remove_dir_all(service).unwrap();
}

#[test]
// 自启动命令保持顶层入口。
fn autostart_commands_remain_top_level() {
    let enable = Cli::try_parse_from(["procora", "enable"]).unwrap();
    let disable = Cli::try_parse_from(["procora", "disable"]).unwrap();

    assert!(matches!(enable.command, Some(Command::Enable)));
    assert!(matches!(disable.command, Some(Command::Disable)));
}

#[test]
// edit与deps命令参数保持稳定。
fn edit_and_deps_arguments_remain_stable() {
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
// clean命令参数保持稳定。
fn clean_arguments_remain_stable() {
    let current = Cli::try_parse_from(["procora", "clean"]).unwrap();
    let explicit = Cli::try_parse_from(["procora", "clean", "./service/procora.yaml"]).unwrap();

    assert!(matches!(
        current.command,
        Some(Command::Clean { path: None })
    ));
    assert!(matches!(
        explicit.command,
        Some(Command::Clean { path: Some(path) }) if path == std::path::Path::new("./service/procora.yaml")
    ));
}

#[test]
// push使用显式声明目标并允许配置SSH登录回退策略。
fn push_arguments_remain_stable() {
    let parsed = Cli::try_parse_from([
        "procora",
        "push",
        "./dist",
        "--target",
        "demo::api::release",
        "--ssh",
        "prod",
        "--batch",
    ])
    .unwrap();

    assert!(matches!(
        parsed.command,
        Some(Command::Push {
            source,
            target: Some(target),
            ssh: Some(ssh),
            remote_bin,
            batch: true,
        })
            if source == std::path::Path::new("./dist")
                && target == "demo::api::release"
                && ssh == "prod"
                && remote_bin == "procora"
    ));
}

#[test]
// push允许省略声明目标，由远端在同一SSH会话中发现目标。
fn push_target_is_optional_for_remote_discovery() {
    let parsed = Cli::try_parse_from([
        "procora",
        "push",
        "./dist",
        "--ssh",
        "prod",
        "--remote-bin",
        "~/.local/bin/procora",
    ])
    .unwrap();

    assert!(matches!(
        parsed.command,
        Some(Command::Push {
            target: None,
            remote_bin,
            ..
        }) if remote_bin == "~/.local/bin/procora"
    ));
}

#[test]
// clean只删除项目运行时目录且可重复执行。
fn clean_only_removes_runtime_directory_and_is_idempotent() {
    let directory = temporary_directory("clean");
    let runtime = directory.join(".procora/logs/tasks");
    fs::create_dir_all(&runtime).unwrap();
    fs::write(runtime.join("worker.log"), "runtime log").unwrap();
    fs::write(
        directory.join("procora.yaml"),
        "version: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    fs::write(directory.join("keep.txt"), "keep").unwrap();
    let binary = env!("CARGO_BIN_EXE_procora");

    let cleaned = ProcessCommand::new(binary)
        .arg("clean")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(cleaned.status.success());
    assert!(!directory.join(".procora").exists());
    assert!(directory.join("procora.yaml").exists());
    assert!(directory.join("keep.txt").exists());

    let repeated = ProcessCommand::new(binary)
        .arg("clean")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stdout).contains("没有需要清理"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// clean拒绝删除同名普通文件。
fn clean_rejects_regular_file_with_runtime_name() {
    let directory = temporary_directory("clean-file");
    let runtime = directory.join(".procora");
    fs::write(&runtime, "not a directory").unwrap();

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_procora"))
        .arg("clean")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(runtime.is_file());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// deps命令可以同步并离线验证本地文件。
fn deps_syncs_and_offline_checks_local_files() {
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
    assert!(
        sync.status.success(),
        "依赖同步失败\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&sync.stdout),
        String::from_utf8_lossy(&sync.stderr)
    );
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
