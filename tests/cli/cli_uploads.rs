#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

/// 创建当前测试独占的目录。
pub(super) fn temporary_directory(label: &str) -> PathBuf {
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

/// 实现单连接协商、平台探测和失败注入的ssh测试替身。
const FAKE_SSH_SCRIPT: &str = r#"#!/bin/sh
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
  remote-common)
    case "$*" in
      *"__PROCORA_PATH__"*)
        printf '%s\n' '__PROCORA_PATH__/home/mock/.local/bin/procora'
        exit 0
        ;;
      *" /home/mock/.local/bin/procora "*)
        ;;
      *)
        printf '%s\n' 'procora: not found' >&2
        exit 127
        ;;
    esac
    ;;
esac
case "$*" in
  *"__ssh-probe"*)
    if [ -n "$FAKE_SSH_PLATFORM" ]; then
      printf '%s\n' "$FAKE_SSH_PLATFORM"
    else
      printf '%s\n' '{"name":"procora-ssh","platform":{"os":"linux","arch":"x86_64","environment":"gnu"}}'
    fi
    exit 0
    ;;
  *"__receive-deploy"*)
    if [ "$FAKE_SSH_MODE" = "old-deploy" ]; then
      printf '%s\n' "error: unrecognized subcommand '__receive-deploy'" >&2
      exit 2
    fi
    IFS= read -r header || exit 1
    printf '%s\n' "$header" > "$FAKE_SSH_HEADER_LOG"
    printf '%s\n' '{"type":"ready","project":"demo"}'
    archive_bytes=$(printf '%s' "$header" | sed -n 's/.*"archive_bytes":\([0-9][0-9]*\).*/\1/p')
    if [ -n "$FAKE_SSH_ARCHIVE_LOG" ]; then
      dd bs=1 count="$archive_bytes" of="$FAKE_SSH_ARCHIVE_LOG" 2>/dev/null
    else
      dd bs=1 count="$archive_bytes" >/dev/null 2>&1
    fi
    printf '%s\n' '{"type":"complete","result":{"project":"demo","release":"0123456789abcdef","previous_release":null,"content_bytes":42,"sha256":"fixture"}}'
    exit 0
    ;;
  *"__upload-targets"*)
    printf '%s\n' '[{"selector":"demo::release","path":"bin/release","kind":"file","max_bytes":20000000,"restart":true},{"selector":"demo::assets","path":"public","kind":"directory","max_bytes":1073741824,"restart":false}]'
    exit 0
    ;;
esac
IFS= read -r header || exit 1
printf '%s\n' "$header" > "$FAKE_SSH_HEADER_LOG"
case "$FAKE_SSH_MODE" in
  protocol-v1)
    printf '%s\n' '错误：不支持上传协议版本 2，当前为 1' >&2
    exit 1
    ;;
  target-missing)
    case "$header" in
      *'"target":null'*)
        ;;
      *)
        printf '%s\n\n%s\n' '错误：找不到服务 `missing`' '运行 `procora --help` 查看用法。' >&2
        exit 1
        ;;
    esac
    ;;
esac
case "$FAKE_SSH_MODE" in
  choose-v1)
    printf '%s\n' '{"type":"choose","targets":[{"selector":"demo::assets","kind":"directory","max_bytes":1024},{"selector":"demo::api::release","kind":"directory","max_bytes":2048}]}'
    IFS= read -r selection || exit 0
    printf '%s\n' '{"type":"ready","target":"demo::assets"}'
    ;;
  choose)
    printf '%s\n' '{"type":"choose","targets":[{"selector":"demo::assets","path":"public","kind":"directory","max_bytes":1024},{"selector":"demo::api::release","path":"releases/api","kind":"directory","max_bytes":2048}]}'
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
case "$header" in
  *'"restart":true'*)
    printf '%s\n' '{"type":"complete","result":{"target":"demo::release","path":"/srv/demo/release","content_bytes":7,"sha256":"fixture","restarted":true}}'
    ;;
  *)
    printf '%s\n' '{"type":"complete","result":{"target":"demo::release","path":"/srv/demo/release","content_bytes":7,"sha256":"fixture","restarted":false}}'
    ;;
esac
"#;

