use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};

use crate::command_support::{remove_directory_when_released, run_background_cli};
pub(super) use crate::deploy_command_support::receive_deploy;

/// 创建当前测试独占的临时目录。
pub(super) fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "procora-deploy-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 创建只包含普通文件的完整 Service 归档。
pub(super) fn service_archive(files: &[(&str, &[u8])]) -> (Vec<u8>, u64) {
    let encoder = GzEncoder::new(Vec::new(), Compression::fast());
    let mut archive = tar::Builder::new(encoder);
    let mut content_bytes = 0_u64;
    for (path, bytes) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, *bytes).unwrap();
        content_bytes += bytes.len() as u64;
    }
    (
        archive.into_inner().unwrap().finish().unwrap(),
        content_bytes,
    )
}

/// 向隐藏全托管部署接收器发送完整协议流。
fn deploy(home: &std::path::Path, archive: &[u8], content_bytes: u64) -> Output {
    deploy_with_keep(home, archive, content_bytes, 3)
}

/// 使用指定release保留数量发送部署。
fn deploy_with_keep(
    home: &std::path::Path,
    archive: &[u8],
    content_bytes: u64,
    keep: u32,
) -> Output {
    let digest = format!("{:x}", Sha256::digest(archive));
    let header = serde_json::json!({
        "protocol": 2,
        "project": "demo",
        "config_path": "procora.yaml",
        "archive_bytes": archive.len(),
        "content_bytes": content_bytes,
        "sha256": digest,
        "timeout_ms": 1000,
        "stable_for_ms": 0,
        "keep": keep,
    });
    receive_deploy(home, archive, &header)
}

#[test]
// 远端无需预声明target即可首次创建并更新全托管Service。
fn managed_deploy_creates_and_updates_service_without_target() {
    let home = temporary_directory("update");
    let first_config = b"version: 1\nproject: demo\ntasks: {}\n";
    let (first_archive, first_bytes) =
        service_archive(&[("procora.yaml", first_config), ("version.txt", b"first")]);
    let first_digest = format!("{:x}", Sha256::digest(&first_archive));
    let first = deploy(&home, &first_archive, first_bytes);
    assert!(
        first.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    let (second_archive, second_bytes) =
        service_archive(&[("procora.yaml", first_config), ("version.txt", b"second")]);
    let second_digest = format!("{:x}", Sha256::digest(&second_archive));
    let second = deploy(&home, &second_archive, second_bytes);
    assert!(
        second.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("services/demo/state.json")).unwrap()).unwrap();
    assert_eq!(state["active_release"], &second_digest[..16]);
    assert_eq!(state["releases"].as_array().unwrap().len(), 2);
    assert!(
        home.join("services/demo/releases")
            .join(&first_digest[..16])
            .is_dir()
    );
    assert!(
        home.join("services/demo/releases")
            .join(&second_digest[..16])
            .is_dir()
    );

    stop_center(&home);
    remove_directory_when_released(&home);
}

#[test]
// 相同归档重复部署时保持当前release，不切换、不重启且不追加成功记录。
fn managed_deploy_is_idempotent_for_active_release() {
    let home = temporary_directory("idempotent");
    let config = b"version: 1\nproject: demo\ntasks: {}\n";
    let (archive, content_bytes) =
        service_archive(&[("procora.yaml", config), ("version.txt", b"same")]);
    let first = deploy(&home, &archive, content_bytes);
    assert!(first.status.success());
    let second = deploy(&home, &archive, content_bytes);

    assert!(
        second.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.contains("跳过切换与重启"), "{stdout}");
    assert!(stdout.contains(r#""changed":false"#), "{stdout}");
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("services/demo/state.json")).unwrap()).unwrap();
    assert_eq!(state["releases"].as_array().unwrap().len(), 1);
    assert_eq!(state["deployments"].as_array().unwrap().len(), 1);

    stop_center(&home);
    remove_directory_when_released(&home);
}

