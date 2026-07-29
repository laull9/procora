#![cfg(unix)]

//! 裸机远端常用观察与控制命令的CLI回归测试。

use std::{fs, os::unix::fs::PermissionsExt, process::Command};

use crate::cli_uploads::temporary_directory;

/// 安装记录远端 `Procora` 参数并返回可识别输出的 `SSH` 替身。
fn install_remote_ssh(directory: &std::path::Path) {
    let script = r#"#!/bin/sh
printf '%s\n' "$*" >> "$REMOTE_SSH_LOG"
case "$*" in
  *" procora list")
    printf '%s\n' '名称	状态	任务	服务目录	配置文件'
    printf '%s\n' 'demo	运行中	1	/srv/demo	/srv/demo/procora.yaml'
    ;;
  *" procora logs demo api")
    printf '%s\n' 'api ready'
    ;;
  *)
    printf '%s\n' 'remote command ok'
    ;;
esac
"#;
    let path = directory.join("ssh");
    fs::write(&path, script).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

/// 构造使用测试 `SSH` 与隔离数据目录的远端命令。
fn remote_command(directory: &std::path::Path) -> Command {
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_procora"));
    command
        .env("PATH", path)
        .env("PROCORA_HOME", directory.join("home"))
        .env("REMOTE_SSH_LOG", directory.join("ssh.log"));
    command
}

#[test]
// remote ps和logs把经过校验的固定参数交给同一SSH远端。
fn remote_ps_and_logs_use_safe_procora_commands() {
    let directory = temporary_directory("remote-observe");
    install_remote_ssh(&directory);
    let ps = remote_command(&directory)
        .args(["remote", "ps", "--ssh", "remote-host"])
        .output()
        .unwrap();
    let logs = remote_command(&directory)
        .args(["remote", "--ssh", "remote-host", "logs", "demo", "api"])
        .output()
        .unwrap();

    assert!(
        ps.status.success(),
        "{}",
        String::from_utf8_lossy(&ps.stderr)
    );
    assert!(
        String::from_utf8_lossy(&ps.stdout).contains("demo\t运行中"),
        "{}",
        String::from_utf8_lossy(&ps.stdout)
    );
    assert!(logs.status.success());
    assert_eq!(String::from_utf8_lossy(&logs.stdout), "api ready\n");
    let invocations = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert!(invocations.contains("remote-host procora list"));
    assert!(invocations.contains("remote-host procora logs demo api"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// remote覆盖常用生命周期操作并把rm映射为现有remove语义。
fn remote_lifecycle_commands_map_to_existing_cli() {
    let directory = temporary_directory("remote-lifecycle");
    install_remote_ssh(&directory);
    for arguments in [
        vec!["remote", "status", "--ssh", "host"],
        vec!["remote", "history", "demo", "--ssh", "host"],
        vec!["remote", "start", "demo", "--ssh", "host"],
        vec!["remote", "restart", "demo", "--ssh", "host"],
        vec!["remote", "stop", "demo", "--ssh", "host"],
        vec!["remote", "rm", "demo", "--ssh", "host"],
    ] {
        let output = remote_command(&directory).args(arguments).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let invocations = fs::read_to_string(directory.join("ssh.log")).unwrap();
    for command in [
        "procora status",
        "procora history demo",
        "procora start demo",
        "procora restart demo",
        "procora stop demo",
        "procora remove demo",
    ] {
        assert!(invocations.contains(command), "缺少远端命令：{command}");
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 当前目录Service可自动复用它自己的成功部署目标。
fn remote_uses_current_service_deploy_target_memory() {
    let directory = temporary_directory("remote-memory");
    install_remote_ssh(&directory);
    let service = directory.join("service");
    let home = directory.join("home");
    fs::create_dir_all(home.join("cli-memory")).unwrap();
    fs::create_dir(&service).unwrap();
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    let root = procora::platform::canonicalize(&service).unwrap();
    fs::write(
        home.join("cli-memory/deploy.json"),
        serde_json::to_vec(&serde_json::json!({
            "entries": [{
                "root": root,
                "project": "demo",
                "ssh_target": "remembered-host",
                "remote_bin": "procora"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let output = remote_command(&directory)
        .args(["remote", "ps"])
        .current_dir(&service)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("remembered-host"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let invocation = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert!(invocation.contains("remembered-host procora list"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// Service与Task参数在启动SSH之前经过领域校验。
fn remote_rejects_unsafe_identifiers_before_ssh() {
    let directory = temporary_directory("remote-invalid");
    install_remote_ssh(&directory);
    let output = remote_command(&directory)
        .args(["remote", "logs", "demo;touch-pwned", "api", "--ssh", "host"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!directory.join("ssh.log").exists());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("只能包含 ASCII"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(directory).unwrap();
}
