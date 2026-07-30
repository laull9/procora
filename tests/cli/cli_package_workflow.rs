//! Procora 包准备脚本、幂等构建与一键部署流程测试。

#![cfg(unix)]

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use crate::{
    cli_uploads::{install_fake_ssh, temporary_directory},
    command_support::remove_directory_when_released,
};

/// 执行包命令并返回完整输出。
fn run(arguments: &[&str], current_directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(arguments)
        .current_dir(current_directory)
        .output()
        .unwrap()
}

#[test]
// 构建帮助同时暴露手动参数、准备命令和一键部署入口。
fn package_build_help_exposes_manual_automation_and_deploy_paths() {
    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["package", "build", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for option in [
        "--output",
        "--platform",
        "--prepare",
        "--deploy",
        "--remote-bin",
        "--timeout",
        "--stable-for",
        "--keep",
        "--batch",
        "--force",
    ] {
        assert!(stdout.contains(option), "构建帮助缺少 {option}\n{stdout}");
    }
}

/// 写入由 Python 准备二进制产物的 Service。
fn write_scripted_service(root: &Path) -> String {
    let platform = procora::config::DeployPlatform::current().key();
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::write(
        root.join("procora.yaml"),
        format!(
            r#"version: 1
project: scripted-package
binaries:
  api:
    target: bin/api
    variants:
      "{platform}": dist/api
tasks: {{}}
"#
        ),
    )
    .unwrap();
    fs::write(root.join("README.txt"), "stable\n").unwrap();
    fs::write(
        root.join("scripts/build package.py"),
        r#"import json
import os
from pathlib import Path

Path("dist").mkdir(exist_ok=True)
Path("dist/api").write_bytes(b"prepared-binary\n")
Path("prepare-env.json").write_text(json.dumps({
    "source": os.environ["PROCORA_PACKAGE_SOURCE"],
    "output": os.environ["PROCORA_PACKAGE_OUTPUT"],
    "platform": os.environ["PROCORA_PACKAGE_PLATFORM"],
    "project": os.environ["PROCORA_PACKAGE_PROJECT"],
}, sort_keys=True), encoding="utf-8")
"#,
    )
    .unwrap();
    r#"python3 "scripts/build package.py""#.to_owned()
}

#[test]
// 显式Python准备命令获得稳定上下文，重复构建无需force且不同内容仍拒绝覆盖。
fn package_prepare_command_and_idempotent_output_form_safe_repeated_workflow() {
    let directory = temporary_directory("package-prepare");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    let prepare = write_scripted_service(&service);
    let package = directory.join("scripted-package.pcpkg");
    let arguments = [
        "package",
        "build",
        service.to_str().unwrap(),
        "--output",
        package.to_str().unwrap(),
        "--prepare",
        &prepare,
        "--prepare",
        &prepare,
    ];

    let first = run(&arguments, &directory);
    assert!(
        first.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let stderr = String::from_utf8_lossy(&first.stderr);
    assert!(stderr.contains("[准备 1/2]"), "{stderr}");
    assert!(stderr.contains("[准备 2/2]"), "{stderr}");
    let environment: serde_json::Value =
        serde_json::from_slice(&fs::read(service.join("prepare-env.json")).unwrap()).unwrap();
    assert_eq!(
        environment["source"],
        procora::platform::canonicalize(&service)
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert_eq!(environment["output"], package.to_str().unwrap());
    assert_eq!(environment["platform"], "all");
    assert_eq!(environment["project"], "scripted-package");
    let original = fs::read(&package).unwrap();

    let repeated = run(&arguments, &directory);
    assert!(repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stdout).contains("包未变化"));
    assert_eq!(fs::read(&package).unwrap(), original);

    fs::write(service.join("README.txt"), "changed\n").unwrap();
    let conflicting = run(
        &[
            "package",
            "build",
            service.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ],
        &directory,
    );
    assert!(!conflicting.status.success());
    assert!(String::from_utf8_lossy(&conflicting.stderr).contains("--force"));
    assert_eq!(fs::read(&package).unwrap(), original);
    remove_directory_when_released(&directory);
}

#[test]
// 准备命令失败发生在替换之前，已有包保持逐字节不变。
fn failed_package_prepare_preserves_existing_output() {
    let directory = temporary_directory("package-prepare-failure");
    let service = directory.join("service");
    fs::create_dir(&service).unwrap();
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: prepare-failure\ntasks: {}\n",
    )
    .unwrap();
    let package = directory.join("prepare-failure.pcpkg");
    let first = run(
        &[
            "package",
            "build",
            service.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ],
        &directory,
    );
    assert!(first.status.success());
    let original = fs::read(&package).unwrap();

    let failed = run(
        &[
            "package",
            "build",
            service.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
            "--prepare",
            "python3 -c \"import sys; sys.exit(7)\"",
            "--force",
        ],
        &directory,
    );
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("构建准备命令失败"));
    assert_eq!(fs::read(&package).unwrap(), original);
    remove_directory_when_released(&directory);
}

#[test]
// 单条命令依次准备产物、构建包并进入现有裸机部署协议。
fn package_build_can_prepare_and_deploy_in_one_command() {
    let directory = temporary_directory("package-build-deploy");
    install_fake_ssh(&directory);
    let service = directory.join("service");
    fs::create_dir_all(service.join("scripts")).unwrap();
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    fs::write(
        service.join("scripts/prepare.py"),
        "from pathlib import Path\nPath('prepared.txt').write_text('ready\\n')\n",
    )
    .unwrap();
    let package = directory.join("demo.pcpkg");
    let path = format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let execute = |remote_mode: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_procora"));
        command
            .args([
                "package",
                "build",
                service.to_str().unwrap(),
                "--output",
                package.to_str().unwrap(),
                "--prepare",
                "python3 scripts/prepare.py",
                "--deploy",
                "mock-host",
                "--timeout",
                "1s",
                "--stable-for",
                "0ms",
            ])
            .env("PATH", &path)
            .env("PROCORA_HOME", directory.join("home"))
            .env("FAKE_SSH_LOG", directory.join("ssh.log"))
            .env("FAKE_SSH_HEADER_LOG", directory.join("ssh-header.log"));
        if let Some(mode) = remote_mode {
            command.env("FAKE_SSH_MODE", mode);
        }
        command.output().unwrap()
    };
    let failed = execute(Some("old-deploy"));
    assert!(!failed.status.success());
    assert!(package.is_file());
    let verified = run(
        &["package", "verify", package.to_str().unwrap()],
        &directory,
    );
    assert!(verified.status.success());

    let output = execute(None);
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("包未变化"), "{stdout}");
    assert!(stdout.contains("开始部署"), "{stdout}");
    assert!(stdout.contains("部署完成：demo"), "{stdout}");
    assert!(package.is_file());
    assert!(service.join("prepared.txt").is_file());
    let invocation = fs::read_to_string(directory.join("ssh.log")).unwrap();
    assert!(invocation.contains("__ssh-probe"));
    assert!(invocation.contains("__receive-deploy"));
    let memory: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("home/cli-memory/deploy.json")).unwrap())
            .unwrap();
    assert_eq!(memory["entries"][0]["project"], "demo");
    assert_eq!(memory["entries"][0]["ssh_target"], "mock-host");
    assert_eq!(
        memory["entries"][0]["root"],
        procora::platform::canonicalize(&service)
            .unwrap()
            .to_str()
            .unwrap()
    );
    remove_directory_when_released(&directory);
}