#[test]
// 相同release在Service已停止时会重新启动，而不是误判成无需更新。
fn managed_deploy_restarts_stopped_active_release() {
    let home = temporary_directory("restart-stopped");
    let config = b"version: 1\nproject: demo\ntasks: {}\n";
    let (archive, content_bytes) = service_archive(&[("procora.yaml", config)]);
    assert!(deploy(&home, &archive, content_bytes).status.success());
    let stopped = Command::new(env!("CARGO_BIN_EXE_procora"))
        .args(["stop", "demo"])
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(stopped.status.success());

    let repeated = deploy(&home, &archive, content_bytes);

    assert!(
        repeated.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&repeated.stdout),
        String::from_utf8_lossy(&repeated.stderr)
    );
    let stdout = String::from_utf8_lossy(&repeated.stdout);
    assert!(stdout.contains(r#""changed":true"#), "{stdout}");
    assert!(!stdout.contains("跳过切换与重启"), "{stdout}");
    let listed = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("list")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&listed.stdout).contains("运行中"));
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("services/demo/state.json")).unwrap()).unwrap();
    assert_eq!(state["deployments"].as_array().unwrap().len(), 2);

    stop_center(&home);
    remove_directory_when_released(&home);
}

#[test]
// 相同release的Task已经退出时会重新创建进程，不会只看Service宿主状态。
fn managed_deploy_restarts_active_release_with_unavailable_task() {
    let home = temporary_directory("restart-unavailable-task");
    let executable = std::env::current_exe().unwrap();
    let config = serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "project": "demo",
        "tasks": {
            "worker": {
                "command": executable,
                "args": [
                    "--exact",
                    "deployments::managed_deploy_short_lived_helper",
                    "--nocapture"
                ],
                "env": {"PROCORA_DEPLOY_SHORT_LIVED_TEST": "1"}
            }
        }
    }))
    .unwrap();
    let (archive, content_bytes) = service_archive(&[("procora.json", config.as_slice())]);
    let first = deploy_with_config(&home, &archive, content_bytes, "procora.json");
    assert!(
        first.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    std::thread::sleep(std::time::Duration::from_millis(2500));

    let repeated = deploy_with_config(&home, &archive, content_bytes, "procora.json");

    assert!(
        repeated.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&repeated.stdout),
        String::from_utf8_lossy(&repeated.stderr)
    );
    let stdout = String::from_utf8_lossy(&repeated.stdout);
    assert!(stdout.contains(r#""changed":true"#), "{stdout}");
    assert!(!stdout.contains("跳过切换与重启"), "{stdout}");

    stop_center(&home);
    remove_directory_when_released(&home);
}

#[test]
// 两阶段状态显示上次切换中断时，下一次部署先恢复已确认release。
fn managed_deploy_recovers_interrupted_switch_before_continuing() {
    let home = temporary_directory("interrupted");
    let config = b"version: 1\nproject: demo\ntasks: {}\n";
    let (first_archive, first_bytes) =
        service_archive(&[("procora.yaml", config), ("version.txt", b"first")]);
    let first_digest = format!("{:x}", Sha256::digest(&first_archive));
    assert!(deploy(&home, &first_archive, first_bytes).status.success());
    let (second_archive, second_bytes) =
        service_archive(&[("procora.yaml", config), ("version.txt", b"second")]);
    let second_digest = format!("{:x}", Sha256::digest(&second_archive));
    assert!(
        deploy(&home, &second_archive, second_bytes)
            .status
            .success()
    );

    let state_path = home.join("services/demo/state.json");
    let mut interrupted: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    interrupted["active_release"] = serde_json::Value::String(first_digest[..16].to_owned());
    interrupted["pending_release"] = serde_json::Value::String(second_digest[..16].to_owned());
    fs::write(
        &state_path,
        serde_json::to_vec_pretty(&interrupted).unwrap(),
    )
    .unwrap();

    let resumed = deploy(&home, &first_archive, first_bytes);

    assert!(
        resumed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(
        String::from_utf8_lossy(&resumed.stdout).contains(r#""phase":"restored""#),
        "{}",
        String::from_utf8_lossy(&resumed.stdout)
    );
    let recovered: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(recovered["active_release"], &first_digest[..16]);
    assert!(recovered["pending_release"].is_null());
    assert!(
        recovered["deployments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|deployment| deployment["message"]
                .as_str()
                .is_some_and(|message| message.contains("上次部署中断")))
    );

    stop_center(&home);
    remove_directory_when_released(&home);
}

#[test]
// release清单提交成功后才清理不再引用的旧目录。
fn managed_deploy_prunes_only_committed_inactive_releases() {
    let home = temporary_directory("prune");
    let config = b"version: 1\nproject: demo\ntasks: {}\n";
    for version in [b"first".as_slice(), b"second", b"third"] {
        let (archive, bytes) =
            service_archive(&[("procora.yaml", config), ("version.txt", version)]);
        assert!(deploy_with_keep(&home, &archive, bytes, 1).status.success());
    }

    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("services/demo/state.json")).unwrap()).unwrap();
    let active = state["active_release"].as_str().unwrap();
    assert_eq!(state["releases"].as_array().unwrap().len(), 1);
    let directories = fs::read_dir(home.join("services/demo/releases"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .collect::<Vec<_>>();
    assert_eq!(directories.len(), 1);
    assert_eq!(directories[0].file_name().to_string_lossy(), active);

    stop_center(&home);
    remove_directory_when_released(&home);
}

#[test]
// 新release启动失败时接收器自动重新注册并验收旧release。
fn managed_deploy_rolls_back_failed_release_automatically() {
    let home = temporary_directory("rollback");
    let good_config = b"version: 1\nproject: demo\ntasks: {}\n";
    let (good_archive, good_bytes) = service_archive(&[("procora.yaml", good_config)]);
    let good_digest = format!("{:x}", Sha256::digest(&good_archive));
    let good = deploy(&home, &good_archive, good_bytes);
    assert!(good.status.success());

    let bad_config = b"version: 1\nproject: demo\ntasks:\n  broken:\n    command: procora-definitely-missing-binary\n";
    let (bad_archive, bad_bytes) = service_archive(&[("procora.yaml", bad_config)]);
    let bad = deploy(&home, &bad_archive, bad_bytes);
    assert!(!bad.status.success());
    let stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(stderr.contains("旧版本已恢复"), "{stderr}");

    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("services/demo/state.json")).unwrap()).unwrap();
    assert_eq!(state["active_release"], &good_digest[..16]);
    assert_eq!(
        state["deployments"].as_array().unwrap()[1]["outcome"],
        "failed_rolled_back"
    );
    let listed = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("list")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    assert!(listed.status.success());
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains(&good_digest[..16]),
        "{}",
        String::from_utf8_lossy(&listed.stdout)
    );

    stop_center(&home);
    remove_directory_when_released(&home);
}

