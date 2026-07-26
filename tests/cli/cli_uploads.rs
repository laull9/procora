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

#[test]
// uploads可通过SSH获取并展示远端活动选择器、类型、上限与声明路径。
fn uploads_lists_remote_targets_and_paths() {
    let directory = temporary_directory("list-targets");
    install_fake_ssh(&directory);
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["uploads", "--ssh", "mock-host", "--batch"])
        .env("PATH", path)
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("demo::release"));
    assert!(stdout.contains("bin/release"));
    assert!(stdout.contains("demo::assets"));
    assert!(stdout.contains("public"));
    assert!(stdout.contains("是"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// uploads查询与push共享远端Procora常见位置发现能力。
fn uploads_discovers_procora_in_common_remote_location() {
    let directory = temporary_directory("list-remote-common");
    install_fake_ssh(&directory);
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["uploads", "--ssh", "mock-host", "--batch"])
        .env("PATH", path)
        .env("FAKE_SSH_MODE", "remote-common")
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("demo::release"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 成功push只把非敏感记忆写入全局Procora子目录。
fn push_memory_is_saved_under_global_procora_home() {
    let directory = temporary_directory("memory");
    let procora_home = temporary_directory("memory-home");
    install_fake_ssh(&directory);
    let source = directory.join("payload.txt");
    fs::write(&source, "payload").unwrap();

    let output = push_command(&directory, &source)
        .args(["--target", "demo::release"])
        .env("PROCORA_HOME", &procora_home)
        .output()
        .unwrap();

    assert!(output.status.success());
    let memory = procora_home.join("cli-memory/push.json");
    assert!(memory.is_file());
    assert_eq!(
        fs::metadata(&memory).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let value: serde_json::Value = serde_json::from_slice(&fs::read(memory).unwrap()).unwrap();
    assert_eq!(value["ssh_target"], "mock-host");
    assert_eq!(value["upload_target"], "demo::release");
    assert!(!directory.join(".procora").exists());
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(procora_home).unwrap();
}

#[test]
// SSH探测返回机器可读的协议范围与能力而不是包版本硬匹配。
fn ssh_probe_reports_protocol_capabilities() {
    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("__ssh-probe")
        .output()
        .unwrap();

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["transfer_protocol"]["min"], 1);
    assert_eq!(value["transfer_protocol"]["max"], 2);
    assert!(
        value["capabilities"]
            .as_array()
            .unwrap()
            .contains(&serde_json::Value::String("configured_restart".to_owned()))
    );
}
