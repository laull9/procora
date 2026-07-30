#![cfg(unix)]

use std::{fs, io::Read, os::unix::fs::PermissionsExt, process::Command};

use flate2::read::GzDecoder;

use crate::{
    cli_uploads::{install_fake_ssh, temporary_directory},
    command_support::remove_directory_when_released,
};

/// 读取部署测试归档中的普通文件、正文和Unix模式。
fn archive_files(path: &std::path::Path) -> std::collections::BTreeMap<String, (Vec<u8>, u32)> {
    let decoder = GzDecoder::new(fs::File::open(path).unwrap());
    let mut archive = tar::Archive::new(decoder);
    let mut files = std::collections::BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.header().entry_type().is_file() {
            let path = entry.path().unwrap().to_string_lossy().into_owned();
            let mode = entry.header().mode().unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            files.insert(path, (bytes, mode));
        }
    }
    files
}
/// 安装把SSH调用转交给真实本地接收器的测试替身。
pub(super) fn install_local_receiver_ssh(directory: &std::path::Path) {
    let script = r#"#!/bin/sh
printf '%s\n' "$*" > "$LOCAL_SSH_LOG"
export PROCORA_HOME="$PROCORA_REMOTE_HOME"
case "$*" in
  *"__ssh-probe"*)
    exec "$PROCORA_TEST_BINARY" __ssh-probe
    ;;
esac
exec "$PROCORA_TEST_BINARY" __receive-deploy
"#;
    let path = directory.join("ssh");
    fs::write(&path, script).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}
