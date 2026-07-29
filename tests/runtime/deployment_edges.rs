//! 全托管接收器在平台漂移与恶意元数据下的边界测试。

use std::{
    io::Write,
    process::{Command, Output, Stdio},
};

use sha2::{Digest, Sha256};

use crate::{
    command_support::remove_directory_when_released,
    deployments::{receive_deploy, service_archive, stop_center, temporary_directory},
};

/// 构造包含归档摘要和有界验收参数的基础协议头。
fn deploy_header(archive: &[u8], content_bytes: u64) -> serde_json::Value {
    serde_json::json!({
        "protocol": 2,
        "project": "demo",
        "config_path": "procora.yaml",
        "archive_bytes": archive.len(),
        "content_bytes": content_bytes,
        "sha256": format!("{:x}", Sha256::digest(archive)),
        "timeout_ms": 1000,
        "stable_for_ms": 0,
        "keep": 3,
    })
}

/// 发送应在`Ready`前被拒绝的协议头。
fn send_rejected_header(
    home: &std::path::Path,
    header: &serde_json::Value,
) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("__receive-deploy")
        .env("PROCORA_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{header}").unwrap();
    child.wait_with_output().unwrap()
}

/// 返回一个与当前宿主确定不一致但仍受支持的平台。
fn foreign_platform() -> serde_json::Value {
    if std::env::consts::OS == "windows" {
        serde_json::json!({"os": "linux", "arch": "x86_64", "environment": "gnu"})
    } else {
        serde_json::json!({"os": "windows", "arch": "x86_64", "environment": "msvc"})
    }
}

#[test]
// 平台在预检与接收间漂移时必须在注册切换前拒绝release。
fn managed_deploy_rejects_platform_drift_before_activation() {
    let home = temporary_directory("platform-drift");
    let (archive, content_bytes) =
        service_archive(&[("procora.yaml", b"version: 1\nproject: demo\ntasks: {}\n")]);
    let mut header = deploy_header(&archive, content_bytes);
    header["target_platform"] = foreign_platform();

    let output = receive_deploy(&home, &archive, &header);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("部署目标平台在探测后发生变化"), "{stderr}");
    let listed = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("list")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&listed.stdout).contains("demo"));
    stop_center(&home);
    remove_directory_when_released(&home);
}

#[test]
// 非规范平台、空二进制和内部目录target都在接收归档前被拒绝。
fn managed_deploy_rejects_malformed_binary_metadata_before_ready() {
    let home = temporary_directory("invalid-binary-metadata");
    let empty_sha = format!("{:x}", Sha256::digest([]));
    let cases = [
        (
            serde_json::json!({
                "target_platform": {"os": "linux", "arch": "amd64"},
                "binaries": []
            }),
            "必须使用规范化名称",
        ),
        (
            serde_json::json!({
                "target_platform": {"os": "linux", "arch": "x86_64", "environment": "gnu"},
                "binaries": [{
                    "name": "api",
                    "selector": "linux-x86_64",
                    "target": "bin/api",
                    "bytes": 0,
                    "sha256": empty_sha
                }]
            }),
            "二进制大小必须",
        ),
        (
            serde_json::json!({
                "target_platform": {"os": "linux", "arch": "x86_64", "environment": "gnu"},
                "binaries": [{
                    "name": "api",
                    "selector": "linux-x86_64",
                    "target": ".procora/escape",
                    "bytes": 1,
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
                }]
            }),
            "名称或 target 重复或无效",
        ),
    ];
    for (metadata, expected) in cases {
        let mut header = serde_json::json!({
            "protocol": 2,
            "project": "demo",
            "config_path": "procora.yaml",
            "archive_bytes": 1,
            "content_bytes": 1,
            "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
            "timeout_ms": 1000,
            "stable_for_ms": 0,
            "keep": 3,
        });
        for (key, value) in metadata.as_object().unwrap() {
            header[key] = value.clone();
        }
        let output: Output = send_rejected_header(&home, &header);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
    }
    remove_directory_when_released(&home);
}
