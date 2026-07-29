#![cfg(unix)]

//! 裸机部署在极端路径、产物与`SSH`握手环境下的回归测试。

use std::{fs, io::Read, os::unix::fs::symlink, process::Command};

use flate2::read::GzDecoder;

use crate::{
    cli_uploads::{install_fake_ssh, temporary_directory},
    command_support::remove_directory_when_released,
};

/// 使用测试`SSH`替身执行`batch`部署。
fn run_deploy(
    directory: &std::path::Path,
    source: &std::path::Path,
    platform: &str,
    archive: Option<&std::path::Path>,
) -> std::process::Output {
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_procora"));
    command
        .args([
            "deploy",
            source.to_str().unwrap(),
            "--ssh",
            "edge-host",
            "--batch",
        ])
        .env("PATH", path)
        .env("FAKE_SSH_PLATFORM", platform)
        .env("FAKE_SSH_LOG", directory.join("ssh.log"))
        .env("FAKE_SSH_HEADER_LOG", directory.join("header.log"));
    if let Some(archive) = archive {
        command.env("FAKE_SSH_ARCHIVE_LOG", archive);
    }
    command.output().unwrap()
}

/// 返回归档内全部普通文件正文。
fn archive_files(path: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let decoder = GzDecoder::new(fs::File::open(path).unwrap());
    let mut archive = tar::Archive::new(decoder);
    archive
        .entries()
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.header().entry_type().is_file())
        .map(|mut entry| {
            let path = entry.path().unwrap().to_string_lossy().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (path, bytes)
        })
        .collect()
}

#[test]
// Unicode target和Service目录外的预编译产物可以被精确选入且不会泄漏其他变体。
fn deploy_accepts_unicode_target_and_external_selected_artifact() {
    let directory = temporary_directory("deploy-external-unicode");
    install_fake_ssh(&directory);
    let source = directory.join("服务");
    let builds = directory.join("外部构建");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&builds).unwrap();
    fs::write(builds.join("api-linux"), b"external-linux-binary").unwrap();
    fs::write(
        source.join("procora.yaml"),
        r#"version: 1
project: demo
binaries:
  api:
    target: "程序/api"
    variants:
      linux-amd64: "../外部构建/api-linux"
      macos-arm64: "../不存在/api-macos"
tasks: {}
"#,
    )
    .unwrap();
    let archive = directory.join("submitted.tar.gz");
    let output = run_deploy(
        &directory,
        &source,
        r#"{"name":"procora-ssh","platform":{"os":"linux","arch":"x86_64","environment":"gnu"}}"#,
        Some(&archive),
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files = archive_files(&archive);
    assert_eq!(files["程序/api"], b"external-linux-binary");
    assert!(!files.keys().any(|path| path.contains("外部构建")));
    remove_directory_when_released(&directory);
}

#[test]
// 空文件和符号链接产物都会在调用远端接收器前失败。
fn deploy_rejects_empty_and_symlink_selected_artifacts_before_upload() {
    let directory = temporary_directory("deploy-invalid-artifacts");
    install_fake_ssh(&directory);
    let platform =
        r#"{"name":"procora-ssh","platform":{"os":"linux","arch":"x86_64","environment":"gnu"}}"#;
    let cases = [("empty", "不能为空文件"), ("symlink", "必须是普通文件")];
    for (label, expected) in cases {
        let source = directory.join(label);
        fs::create_dir_all(source.join("dist")).unwrap();
        fs::write(
            source.join("procora.yaml"),
            "version: 1\nproject: demo\nbinaries:\n  api:\n    target: bin/api\n    variants:\n      linux-amd64: dist/api\ntasks: {}\n",
        )
        .unwrap();
        let artifact = source.join("dist/api");
        if label == "empty" {
            fs::write(&artifact, []).unwrap();
        } else {
            fs::write(source.join("dist/real"), b"binary").unwrap();
            symlink("real", &artifact).unwrap();
        }
        let output = run_deploy(&directory, &source, platform, None);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
    }
    let log = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert!(!log.contains("__receive-deploy"));
    remove_directory_when_released(&directory);
}

#[test]
// 超大、伪装接收器和未知架构握手都必须有界失败且不进入上传阶段。
fn deploy_rejects_extreme_or_malformed_platform_probes() {
    let directory = temporary_directory("deploy-probe-boundaries");
    install_fake_ssh(&directory);
    let source = directory.join("service");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("procora.yaml"),
        "version: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    let cases = [
        ("x".repeat(70 * 1024), "超过 64 KiB"),
        (
            r#"{"name":"not-procora","platform":{"os":"linux","arch":"x86_64"}}"#.to_owned(),
            "不是兼容的 Procora",
        ),
        (
            r#"{"name":"procora-ssh","platform":{"os":"linux","arch":"quantum"}}"#.to_owned(),
            "不支持的架构",
        ),
    ];
    for (probe, expected) in cases {
        let output = run_deploy(&directory, &source, &probe, None);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
    }
    let log = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert!(!log.contains("__receive-deploy"));
    remove_directory_when_released(&directory);
}
