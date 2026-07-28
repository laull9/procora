#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, process::Command};

use crate::{
    cli_uploads::{install_fake_ssh, temporary_directory},
    command_support::remove_directory_when_released,
};

/// 安装把SSH调用转交给真实本地接收器的测试替身。
fn install_local_receiver_ssh(directory: &std::path::Path) {
    let script = r#"#!/bin/sh
printf '%s\n' "$*" > "$LOCAL_SSH_LOG"
export PROCORA_HOME="$PROCORA_REMOTE_HOME"
exec "$PROCORA_TEST_BINARY" __receive-deploy
"#;
    let path = directory.join("ssh");
    fs::write(&path, script).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
// deploy不协商声明式target并直接发送完整Service元数据。
fn deploy_uses_managed_receiver_without_remote_target() {
    let directory = temporary_directory("managed-deploy");
    install_fake_ssh(&directory);
    let source = directory.join("service");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("procora.yaml"),
        "version: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args([
            "deploy",
            source.to_str().unwrap(),
            "--ssh",
            "mock-host",
            "--batch",
            "--timeout",
            "1s",
            "--stable-for",
            "0ms",
        ])
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("部署完成：demo"));
    let invocation = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert!(invocation.contains("__receive-deploy"));
    let header = fs::read_to_string(directory.join("ssh-header.log")).unwrap();
    assert!(header.contains(r#""project":"demo""#));
    assert!(header.contains(r#""config_path":"procora.yaml""#));
    assert!(!header.contains(r#""target""#));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 远端版本缺少托管接收器时给出直接升级提示。
fn deploy_explains_when_remote_version_is_too_old() {
    let directory = temporary_directory("managed-deploy-old-remote");
    install_fake_ssh(&directory);
    let source = directory.join("service");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("procora.yaml"),
        "version: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args([
            "deploy",
            source.to_str().unwrap(),
            "--ssh",
            "old-host",
            "--batch",
        ])
        .env("PATH", path)
        .env("FAKE_SSH_MODE", "old-deploy")
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("请升级远端 Procora"), "{stderr}");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// deploy命令经SSH协议驱动真实接收器完成免target部署和阶段反馈。
fn deploy_cli_reaches_real_managed_receiver_end_to_end() {
    let directory = temporary_directory("managed-deploy-e2e");
    install_local_receiver_ssh(&directory);
    let source = directory.join("service");
    let remote_home = directory.join("remote-home");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("procora.yaml"),
        "version: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    fs::write(source.join("version.txt"), "real-e2e").unwrap();
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args([
            "deploy",
            source.to_str().unwrap(),
            "--ssh",
            "local-receiver",
            "--batch",
            "--timeout",
            "2s",
            "--stable-for",
            "0ms",
        ])
        .env("PATH", path)
        .env("LOCAL_SSH_LOG", directory.join("ssh.log"))
        .env("PROCORA_REMOTE_HOME", &remote_home)
        .env("PROCORA_TEST_BINARY", env!("CARGO_BIN_EXE_procora"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("部署完成：demo"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("[校验]"), "{stderr}");
    assert!(stderr.contains("[切换]"), "{stderr}");
    assert!(stderr.contains("[验活]"), "{stderr}");
    assert!(
        fs::read_to_string(directory.join("ssh.log"))
            .unwrap()
            .contains("__receive-deploy")
    );
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(remote_home.join("services/demo/state.json")).unwrap())
            .unwrap();
    let active = state["active_release"].as_str().unwrap();
    assert!(
        remote_home
            .join("services/demo/releases")
            .join(active)
            .join("version.txt")
            .is_file()
    );

    let stopped = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("down")
        .env("PROCORA_HOME", &remote_home)
        .output()
        .unwrap();
    assert!(stopped.status.success());
    remove_directory_when_released(&directory);
}