#[test]
// 新release进程已运行但健康检查失败时仍自动回滚。
fn managed_deploy_rolls_back_unhealthy_release() {
    let home = temporary_directory("unhealthy");
    let good_config = b"version: 1\nproject: demo\ntasks: {}\n";
    let (good_archive, good_bytes) = service_archive(&[("procora.yaml", good_config)]);
    let good_digest = format!("{:x}", Sha256::digest(&good_archive));
    assert!(deploy(&home, &good_archive, good_bytes).status.success());

    let executable = std::env::current_exe().unwrap();
    let unhealthy_config = serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "project": "demo",
        "tasks": {
            "server": {
                "command": executable,
                "args": [
                    "--exact",
                    "deployments::managed_deploy_long_running_helper",
                    "--nocapture"
                ],
                "env": {"PROCORA_DEPLOY_HEALTH_TEST": "1"},
                "healthcheck": {
                    "command": executable,
                    "args": [
                        "--exact",
                        "deployments::managed_deploy_failing_health_helper",
                        "--nocapture"
                    ],
                    "period": "20ms",
                    "timeout": "500ms",
                    "failure_threshold": 1
                }
            }
        }
    }))
    .unwrap();
    let (bad_archive, bad_bytes) =
        service_archive(&[("procora.json", unhealthy_config.as_slice())]);
    let bad = deploy_with_config(&home, &bad_archive, bad_bytes, "procora.json");
    assert!(!bad.status.success());
    let stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(stderr.contains("健康检查失败"), "{stderr}");
    assert!(stderr.contains("旧版本已恢复"), "{stderr}");
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("services/demo/state.json")).unwrap()).unwrap();
    assert_eq!(state["active_release"], &good_digest[..16]);

    stop_center(&home);
    remove_directory_when_released(&home);
}

