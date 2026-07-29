//! 包文件删除与同名 Service 安全清理回归测试。

use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::command_support::{remove_directory_when_released, run_background_cli};

/// 创建当前测试独占的临时目录。
fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = procora::platform::temp_dir().join(format!(
        "procora-package-management-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
// 包文件删除只接受显式.pcpkg文件，清单损坏也不阻塞删除。
fn package_file_delete_accepts_corrupt_package_and_rejects_other_paths() {
    let directory = temporary_directory("delete-file");
    let package = directory.join("broken.pcpkg");
    let ordinary = directory.join("keep.txt");
    let disguised_directory = directory.join("directory.pcpkg");
    fs::write(&package, b"not a valid package").unwrap();
    fs::write(&ordinary, b"keep").unwrap();
    fs::create_dir(&disguised_directory).unwrap();

    procora::package::delete_file(&package).unwrap();
    assert!(!package.exists());
    assert!(procora::package::delete_file(&ordinary).is_err());
    assert!(procora::package::delete_file(&disguised_directory).is_err());
    assert!(ordinary.is_file());
    assert!(disguised_directory.is_dir());

    remove_directory_when_released(&directory);
}

#[test]
// 状态身份与目录不一致时必须退回目录名，保证TUI清理目标仍然可达。
fn installed_catalog_uses_directory_identity_for_mismatched_state() {
    let directory = temporary_directory("catalog-identity");
    let service_root = directory.join("services/expected");
    fs::create_dir_all(&service_root).unwrap();
    fs::write(
        service_root.join("state.json"),
        br#"{
            "project": "different",
            "active_release": null,
            "pending_release": null,
            "releases": []
        }"#,
    )
    .unwrap();

    let catalog = procora::package::installed_catalog(&directory.join("services")).unwrap();
    assert_eq!(catalog.services.len(), 1);
    assert_eq!(catalog.services[0].project, "expected");
    assert!(
        catalog.services[0]
            .error
            .as_deref()
            .unwrap()
            .contains("托管目录名称是 `expected`")
    );

    remove_directory_when_released(&directory);
}

#[test]
// 同名普通Service必须保留，同时损坏的包安装目录仍可解除并永久删除。
fn package_uninstall_preserves_unrelated_same_name_service() {
    let directory = temporary_directory("same-name");
    let home = directory.join("home");
    let ordinary = directory.join("ordinary-service");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&ordinary).unwrap();
    fs::write(
        ordinary.join("procora.yaml"),
        "version: 1\nproject: collision\ntasks: {}\n",
    )
    .unwrap();
    let binary = env!("CARGO_BIN_EXE_procora");

    let added = run_background_cli(
        Command::new(binary)
            .arg("add")
            .arg(&ordinary)
            .env("PROCORA_HOME", &home),
        &directory,
        "same-name-add",
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );

    let package_root = home.join("services/collision");
    fs::create_dir_all(package_root.join("releases")).unwrap();
    fs::create_dir_all(package_root.join("packages")).unwrap();
    fs::write(package_root.join("state.json"), b"{broken json").unwrap();
    fs::write(package_root.join("packages/original.pcpkg"), b"broken").unwrap();

    let detached = run_background_cli(
        Command::new(binary)
            .args(["package", "uninstall", "collision"])
            .env("PROCORA_HOME", &home),
        &directory,
        "same-name-detach",
    );
    assert!(
        detached.status.success(),
        "{}",
        String::from_utf8_lossy(&detached.stderr)
    );
    assert!(
        String::from_utf8_lossy(&detached.stdout).contains("同名普通 Service `collision` 保持不变")
    );
    assert!(package_root.is_dir());
    fs::remove_dir_all(package_root.join("releases")).unwrap();

    let purged = run_background_cli(
        Command::new(binary)
            .args(["package", "uninstall", "collision", "--purge"])
            .env("PROCORA_HOME", &home),
        &directory,
        "same-name-purge",
    );
    assert!(
        purged.status.success(),
        "{}",
        String::from_utf8_lossy(&purged.stderr)
    );
    assert!(
        String::from_utf8_lossy(&purged.stdout).contains("同名普通 Service `collision` 保持不变")
    );
    assert!(!package_root.exists());

    let listed = run_background_cli(
        Command::new(binary).arg("list").env("PROCORA_HOME", &home),
        &directory,
        "same-name-list",
    );
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("collision"));

    let removed = run_background_cli(
        Command::new(binary)
            .args(["remove", "collision"])
            .env("PROCORA_HOME", &home),
        &directory,
        "same-name-remove",
    );
    assert!(removed.status.success());
    let down = run_background_cli(
        Command::new(binary).arg("down").env("PROCORA_HOME", &home),
        &directory,
        "same-name-down",
    );
    assert!(down.status.success());
    remove_directory_when_released(&directory);
}