/// 写入使用当前测试平台变体的真实可执行Service。
fn write_e2e_binary_service(source: &std::path::Path) {
    fs::create_dir_all(source.join("dist")).unwrap();
    let platform = procora::config::DeployPlatform::current().key();
    fs::write(
        source.join("procora.yaml"),
        format!(
            r#"version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      "{platform}": dist/api-current
tasks:
  api:
    command: "${{binary.api}}"
"#
        ),
    )
    .unwrap();
    fs::write(
        source.join("dist/api-current"),
        "#!/bin/sh\ntrap 'exit 0' TERM INT\nwhile :; do sleep 1; done\n",
    )
    .unwrap();
    fs::write(source.join("version.txt"), "real-e2e").unwrap();
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
// dry-run只探测平台和构造计划，不调用远端部署接收器。
fn deploy_dry_run_prints_plan_without_uploading() {
    let directory = temporary_directory("managed-deploy-dry-run");
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
            "preview-host",
            "--dry-run",
        ])
        .env("PATH", path)
        .env("PROCORA_HOME", directory.join("home"))
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
    assert!(stdout.contains("部署计划：demo → preview-host"), "{stdout}");
    assert!(stdout.contains("预检完成：未修改远端"), "{stdout}");
    assert!(stdout.contains("修订："), "{stdout}");
    let invocation = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert!(invocation.contains("__ssh-probe"));
    assert!(!invocation.contains("__receive-deploy"));
    assert!(!directory.join("ssh-header.log").exists());
    assert!(!directory.join("home/cli-memory/deploy.json").exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 同一Service成功部署后可省略SSH并复用该项目自己的目标。
fn deploy_remembers_successful_target_per_service() {
    let directory = temporary_directory("managed-deploy-memory");
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
    let home = directory.join("home");
    let first = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args([
            "deploy",
            source.to_str().unwrap(),
            "--ssh",
            "remembered-host",
        ])
        .env("PATH", &path)
        .env("PROCORA_HOME", &home)
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    fs::write(directory.join("ssh.log"), "").unwrap();
    let repeated = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["deploy", source.to_str().unwrap()])
        .env("PATH", path)
        .env("PROCORA_HOME", &home)
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
        .output()
        .unwrap();

    assert!(
        repeated.status.success(),
        "{}",
        String::from_utf8_lossy(&repeated.stderr)
    );
    let stderr = String::from_utf8_lossy(&repeated.stderr);
    assert!(
        stderr.contains("上次成功的部署目标：remembered-host"),
        "{stderr}"
    );
    let invocation = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert!(invocation.contains("remembered-host"), "{invocation}");
    let memory: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("cli-memory/deploy.json")).unwrap()).unwrap();
    assert_eq!(memory["entries"][0]["project"], "demo");
    assert_eq!(memory["entries"][0]["ssh_target"], "remembered-host");
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(home.join("cli-memory/deploy.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// deploy按远端平台只提交匹配二进制并映射到稳定target。
fn deploy_selects_and_submits_only_remote_platform_binary() {
    let directory = temporary_directory("managed-deploy-binary");
    install_fake_ssh(&directory);
    let source = directory.join("service");
    fs::create_dir_all(source.join("dist")).unwrap();
    fs::write(
        source.join("procora.yaml"),
        r"version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      linux-amd64: dist/api-linux-x86
      macos-arm64: dist/api-arm64-macos
tasks: {}
",
    )
    .unwrap();
    fs::write(source.join("dist/api-linux-x86"), b"linux-selected").unwrap();
    fs::write(source.join("dist/api-arm64-macos"), b"macos-not-selected").unwrap();
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let archive_path = directory.join("submitted.tar.gz");

    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args([
            "deploy",
            source.to_str().unwrap(),
            "--ssh",
            "mock-host",
            "--batch",
        ])
        .env("PATH", path)
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
        .env("FAKE_SSH_ARCHIVE_LOG", &archive_path)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("远端平台：linux-x86_64-gnu"), "{stdout}");
    assert!(stdout.contains("linux-x86_64"), "{stdout}");
    let files = archive_files(&archive_path);
    assert_eq!(files["bin/api"].0, b"linux-selected");
    assert_eq!(files["bin/api"].1 & 0o111, 0o111);
    assert!(!files.contains_key("dist/api-linux-x86"));
    assert!(!files.contains_key("dist/api-arm64-macos"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// macOS universal和Windows变体target经真实CLI归档选择后只提交对应产物。
fn deploy_archives_macos_universal_and_windows_executable_targets() {
    let directory = temporary_directory("managed-deploy-three-platforms");
    install_fake_ssh(&directory);
    let source = directory.join("service");
    fs::create_dir_all(source.join("dist")).unwrap();
    fs::write(
        source.join("procora.yaml"),
        r"version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      linux-amd64: dist/api-linux
      macos-universal: dist/api-macos-universal
      windows-amd64:
        source: dist/api-windows.exe
        target: bin/api.exe
tasks: {}
",
    )
    .unwrap();
    fs::write(source.join("dist/api-linux"), b"linux").unwrap();
    fs::write(source.join("dist/api-macos-universal"), b"macos-universal").unwrap();
    fs::write(source.join("dist/api-windows.exe"), b"windows").unwrap();
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let cases = [
        (
            r#"{"name":"procora-ssh","platform":{"os":"macos","arch":"aarch64"}}"#,
            "macos.tar.gz",
            "bin/api",
            b"macos-universal".as_slice(),
        ),
        (
            r#"{"name":"procora-ssh","platform":{"os":"windows","arch":"x86_64","environment":"msvc"}}"#,
            "windows.tar.gz",
            "bin/api.exe",
            b"windows".as_slice(),
        ),
    ];
    for (platform, archive_name, target, expected) in cases {
        let archive_path = directory.join(archive_name);
        let output = Command::new(env!("CARGO_BIN_EXE_procora"))
            .args([
                "deploy",
                source.to_str().unwrap(),
                "--ssh",
                "mock-host",
                "--batch",
            ])
            .env("PATH", &path)
            .env("FAKE_SSH_PLATFORM", platform)
            .env("FAKE_SSH_LOG", directory.join("ssh.log"))
            .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
            .env("FAKE_SSH_ARCHIVE_LOG", &archive_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let files = archive_files(&archive_path);
        assert_eq!(files[target].0, expected);
        assert_eq!(
            files.keys().filter(|path| path.starts_with("bin/")).count(),
            1
        );
        assert!(!files.keys().any(|path| path.starts_with("dist/")));
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 缺少远端平台变体时在构造归档和调用接收器前失败。
fn deploy_rejects_missing_remote_platform_binary_before_upload() {
    let directory = temporary_directory("managed-deploy-missing-binary");
    install_fake_ssh(&directory);
    let source = directory.join("service");
    fs::create_dir_all(source.join("dist")).unwrap();
    fs::write(
        source.join("procora.yaml"),
        r"version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      macos-arm64: dist/api-arm64-macos
tasks: {}
",
    )
    .unwrap();
    fs::write(source.join("dist/api-arm64-macos"), b"macos").unwrap();
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
            "linux-host",
            "--batch",
        ])
        .env("PATH", path)
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"))
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("linux-x86_64-gnu"), "{stderr}");
    assert!(stderr.contains("macos-aarch64"), "{stderr}");
    assert!(
        !fs::read_to_string(directory.join("ssh.log"))
            .unwrap()
            .contains("__receive-deploy")
    );
    assert!(!directory.join("ssh-header.log").exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
// 能力握手声明旧部署协议时在归档上传和远端切换前给出升级提示。
fn deploy_rejects_incompatible_protocol_during_preflight() {
    let directory = temporary_directory("managed-deploy-protocol");
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
        .env(
            "FAKE_SSH_PLATFORM",
            r#"{"name":"procora-ssh","platform":{"os":"linux","arch":"x86_64"},"deploy_protocol":{"min":1,"max":1}}"#,
        )
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("协议为 1–1"), "{stderr}");
    assert!(
        !fs::read_to_string(directory.join("ssh.log"))
            .unwrap()
            .contains("__receive-deploy")
    );
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
    write_e2e_binary_service(&source);
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
    let release = state["releases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|release| release["id"] == active)
        .unwrap();
    assert_eq!(release["binaries"][0]["name"], "api");
    assert_eq!(release["binaries"][0]["target"], "bin/api");
    assert_eq!(
        release["target_platform"]["os"],
        procora::config::DeployPlatform::current().os
    );
    assert!(
        remote_home
            .join("services/demo/releases")
            .join(active)
            .join("bin/api")
            .is_file()
    );
    assert!(
        !remote_home
            .join("services/demo/releases")
            .join(active)
            .join("dist/api-current")
            .exists()
    );
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