#[test]
// deploy拒绝接管用户在托管目录外注册的同名Service。
fn managed_deploy_rejects_unmanaged_name_conflict() {
    let home = temporary_directory("conflict-home");
    let service = temporary_directory("conflict-service");
    fs::write(
        service.join("procora.yaml"),
        "version: 1\nproject: demo\ntasks: {}\n",
    )
    .unwrap();
    let added = run_background_cli(
        Command::new(env!("CARGO_BIN_EXE_procora"))
            .arg("add")
            .arg(&service)
            .env("PROCORA_HOME", &home),
        &home,
        "deploy-conflict-add",
    );
    assert!(added.status.success());

    let (archive, content_bytes) =
        service_archive(&[("procora.yaml", b"version: 1\nproject: demo\ntasks: {}\n")]);
    let output = deploy(&home, &archive, content_bytes);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("拒绝由 deploy 接管"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("list")
        .env("PROCORA_HOME", &home)
        .output()
        .unwrap();
    let listed_text = String::from_utf8_lossy(&listed.stdout);
    let listed_root = listed_text
        .lines()
        .find(|line| line.starts_with("demo\t"))
        .and_then(|line| line.split('\t').nth(3))
        .expect("list 应返回 demo 的服务目录");
    assert_eq!(
        procora::platform::canonicalize(listed_root).unwrap(),
        procora::platform::canonicalize(&service).unwrap(),
        "{listed_text}"
    );

    stop_center(&home);
    remove_directory_when_released(&home);
    fs::remove_dir_all(service).unwrap();
}

#[test]
// 远端在切换前复核平台选择和选中二进制SHA-256。
fn managed_deploy_rejects_binary_digest_mismatch() {
    let home = temporary_directory("binary-digest");
    let platform = procora::config::DeployPlatform::current();
    let platform_key = platform.key();
    let config = format!(
        r#"version: 1
project: demo
binaries:
  api:
    target: bin/api
    variants:
      "{platform_key}": dist/local-build
tasks: {{}}
"#
    );
    let binary = b"selected-binary";
    let (archive, content_bytes) =
        service_archive(&[("procora.yaml", config.as_bytes()), ("bin/api", binary)]);
    let digest = format!("{:x}", Sha256::digest(&archive));
    let header = serde_json::json!({
        "protocol": 2,
        "project": "demo",
        "config_path": "procora.yaml",
        "archive_bytes": archive.len(),
        "content_bytes": content_bytes,
        "sha256": digest,
        "timeout_ms": 1000,
        "stable_for_ms": 0,
        "keep": 3,
        "target_platform": platform,
        "binaries": [{
            "name": "api",
            "selector": platform_key,
            "target": "bin/api",
            "bytes": binary.len(),
            "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        }]
    });

    let output = receive_deploy(&home, &archive, &header);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("二进制 `api` SHA-256 不匹配"), "{stderr}");
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
// 为健康门控提供持续运行的跨平台受管进程。
fn managed_deploy_long_running_helper() {
    if std::env::var_os("PROCORA_DEPLOY_HEALTH_TEST").is_none() {
        return;
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

#[test]
// 为健康门控稳定返回非零检查结果。
fn managed_deploy_failing_health_helper() {
    assert!(
        std::env::var_os("PROCORA_DEPLOY_HEALTH_TEST").is_none(),
        "模拟健康检查失败"
    );
}

#[test]
// 为幂等部署测试提供启动后很快正常退出的跨平台Task。
fn managed_deploy_short_lived_helper() {
    if std::env::var_os("PROCORA_DEPLOY_SHORT_LIVED_TEST").is_some() {
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// 正常停止测试 Center。
pub(super) fn stop_center(home: &std::path::Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_procora"))
        .arg("down")
        .env("PROCORA_HOME", home)
        .output()
        .unwrap();
    assert!(output.status.success());
}

/// 使用指定声明式配置入口发送部署。
fn deploy_with_config(
    home: &std::path::Path,
    archive: &[u8],
    content_bytes: u64,
    config_path: &str,
) -> Output {
    let digest = format!("{:x}", Sha256::digest(archive));
    let header = serde_json::json!({
        "protocol": 2,
        "project": "demo",
        "config_path": config_path,
        "archive_bytes": archive.len(),
        "content_bytes": content_bytes,
        "sha256": digest,
        "timeout_ms": 2000,
        "stable_for_ms": 0,
        "keep": 3,
    });
    receive_deploy(home, archive, &header)
}
