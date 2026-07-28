#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, process::Command};

use super::cli_uploads::{install_fake_ssh, temporary_directory};

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
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args([
            "push",
            source.to_str().unwrap(),
            "--ssh",
            "mock-host",
            "--target",
            "demo::release",
        ])
        .env("PATH", path)
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
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
