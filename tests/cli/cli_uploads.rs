#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

/// 创建当前测试独占的目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-cli-upload-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 安装实现单连接协商协议的 ssh 测试替身。
fn install_fake_ssh(directory: &std::path::Path) {
    let script = r#"#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_SSH_LOG"
case "$FAKE_SSH_MODE" in
  auth-failure)
    printf '%s\n' 'Permission denied' >&2
    exit 255
    ;;
  remote-missing)
    printf '%s\n' 'procora: not found' >&2
    exit 127
    ;;
esac
IFS= read -r header || exit 1
case "$FAKE_SSH_MODE" in
  choose)
    printf '%s\n' '{"type":"choose","targets":[{"selector":"demo::assets","kind":"directory","max_bytes":1024},{"selector":"demo::api::release","kind":"directory","max_bytes":2048}]}'
    IFS= read -r selection || exit 0
    printf '%s\n' '{"type":"ready","target":"demo::assets"}'
    ;;
  auto)
    printf '%s\n' '{"type":"ready","target":"demo::assets"}'
    ;;
  *)
    printf '%s\n' '{"type":"ready","target":"demo::release"}'
    ;;
esac
archive_bytes=$(printf '%s' "$header" | sed -n 's/.*"archive_bytes":\([0-9][0-9]*\).*/\1/p')
dd bs=1 count="$archive_bytes" >/dev/null 2>&1
printf '%s\n' '{"type":"complete","result":{"target":"demo::release","path":"/srv/demo/release","content_bytes":7,"sha256":"fixture"}}'
"#;
    let path = directory.join("ssh");
    fs::write(&path, script).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

/// 构造使用测试 ssh 的 push 命令。
fn push_command(directory: &std::path::Path, source: &std::path::Path) -> Command {
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_procora"));
    command
        .args(["push", source.to_str().unwrap(), "--ssh", "mock-host"])
        .env("PATH", path)
        .env("FAKE_SSH_LOG", directory.join("ssh.log"));
    command
}

#[test]
// 自动登录、目标协商和上传只建立一条SSH连接。
fn push_uses_one_automatic_ssh_session() {
    let directory = temporary_directory("automatic");
    install_fake_ssh(&directory);
    let source = directory.join("payload.txt");
    fs::write(&source, "payload").unwrap();

    let output = push_command(&directory, &source)
        .args(["--target", "demo::release", "--batch"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("上传完成"));
    assert_eq!(
        fs::read_to_string(directory.join("ssh.log"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 省略target且远端只有一个兼容目标时自动选择。
fn push_automatically_selects_the_only_remote_target() {
    let directory = temporary_directory("auto-target");
    install_fake_ssh(&directory);
    let source = directory.join("folder");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("payload.txt"), "payload").unwrap();

    let output = push_command(&directory, &source)
        .arg("--batch")
        .env("FAKE_SSH_MODE", "auto")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("使用远端上传目标：demo::assets"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 非交互模式遇到多个兼容目标时列出选择器而不擅自覆盖。
fn batch_push_lists_multiple_targets_and_requires_selection() {
    let directory = temporary_directory("multiple-targets");
    install_fake_ssh(&directory);
    let source = directory.join("folder");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("payload.txt"), "payload").unwrap();

    let output = push_command(&directory, &source)
        .arg("--batch")
        .env("FAKE_SSH_MODE", "choose")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("远端有多个兼容上传目标"));
    assert!(stderr.contains("demo::assets"));
    assert!(stderr.contains("--target"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// batch模式下自动认证失败会给出可操作错误且不会等待密码。
fn batch_push_reports_automatic_login_failure_without_prompting() {
    let directory = temporary_directory("batch-failure");
    install_fake_ssh(&directory);
    let source = directory.join("payload.txt");
    fs::write(&source, "payload").unwrap();

    let output = push_command(&directory, &source)
        .args(["--target", "demo::release", "--batch"])
        .env("FAKE_SSH_MODE", "auth-failure")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("SSH 自动登录失败"));
    assert!(stderr.contains("Permission denied"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 自动认证失败时进入人工回退边界，非终端环境则提示显式修正地址或密钥。
fn automatic_login_failure_attempts_manual_fallback() {
    let directory = temporary_directory("manual-fallback");
    install_fake_ssh(&directory);
    let source = directory.join("payload.txt");
    fs::write(&source, "payload").unwrap();

    let output = push_command(&directory, &source)
        .args(["--target", "demo::release"])
        .env("FAKE_SSH_MODE", "auth-failure")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("SSH 自动登录失败"));
    assert!(stderr.contains("当前不是交互终端"));
    assert!(stderr.contains("--ssh <目标>"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 远端命令缺失不是认证问题，不进入密码登录回退。
fn remote_command_failure_does_not_trigger_login_fallback() {
    let directory = temporary_directory("remote-missing");
    install_fake_ssh(&directory);
    let source = directory.join("payload.txt");
    fs::write(&source, "payload").unwrap();

    let output = push_command(&directory, &source)
        .args(["--target", "demo::release"])
        .env("FAKE_SSH_MODE", "remote-missing")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--remote-bin ~/.local/bin/procora"));
    assert!(!stderr.contains("当前不是交互终端"));
    assert_eq!(
        fs::read_to_string(directory.join("ssh.log"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    fs::remove_dir_all(directory).unwrap();
}