/// 安装实现单连接协商协议的ssh测试替身。
pub(super) fn install_fake_ssh(directory: &std::path::Path) {
    let path = directory.join("ssh");
    fs::write(&path, FAKE_SSH_SCRIPT).unwrap();
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
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"));
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
    assert!(
        fs::read_to_string(directory.join("ssh-header.log"))
            .unwrap()
            .contains(r#""protocol":1"#)
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
// 新客户端可解析旧协议不含路径和重启元数据的候选清单。
fn batch_push_accepts_legacy_target_metadata() {
    let directory = temporary_directory("legacy-targets");
    install_fake_ssh(&directory);
    let source = directory.join("folder");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("payload.txt"), "payload").unwrap();

    let output = push_command(&directory, &source)
        .arg("--batch")
        .env("FAKE_SSH_MODE", "choose-v1")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("远端有多个兼容上传目标"));
    assert!(!stderr.contains("无效上传协商消息"));
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
// 自动认证失败时保留既定SSH地址，非终端环境提示改用密钥或交互终端。
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
    assert!(stderr.contains("SSH 密钥自动登录不可用"));
    assert!(stderr.contains("SSH 密码登录需要交互终端"));
    assert!(stderr.contains("`mock-host`"));
    assert!(!stderr.contains("SSH 目标 ["));
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
    assert!(stderr.contains("已检查远端 PATH 与常见安装位置"));
    assert!(!stderr.contains("SSH 自动登录失败"));
    assert!(
        fs::read_to_string(directory.join("ssh.log"))
            .unwrap()
            .lines()
            .count()
            > 1
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 旧远端拒绝无效选择器时，交互push重新连接并拉取兼容目标而不直接退出。
fn invalid_target_falls_back_to_remote_candidates() {
    let directory = temporary_directory("invalid-target");
    install_fake_ssh(&directory);
    let source = directory.join("payload.txt");
    fs::write(&source, "payload").unwrap();

    let output = push_command(&directory, &source)
        .args(["--target", "missing::release"])
        .env("FAKE_SSH_MODE", "target-missing")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("远端没有上传目标 `missing::release`，正在读取可用列表")
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("上传完成"));
    assert_eq!(
        fs::read_to_string(directory.join("ssh.log"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// batch模式下无效选择器保持确定性失败并解释SSH地址与服务选择器的区别。
fn batch_invalid_target_does_not_choose_another_destination() {
    let directory = temporary_directory("batch-invalid-target");
    install_fake_ssh(&directory);
    let source = directory.join("payload.txt");
    fs::write(&source, "payload").unwrap();

    let output = push_command(&directory, &source)
        .args(["--target", "missing::release", "--batch"])
        .env("FAKE_SSH_MODE", "target-missing")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`missing` 是远端 Procora 服务名，不是 SSH 地址"));
    assert_eq!(
        stderr.matches("运行 `procora --help` 查看用法。").count(),
        1
    );
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
// PATH缺少Procora时自动扫描常见位置并用能力握手确认后上传。
fn push_discovers_procora_in_common_remote_location() {
    let directory = temporary_directory("remote-common");
    install_fake_ssh(&directory);
    let source = directory.join("payload.txt");
    fs::write(&source, "payload").unwrap();

    let output = push_command(&directory, &source)
        .args(["--target", "demo::release", "--batch"])
        .env("FAKE_SSH_MODE", "remote-common")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("已自动找到远端 Procora：/home/mock/.local/bin/procora")
    );
    let log = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert!(log.contains("/home/mock/.local/bin/procora __receive"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// restart参数进入上传协议且成功结果明确反馈自动重启。
fn push_restart_is_explicit_and_reported() {
    let directory = temporary_directory("restart");
    install_fake_ssh(&directory);
    let source = directory.join("payload.txt");
    fs::write(&source, "payload").unwrap();

    let output = push_command(&directory, &source)
        .args(["--target", "demo::release", "--batch", "--restart"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("已自动重启：demo"));
    assert!(
        fs::read_to_string(directory.join("ssh-header.log"))
            .unwrap()
            .contains(r#""protocol":2"#)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 旧远端缺少显式重启能力时给出升级或兼容覆盖选择。
fn push_restart_reports_remote_capability_mismatch() {
    let directory = temporary_directory("restart-protocol");
    install_fake_ssh(&directory);
    let source = directory.join("payload.txt");
    fs::write(&source, "payload").unwrap();

    let output = push_command(&directory, &source)
        .args(["--target", "demo::release", "--batch", "--restart"])
        .env("FAKE_SSH_MODE", "protocol-v1")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("不支持客户端请求的上传后重启能力"));
    assert!(stderr.contains("移除 `--restart`"));
    fs::remove_dir_all(directory).unwrap();
}
